#!/usr/bin/env python3
"""Solace benchmark harness (PROPOSAL.md §19.1).

  bench.py validate [TASK...]   check each task: hidden tests fail before, pass with the reference solution
  bench.py run [--arm files|solace] [--agent codex|claude] [--model M] [--effort E]
               [--budget USD] [--repeat N] [TASK...]
                                run an agent on each task and grade it
  bench.py report RUN_DIR       per-task pass rate and medians for a run

A task is a directory in bench/tasks/ holding prompt.md, hidden_test.gleam,
solution.patch, and optionally setup.patch, `project` (the project directory
it runs against; default pilot) and work.md (a work record left by an earlier
developer: HANDOFF.md in the files arm, a store work record in the solace arm). Every run works on a fresh copy of that project.
TASK arguments may be shell-style patterns, e.g. 'L*'.
"""

import argparse
import fnmatch
import json
import os
import re
import shutil
import statistics
import subprocess
import sys
import tempfile
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent
TASKS = ROOT / "tasks"
RESULTS = ROOT / "results"
HIDDEN = "test/bench_hidden_test.gleam"
EDIT_TOOLS = {"Edit", "Write", "MultiEdit", "NotebookEdit"}


def is_edit(block):
    """A tool call that changes code: a file edit, or a solace submit/delete."""
    if block.get("name") in EDIT_TOOLS:
        return True
    command = block.get("input", {}).get("command", "").strip()
    command = command.removeprefix("solace").strip()
    return block.get("name", "").endswith("solace") and command.startswith(("submit", "delete"))
TOOLS = "Read,Edit,Write,Glob,Grep,Bash"  # the only tools the agent sees
ALLOWED_TOOLS = [  # and what runs without a permission prompt
    "Read", "Edit", "Write", "Glob", "Grep",
    "Bash(gleam:*)", "Bash(ls:*)", "Bash(cat:*)", "Bash(find:*)", "Bash(grep:*)",
]
SOLACE_BIN = ROOT.parent / "solace-cli" / "target" / "release"
DEFAULT_MODEL = {"codex": "gpt-5.6-terra", "claude": "claude-sonnet-5"}
# Codex features that aren't part of the task environment, turned off in both arms.
CODEX_OFF = ["multi_agent", "apps", "plugins", "goals", "image_generation", "view_image", "browser_use",
             "computer_use", "in_app_browser", "hooks", "memories", "tool_suggest"]
CODEX_SOLACE_HINT = ("The tool is the MCP tool `solace`; from `exec`, call it as "
                     "`await tools.mcp__solace__solace({command: \"session start\"})`, "
                     "with the new code in `text` for submit commands. One `exec` script can make "
                     "several solace calls, so read everything you need in one step.\n\n")
# In Codex code mode every tool call is a JavaScript snippet; these mark the ones that change code.
CODEX_EDIT = re.compile(r"tools\.apply_patch|[\"'`]\s*(?:solace\s+)?(?:submit|delete)\b"
                        r"|sed\s+-i|perl\s+-pi|\btee\s|>\s*[\w./-]+\.gleam")
SOLACE_PREAMBLE = (
    "You are working on a Gleam project: an order-management service. Its code lives in a "
    "Solace semantic store, and you work on it only through the `solace` tool. You cannot "
    "read or edit files or run shell commands. Start with the command `session start`: it "
    "lists every symbol, open work and the commands. Complete the task below. Keep existing behavior and tests working unless the "
    "task says otherwise, and add or update tests as appropriate (tests are symbols in the "
    "test modules).\n\n"
)
PREAMBLE = (
    "You are working in a Gleam project in the current directory: an order-management "
    "service. `gleam test` runs its tests. Complete the task below. Keep existing "
    "behavior and tests working unless the task says otherwise, and add or update "
    "tests as appropriate.\n\n"
)


