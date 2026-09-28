#!/usr/bin/env python3
"""Solace benchmark harness (PROPOSAL.md §19.1).

  bench.py validate [TASK...]   check each task: hidden tests fail before, pass with the reference solution
  bench.py run [--arm files|solace] [--model M] [--budget USD] [--repeat N] [TASK...]
                                run an agent on each task and grade it
  bench.py report RUN_DIR       per-task pass rate and medians for a run

A task is a directory in bench/tasks/ holding prompt.md, hidden_test.gleam,
solution.patch, and optionally setup.patch and `project` (the project directory
it runs against; default pilot). Every run works on a fresh copy of that project.
TASK arguments may be shell-style patterns, e.g. 'L*'.
"""

import argparse
import fnmatch
import json
import os
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
SOLACE_PREAMBLE = (
    "You are working on a Gleam project: an order-management service. Its code lives in a "
    "Solace semantic store, and you work on it only through the `solace` tool. You cannot "
    "read or edit files or run shell commands. Start with the command `session start`: it "
    "lists the modules and the commands. Complete the task below. Keep existing behavior and tests working unless the "
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


def prepare(task: Path) -> Path:
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
    first_edit = None  # (tokens, tool calls) spent before the first edit
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
                        first_edit = (spent_before, calls)
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
        "turns": result.get("num_turns"),
        "cost_usd": result.get("total_cost_usd"),
        "duration_ms": result.get("duration_ms"),
        "agent_error": result.get("is_error", True),
        "init": init,
    }


def run(tasks, model, budget, repeat, arm):
    run_dir = RESULTS / f"{time.strftime('%Y%m%d-%H%M%S')}-{model}" if arm == "files" \
        else RESULTS / f"{time.strftime('%Y%m%d-%H%M%S')}-{model}-{arm}"
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
        work = prepare(task)
        if arm == "solace":
            r = sh(["solace", "import"], work, env=env)
            if r.returncode:
                sys.exit(f"{task.name}: solace import failed\n{r.stdout}")
            (work / ".git" / "info" / "exclude").write_text(".solace/\n")
        prompt = preamble + (task / "prompt.md").read_text()
        try:
            proc = sh(cmd, work, input=prompt, timeout=1800, env=env)
            stream = proc.stdout
        except subprocess.TimeoutExpired as e:
            stream = e.stdout.decode() if isinstance(e.stdout, bytes) else (e.stdout or "")
        (run_dir / f"{name}.stream.jsonl").write_text(stream)
        row = {"task": task.name, "rep": rep, "model": model, "arm": arm, **parse_stream(stream.splitlines()), **grade(task, work)}
        sh(["git", "add", "-A"], work)
        (run_dir / f"{name}.diff").write_text(sh(["git", "diff", "--cached", "HEAD"], work).stdout)
        init = row.pop("init")
        (run_dir / "init.json").write_text(json.dumps(init, indent=2))
        with open(run_dir / "summary.jsonl", "a") as f:
            f.write(json.dumps(row) + "\n")
        print(f"{'PASS' if row['hidden_pass'] else 'fail'} {name}  tokens={row['tokens']} "
              f"calls={row['tool_calls']} before_edit={row['tokens_before_first_edit']} cost=${row['cost_usd']}")
        shutil.rmtree(work)
    print(f"results: {run_dir}")


def report(run_dir: Path):
    rows = [json.loads(l) for l in (run_dir / "summary.jsonl").read_text().splitlines()]
    for r in rows:  # older runs: derive newer metrics from the saved stream
        stream = run_dir / f"{r['task']}.{r.get('rep', 1)}.stream.jsonl"
        if "work_tokens" not in r and stream.exists():
            r.update({k: v for k, v in parse_stream(stream.read_text().splitlines()).items() if k != "init"})
    cols = ["tokens", "work_tokens", "tokens_before_first_edit", "uncached_tokens", "cost_usd", "tool_calls"]

    def med(rs, c):
        vals = [r[c] for r in rs if r[c] is not None]
        return statistics.median(vals) if vals else None

    print(f"{'task':<28} {'pass':>5} " + " ".join(f"{c:>12}" for c in ["tokens", "work", "tok<edit", "uncached", "cost", "calls"]))
    for name in sorted({r["task"] for r in rows}) + ["ALL"]:
        rs = rows if name == "ALL" else [r for r in rows if r["task"] == name]
        passed = sum(r["hidden_pass"] for r in rs)
        cells = [med(rs, c) for c in cols]
        print(f"{name:<28} {passed:>2}/{len(rs):<2} " + " ".join(
            f"{'-' if v is None else round(v, 3) if c == 'cost_usd' else int(v):>12}" for c, v in zip(cols, cells)))
    print(f"total cost ${sum(r['cost_usd'] or 0 for r in rows):.2f}  (medians per run; ALL = median over every run)")
    print("tokens: all input+output; work: tokens minus the fixed prompt re-sent each call; "
          "tok<edit: tokens before the first edit; uncached: fresh input+output")


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("command", choices=["validate", "run", "report"])
    ap.add_argument("tasks", nargs="*", help="task directory names (default: all); for report, the run directory")
    ap.add_argument("--model", default="claude-sonnet-5")
    ap.add_argument("--budget", type=float, default=2.0, help="max USD per task")
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
    run(tasks, a.model, a.budget, a.repeat, a.arm)


if __name__ == "__main__":
    main()
