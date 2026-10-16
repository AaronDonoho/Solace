#!/usr/bin/env python3
"""Checks the column layout of the reference's "Commands in full" section (D-143).

Usage: reference_layout.py <file>. Exit 0 when the layout is clean; otherwise
exit 1 with one line per fault. Columns count from 1, as editors show them.

- A command row starts with two spaces. Its description, when on the same
  line, follows a gap of two or more spaces and starts at the description
  column: the one most rows use. Rows next to each other may share another
  column of their own (a table, like the file tiers); a row whose term fills
  that column with one space (`  protected solace.toml`) takes its neighbors'.
- A continuation line starts where its row's description starts (at the
  description column when the row has none on its line). Lines two or more
  columns deeper than that are an example block only right after a line ending
  in `:`, and the whole block shares one column; one column deeper is a slip.
- Headings, prose and blank lines (no leading space) are left alone, and so is
  everything before the section. There is no width limit (the human's call).
- The summary table (the indented rows after the line starting `Commands (`)
  names the commands `help` answers: each row's first word must have its own
  two-space entry in the section, so `help <command>` answers one paragraph
  (D-144).
"""
import re
import sys
from collections import Counter

SECTION = "Commands in full"
HOST = "Host commands"


def indent(line):
    return len(line) - len(line.lstrip(" "))


def gap_column(line):
    """Where a row's same-line description starts, after a gap of two or more
    spaces; None when it has none."""
    m = re.search(r"(?<=\S) {2,}(?=\S)", line)
    return m.end() if m else None


def at_word(line, column):
    """Whether a word of `line` starts at `column`, after one space."""
    return 0 < column < len(line) and line[column - 1] == " " and line[column] != " "


def row_names(line):
    """The command names a row's term lists: the first word of each part
    split on `|` (`view | inspect | refs | find`)."""
    term = line.strip().split("  ")[0]
    return [p.split()[0] for p in term.split("|") if p.split()]


def faults(text):
    """One message per fault, in line order."""
    lines = text.split("\n")
    if SECTION not in lines:
        return [f'no "{SECTION}" line']
    section = lines.index(SECTION)
    # Runs of indented lines, between blank lines, headings and prose.
    runs, run = [], []
    for i in range(section + 1, len(lines)):
        if indent(lines[i]) == 0 or not lines[i].strip():
            runs.append(run)
            run = []
        else:
            run.append(i)
    runs.append(run)
    rows = [[i for i in run if indent(lines[i]) == 2] for run in runs]
    column = {i: gap_column(lines[i]) for run_rows in rows for i in run_rows}
    found = [c for c in column.values() if c is not None]
    if not found:
        return ["no command row has its description on the same line"]
    d = Counter(found).most_common(1)[0][0]
    out = []
    # Every command in the summary table needs its own entry in the section,
    # so `help <command>` answers one paragraph (D-144). `help` never shows the
    # host commands, so entries there don't count.
    end = next((k for k in range(section + 1, len(lines)) if lines[k].startswith(HOST)), len(lines))
    entries = [lines[k] for k in range(section + 1, end)
               if lines[k].startswith("  ") and not lines[k][2:3].isspace()]
    for i, line in enumerate(lines[:section]):
        if not line.startswith("Commands ("):
            continue
        j = i + 1
        while j < len(lines) and lines[j].startswith("  "):
            name = lines[j].split()[0]
            if not any(name in row_names(e) for e in entries):
                out.append((j, f'line {j + 1}: no entry in "{SECTION}" for `{name}`'))
            j += 1
        break
    for run, run_rows in zip(runs, rows):
        for n, i in enumerate(run_rows):
            if column[i] is None:
                before = [column[j] for j in reversed(run_rows[:n]) if column[j] is not None][:1]
                after = [column[j] for j in run_rows[n + 1:] if column[j] is not None][:1]
                column[i] = next((c for c in before + after if c != d and at_word(lines[i], c)), None)
        described = [i for i in run_rows if column[i] is not None]
        misplaced = set()
        for n, i in enumerate(described):
            near = described[max(n - 1, 0):n] + described[n + 1:n + 2]
            if column[i] != d and all(column[j] != column[i] for j in near):
                misplaced.add(i)
                out.append((i, f"line {i + 1}: its description starts at column {column[i] + 1}; "
                               f"the descriptions start at column {d + 1}"))
        expected, block = None, None
        for i in run:
            at = indent(lines[i])
            if at == 2:
                own = column[i]
                expected = d if own is None or i in misplaced else own
                block = None
            elif expected is None:
                after = "a blank line" if not lines[run[0] - 1].strip() else "a heading or prose"
                out.append((i, f"line {i + 1}: starts at column {at + 1}; "
                               f"after {after}, a line starts at column 1 or 3"))
            elif at == expected:
                block = None
            elif at > expected and (block is not None or at >= expected + 2 and lines[i - 1].rstrip().endswith(":")):
                if block is None:
                    block = at
                elif at != block:
                    out.append((i, f"line {i + 1}: starts at column {at + 1}; "
                                   f"the example starts at column {block + 1}"))
            else:
                out.append((i, f"line {i + 1}: starts at column {at + 1}; "
                               f"the descriptions start at column {expected + 1}"))
    return [m for _, m in sorted(out)]


def main():
    if len(sys.argv) != 2:
        sys.exit("usage: reference_layout.py <file>")
    with open(sys.argv[1], encoding="utf-8") as f:
        found = faults(f.read())
    for fault in found:
        print(fault)
    sys.exit(1 if found else 0)


if __name__ == "__main__":
    main()