def sh(cmd, cwd, **kw):
    return subprocess.run(cmd, cwd=cwd, capture_output=True, text=True, **kw)


def prepare(task: Path, arm: str = "files") -> Path:
    """Fresh git copy of the pilot with the task's setup applied and committed."""
    work = Path(tempfile.mkdtemp(prefix=f"solace-{task.name}-"))
    project = (task / "project").read_text().strip() if (task / "project").exists() else "pilot"
    shutil.copytree(ROOT.parent / project, work, dirs_exist_ok=True, ignore=shutil.ignore_patterns("dev", "prod"))
    git = ["git", "-c", "user.email=bench@solace", "-c", "user.name=bench"]
    sh(git + ["init", "-q"], work)
    if (task / "setup.patch").exists():
        r = sh(["git", "apply", str(task / "setup.patch")], work)
        if r.returncode:
            sys.exit(f"{task.name}: setup.patch does not apply\n{r.stderr}")
    if arm == "files" and (task / "work.md").exists():
        shutil.copy(task / "work.md", work / "HANDOFF.md")
    sh(git + ["add", "-A"], work)
    sh(git + ["commit", "-qm", "base"], work)
    return work


def gleam_test(work: Path) -> bool:
    return sh(["gleam", "test"], work, timeout=600).returncode == 0


def grade(task: Path, work: Path) -> dict:
    visible = gleam_test(work)
    shutil.copy(task / "hidden_test.gleam", work / HIDDEN)
    hidden = gleam_test(work)
    (work / HIDDEN).unlink()
    return {"visible_pass": visible, "hidden_pass": hidden}


def validate(tasks):
    ok = True
    for task in tasks:
        work = prepare(task)
        before = grade(task, work)
        r = sh(["git", "apply", str(task / "solution.patch")], work)
        after = grade(task, work) if r.returncode == 0 else {"visible_pass": False, "hidden_pass": False}
        good = before["visible_pass"] and not before["hidden_pass"] and after["visible_pass"] and after["hidden_pass"]
        ok &= good
        print(f"{'ok  ' if good else 'FAIL'} {task.name}  before={before}  after={after}")
        shutil.rmtree(work)
    return ok


def input_side(u):
    return sum(u.get(k, 0) for k in ("input_tokens", "cache_creation_input_tokens", "cache_read_input_tokens"))


def parse_stream(lines):
    """Metrics from `claude -p --output-format stream-json` output.

    Totals come from the final result event: per-message output counts in the
    stream are partial. Tokens before the first edit count each earlier
    message's input side (fresh + cached context), which the stream reports exactly.
    """
    seen, blocks = set(), set()
    context = calls = 0
    first_context = None
    first_edit = None  # (tokens, tool calls, turns) spent before the first edit
    result, init = {}, {}
    for line in lines:
        try:
            ev = json.loads(line)
        except json.JSONDecodeError:
            continue
        if ev.get("type") == "system" and ev.get("subtype") == "init":
            init = {k: ev.get(k) for k in ("model", "tools", "mcp_servers", "plugins", "slash_commands")}
        elif ev.get("type") == "assistant":
            msg = ev["message"]
            if msg.get("id") not in seen:  # one message can arrive as several events
                seen.add(msg.get("id"))
                spent_before = context
                size = input_side(msg.get("usage", {}))
                context += size
                if first_context is None:
                    first_context = size
            for b in msg.get("content", []):
                if b.get("type") == "tool_use" and b.get("id") not in blocks:
                    blocks.add(b.get("id"))
                    if is_edit(b) and first_edit is None:
                        first_edit = (spent_before, calls, len(seen))
                    calls += 1
        elif ev.get("type") == "result":
            result = ev
    usage = result.get("usage", {})
    tokens = input_side(usage) + usage.get("output_tokens", 0)
    # The first call's context is Claude Code's fixed prompt and tools, which
    # every later call re-sends too. Everything above that is the task's work.
    overhead = (first_context or 0) * len(seen)
    return {
        "tokens": tokens,
        "work_tokens": tokens - overhead,
        "uncached_tokens": usage.get("input_tokens", 0) + usage.get("cache_creation_input_tokens", 0)
        + usage.get("output_tokens", 0),
        "output_tokens": usage.get("output_tokens"),
        "tool_calls": calls,
        "tokens_before_first_edit": first_edit[0] if first_edit else None,
        "tool_calls_before_first_edit": first_edit[1] if first_edit else None,
        # model turns up to and including the one that makes the first edit
        "turns_to_first_edit": first_edit[2] if first_edit else None,
        "turns": result.get("num_turns"),
        "cost_usd": result.get("total_cost_usd"),
        "duration_ms": result.get("duration_ms"),
        "agent_error": result.get("is_error", True),
        "init": init,
    }


