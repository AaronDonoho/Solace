#!/usr/bin/env python3
"""Turn-level analysis of Codex bench runs: where tokens, output and time go.

  analyze.py RUN[/ARM]... [--task PAT]          per-run table, medians per group
  analyze.py RUN[/ARM]... [--task PAT] --turns  also one line per model turn
  analyze.py RUN/ARM --show TASK.REP:TURN       a turn's calls and replies in full

RUN is a number under bench/results (15) or a path; without /ARM every arm is
included. Turns: each model call (one token_usage_record). A writing turn sends
at least WRITE characters of tool-call input (code being written); the rest are
reading turns. Output tokens include reasoning; time is wall time from the
model's input being ready until its usage record.
"""

import argparse
import fnmatch
import json
import re
import statistics
from datetime import datetime
from pathlib import Path

from bench import RESULTS, output_text, solace_commands

WRITE = 1200


def ts(s):
    return datetime.fromisoformat(s.replace("Z", "+00:00")).timestamp()


def summary_of(src):
    """One line naming what a tool call did: solace commands, shell commands, patch targets."""
    cmds = solace_commands(src)
    if cmds:
        return " ; ".join(c[:60] for c in cmds)
    parts = [f"patch {', '.join(t.rsplit('/', 1)[-1] for t in re.findall(r'\*\*\* (?:Update|Add) File: ([^\\\n]+)', src))}"
             for _ in re.findall(r"apply_patch", src)][:1]
    parts += [c[:60] for c in re.findall(r'cmd:\s*"((?:[^"\\]|\\.)*)"', src)]
    return " ; ".join(parts) or src[:60].replace("\n", " ")


def turns(path):
    """Per model turn: output, reasoning, input, seconds, calls (source), replies (text)."""
    out, cur, owner, ready = [], {"calls": [], "msg": ""}, {}, None
    for line in path.read_text().splitlines():
        try:
            ev = json.loads(line)
        except json.JSONDecodeError:
            continue
        p, t = ev.get("payload") or {}, ev.get("type")
        kind = p.get("type") if t == "response_item" else None
        if kind == "message" and p.get("role") == "user" and ready is None:
            ready = ts(ev["timestamp"])
        elif kind == "message" and p.get("role") == "assistant":
            cur["msg"] += "".join(c.get("text", "") for c in p.get("content", []))
        elif kind in ("custom_tool_call", "function_call"):
            cur["calls"].append(str(p.get("input") or p.get("arguments") or ""))
            owner[p.get("call_id")] = cur
            cur.setdefault("replies", [])
        elif kind in ("custom_tool_call_output", "function_call_output"):
            r = output_text(p)
            if p.get("call_id") in owner:
                owner[p["call_id"]].setdefault("replies", []).append(r)
            ready = ts(ev["timestamp"])
        elif t == "token_usage_record":
            u = p.get("usage", {})
            now = ts(ev["timestamp"])
            cur.update(out=u.get("output_tokens", 0), reason=u.get("reasoning_output_tokens", 0),
                       input=u.get("input_tokens", 0), cached=u.get("cached_input_tokens", 0),
                       secs=now - (ready or now))
            cur.setdefault("replies", [])
            out.append(cur)
            cur = {"calls": [], "msg": ""}
    return out


def runs(specs, task_pat):
    """(label, summary row, stream path) for each run matching."""
    found = []
    for spec in specs:
        d = Path(spec) if Path(spec).exists() else RESULTS / spec
        for summary in [d / "summary.jsonl"] if (d / "summary.jsonl").exists() else sorted(d.glob("*/summary.jsonl")):
            for line in summary.read_text().splitlines():
                r = json.loads(line)
                if task_pat and not fnmatch.fnmatch(r["task"], task_pat if "*" in task_pat else f"{task_pat}*"):
                    continue
                stream = summary.parent / f"{r['task']}.{r.get('rep', 1)}.stream.jsonl"
                if stream.exists():
                    label = f"{summary.parent.parent.name}/{r['arm']}"
                    found.append((label, r, stream))
    return found