def parse_codex(lines):
    """Metrics from Codex session logs (`$CODEX_HOME/sessions/**.jsonl`).

    Each model call writes one token_usage_record, after the tool calls it made.
    OpenAI usage counts cached input inside input_tokens.
    """
    calls = turns = context = 0
    first_context = None
    first_edit = None  # (tokens, tool calls, turns) spent before the first edit
    edit_pending = None  # tool calls before the first edit, until its turn's usage arrives
    tokens = uncached = output = 0
    for line in lines:
        try:
            ev = json.loads(line)
        except json.JSONDecodeError:
            continue
        p = ev.get("payload") or {}
        if ev.get("type") == "response_item" and p.get("type") in ("custom_tool_call", "function_call"):
            calls += 1
            if CODEX_EDIT.search(str(p.get("input") or p.get("arguments") or "")) and edit_pending is None:
                edit_pending = calls - 1
        elif ev.get("type") == "token_usage_record":
            u = p.get("usage", {})
            size = u.get("input_tokens", 0)
            turns += 1
            if edit_pending is not None and first_edit is None:
                first_edit = (context, edit_pending, turns)
            context += size
            first_context = size if first_context is None else first_context
            tokens += size + u.get("output_tokens", 0)
            uncached += size - u.get("cached_input_tokens", 0) + u.get("output_tokens", 0)
            output += u.get("output_tokens", 0)
    return {
        "tokens": tokens,
        "work_tokens": tokens - (first_context or 0) * turns,
        "uncached_tokens": uncached,
        "output_tokens": output,
        "tool_calls": calls,
        "tokens_before_first_edit": first_edit[0] if first_edit else None,
        "tool_calls_before_first_edit": first_edit[1] if first_edit else None,
        "turns_to_first_edit": first_edit[2] if first_edit else None,
        "turns": turns,
        "cost_usd": None,  # ChatGPT sign-in: no per-token price
        "duration_ms": None,
        "agent_error": first_edit is None and turns == 0,
        "init": {},
    }


def codex_cmd(arm, model, effort):
    cmd = ["codex", "exec", "--json", "--skip-git-repo-check", "-m", model,
           "-c", f'model_reasoning_effort="{effort}"', "-c", 'web_search="disabled"',
           # Codex cuts tool results over 10k tokens by default; batched reads overshoot it.
           "-c", "tool_output_token_limit=30000"]
    for f in CODEX_OFF:
        cmd += ["--disable", f]
    if arm == "files":
        return cmd + ["-s", "workspace-write", "-"]
    # D-8: no shell, and a read-only sandbox so apply_patch can't write; the
    # solace MCP server runs outside the sandbox.
    return cmd + ["-s", "read-only", "--disable", "shell_tool", "--disable", "unified_exec",
                  "-c", f'mcp_servers.solace.command="{SOLACE_BIN / "solace"}"',
                  "-c", 'mcp_servers.solace.args=["mcp"]',
                  "-c", 'mcp_servers.solace.default_tools_approval_mode="approve"', "-"]


def run(tasks, model, budget, repeat, arm, agent="codex", effort="medium"):
    label = model if agent == "claude" else f"{model}-{effort}"
    run_dir = RESULTS / f"{time.strftime('%Y%m%d-%H%M%S')}-{label}" if arm == "files" \
        else RESULTS / f"{time.strftime('%Y%m%d-%H%M%S')}-{label}-{arm}"
    run_dir.mkdir(parents=True)
    env = {**os.environ, "PATH": f"{SOLACE_BIN}:{os.environ['PATH']}"}
    cmd = ["claude", "-p", "--output-format", "stream-json", "--verbose", "--model", model,
           "--no-session-persistence", "--strict-mcp-config", "--disable-slash-commands",
           "--max-budget-usd", str(budget)]
    if arm == "files":
        preamble = PREAMBLE
        cmd += ["--tools", TOOLS, "--allowedTools", *ALLOWED_TOOLS]
    else:  # D-8: the protocol only, as one MCP tool; no built-in tools at all
        preamble = SOLACE_PREAMBLE
        server = {"mcpServers": {"solace": {"command": str(SOLACE_BIN / "solace"), "args": ["mcp"]}}}
        cmd += ["--tools", "", "--mcp-config", json.dumps(server), "--allowedTools", "mcp__solace__solace"]
    # Keep the user's own plugins, hooks and settings out of the measurement.
    cmd += ["--bare"] if os.environ.get("ANTHROPIC_API_KEY") else ["--setting-sources", "project"]
    for task, rep in [(t, r) for r in range(1, repeat + 1) for t in tasks]:
        name = f"{task.name}.{rep}"
        work = prepare(task, arm)
        if arm == "solace":
            r = sh(["solace", "import"], work, env=env)
            if r.returncode:
                sys.exit(f"{task.name}: solace import failed\n{r.stdout}")
            (work / ".git" / "info" / "exclude").write_text(".solace/\n")
            if (task / "work.md").exists():
                sh(["solace", "work", "add"], work, env=env, input=(task / "work.md").read_text())
        if agent == "codex" and arm == "solace":
            preamble = SOLACE_PREAMBLE + CODEX_SOLACE_HINT
        prompt = preamble + (task / "prompt.md").read_text()
        if agent == "codex":
            # A fresh CODEX_HOME keeps the user's config, plugins, hooks and memories out.
            home = Path(tempfile.mkdtemp(prefix="codex-home-"))
            (home / "auth.json").symlink_to(Path.home() / ".codex" / "auth.json")
            try:
                sh(codex_cmd(arm, model, effort), work, input=prompt, timeout=1800, env={**env, "CODEX_HOME": str(home)})
            except subprocess.TimeoutExpired:
                pass
            logs = sorted(home.glob("sessions/**/*.jsonl"), key=lambda f: f.stat().st_mtime)
            if len(logs) > 1:
                print(f"  note: {len(logs)} session logs (sub-agents?); metrics use all of them")
            stream = "".join(f.read_text() for f in logs)
            shutil.rmtree(home)
            metrics = parse_codex(stream.splitlines())
        else:
            try:
                proc = sh(cmd, work, input=prompt, timeout=1800, env=env)
                stream = proc.stdout
            except subprocess.TimeoutExpired as e:
                stream = e.stdout.decode() if isinstance(e.stdout, bytes) else (e.stdout or "")
            metrics = parse_stream(stream.splitlines())
        (run_dir / f"{name}.stream.jsonl").write_text(stream)
        row = {"task": task.name, "rep": rep, "agent": agent, "model": model, "effort": effort if agent == "codex" else None,
               "arm": arm, **metrics, **grade(task, work)}
        sh(["git", "add", "-A"], work)
        (run_dir / f"{name}.diff").write_text(sh(["git", "diff", "--cached", "HEAD"], work).stdout)
        init = row.pop("init")
        (run_dir / "init.json").write_text(json.dumps(init, indent=2))
        with open(run_dir / "summary.jsonl", "a") as f:
            f.write(json.dumps(row) + "\n")
        print(f"{'PASS' if row['hidden_pass'] else 'fail'} {name}  tokens={row['tokens']} "
              f"turns_to_edit={row['turns_to_first_edit']} before_edit={row['tokens_before_first_edit']}")
        shutil.rmtree(work)
    print(f"results: {run_dir}")