COLS = [("tokens", 8), ("uncach", 7), ("out", 6), ("reason", 6), ("secs", 5), ("turns", 5), ("→edit", 5),
        ("w.n", 3), ("w.out", 6), ("w.secs", 6), ("w.chars", 7), ("r.out", 6), ("r.secs", 6), ("reply", 7), ("f.edit", 6)]


def row(r, T):
    W = [t for t in T if sum(map(len, t["calls"])) >= WRITE]
    R = [t for t in T if t not in W]
    s = lambda L, k: sum(t[k] for t in L)
    return {
        "tokens": r["tokens"], "uncach": r["uncached_tokens"], "out": s(T, "out"), "reason": s(T, "reason"),
        "secs": round(r["duration_ms"] / 1000), "turns": len(T), "→edit": r.get("turns_to_first_edit"),
        "w.n": len(W), "w.out": s(W, "out"), "w.secs": round(s(W, "secs")), "w.chars": sum(len(c) for t in W for c in t["calls"]),
        "r.out": s(R, "out"), "r.secs": round(s(R, "secs")), "reply": sum(len(x) for t in T for x in t["replies"]),
        "f.edit": r.get("failed_edits"),
    }


def fmt(cells):
    return " ".join(f"{'-' if cells[c] is None else cells[c]:>{w}}" for c, w in COLS)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("runs", nargs="+", help="run numbers or directories, optionally RUN/ARM")
    ap.add_argument("--task", help="task name or prefix, e.g. L08")
    ap.add_argument("--turns", action="store_true", help="one line per model turn")
    ap.add_argument("--show", help="TASK.REP:TURN, e.g. L08.2:5: that turn's calls and replies in full")
    a = ap.parse_args()
    found = runs(a.runs, a.task)
    if a.show:
        name, turn = a.show.rsplit(":", 1)
        task, rep = name.rsplit(".", 1)
        for label, r, stream in found:
            if r["task"].startswith(task) and str(r.get("rep", 1)) == rep:
                t = turns(stream)[int(turn) - 1]
                print(f"## {label} {r['task']}.{rep} turn {turn}: out {t['out']} reasoning {t['reason']} {t['secs']:.1f}s")
                if t["msg"]:
                    print(f"-- message:\n{t['msg']}")
                for c in t["calls"]:
                    print(f"-- call ({len(c)} chars):\n{c}")
                for x in t["replies"]:
                    print(f"-- reply ({len(x)} chars):\n{x}")
        return
    print(f"{'run':<22} " + " ".join(f"{c:>{w}}" for c, w in COLS))
    groups = {}
    for label, r, stream in found:
        T = turns(stream)
        cells = row(r, T)
        groups.setdefault((label, r["task"]), []).append(cells)
        print(f"{label + ' ' + r['task'][:3] + '.' + str(r.get('rep', 1)) + ('' if r['hidden_pass'] else ' FAIL'):<22} {fmt(cells)}")
        if a.turns:
            for k, t in enumerate(T, 1):
                calls = sum(map(len, t["calls"]))
                what = " | ".join(summary_of(c) for c in t["calls"]) or ("(final message)" if t["msg"] else "")
                print(f"    {k:>2} out {t['out']:>5} reason {t['reason']:>5} {t['secs']:>6.1f}s in {t['input']:>6} "
                      f"call {calls:>6} reply {sum(map(len, t['replies'])):>6}{' W' if calls >= WRITE else '  '} {what[:110]}")
    if len(found) > 1:
        print("\nmedians")
        for (label, task), rows in groups.items():
            med = {c: (None if any(x[c] is None for x in rows) else int(statistics.median(x[c] for x in rows))) for c, _ in COLS}
            print(f"{label + ' ' + task[:3] + ' ×' + str(len(rows)):<22} {fmt(med)}")
    print(f"\nuncach: uncached input + output; out includes reason; secs: wall time; →edit: turns to first edit; "
          f"w.*: writing turns (≥{WRITE} chars of tool-call input): count, output, seconds, characters sent; "
          f"r.*: other turns; reply: characters of tool replies; f.edit: failed edits")


if __name__ == "__main__":
    main()