def report(run_dir: Path):
    rows = [json.loads(l) for l in (run_dir / "summary.jsonl").read_text().splitlines()]
    for r in rows:  # older runs: derive newer metrics from the saved stream
        stream = run_dir / f"{r['task']}.{r.get('rep', 1)}.stream.jsonl"
        if (r.get("agent") == "codex" or "turns_to_first_edit" not in r) and stream.exists():
            parse = parse_codex if r.get("agent") == "codex" else parse_stream
            r.update({k: v for k, v in parse(stream.read_text().splitlines()).items() if k != "init"})
    cols = ["tokens", "work_tokens", "turns_to_first_edit", "tokens_before_first_edit", "uncached_tokens", "cost_usd"]

    def med(rs, c):
        vals = [r[c] for r in rs if r[c] is not None]
        return statistics.median(vals) if vals else None

    print(f"{'task':<28} {'pass':>5} " + " ".join(f"{c:>12}" for c in ["tokens", "work", "turns→edit", "tok<edit", "uncached", "cost"]))
    for name in sorted({r["task"] for r in rows}) + ["ALL"]:
        rs = rows if name == "ALL" else [r for r in rows if r["task"] == name]
        passed = sum(r["hidden_pass"] for r in rs)
        cells = [med(rs, c) for c in cols]
        print(f"{name:<28} {passed:>2}/{len(rs):<2} " + " ".join(
            f"{'-' if v is None else round(v, 3) if c == 'cost_usd' else int(v):>12}" for c, v in zip(cols, cells)))
    cost = sum(r['cost_usd'] or 0 for r in rows)
    print((f"total cost ${cost:.2f}  " if cost else "") + "(medians per run; ALL = median over every run)")
    print("tokens: all input+output; work: tokens minus the fixed prompt re-sent each call; "
          "turns→edit: model turns up to the first edit; tok<edit: tokens before it; uncached: fresh input+output")


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("command", choices=["validate", "run", "report"])
    ap.add_argument("tasks", nargs="*", help="task directory names (default: all); for report, the run directory")
    ap.add_argument("--agent", choices=["codex", "claude"], default="codex")
    ap.add_argument("--model", help="default: gpt-5.6-terra (codex), claude-sonnet-5 (claude)")
    ap.add_argument("--effort", default="medium", help="codex reasoning effort")
    ap.add_argument("--budget", type=float, default=2.0, help="max USD per task (claude only)")
    ap.add_argument("--repeat", type=int, default=1, help="runs per task")
    ap.add_argument("--arm", choices=["files", "solace"], default="files",
                    help="files: normal file tools; solace: the solace protocol only (D-8)")
    a = ap.parse_args()
    if a.command == "report":
        return report(Path(a.tasks[0]))
    all_names = sorted(p.name for p in TASKS.iterdir() if p.is_dir())
    names = [n for n in all_names if not a.tasks or any(fnmatch.fnmatch(n, pat) for pat in a.tasks)]
    tasks = [TASKS / n for n in names]
    if a.command == "validate":
        sys.exit(0 if validate(tasks) else 1)
    if a.arm == "solace":
        r = subprocess.run(["cargo", "build", "--release", "--quiet"], cwd=ROOT.parent / "solace-cli")
        if r.returncode:
            sys.exit("could not build solace-cli")
    run(tasks, a.model or DEFAULT_MODEL[a.agent], a.budget, a.repeat, a.arm, a.agent, a.effort)


if __name__ == "__main__":
    main()
