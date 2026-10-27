#!/usr/bin/env python3
"""Checks the column layout of the reference's command sections (D-143).

Usage: reference_layout.py <file>. Exit 0 when the layout is clean; otherwise
exit 1 with one line per fault. Columns count from 1, as editors show them.
The file may be the whole reference or one section of it: without a
"Commands in full" line, the whole file is checked.

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
- The workflow file (the one with the `Commands (` summary but no section) is
  not a command section: its numbered steps are left alone. A write to it, or
  to any section file, checks the summary table against the joined sections,
  with the new text in its place, so neither an added row nor a removed entry
  can leave a command without its entry (D-144).
- The summary table (the indented rows after the line starting `Commands (`)
  names the commands `help` answers: each row's first word must have its own
  two-space entry in the section (or, for the workflow alone, in its sibling
  sections), so `help <command>` answers one paragraph (D-144).
"""
import re
import sys
from collections import Counter
from pathlib import Path

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


def has_summary(lines):
    """Whether the text has the workflow's command summary table."""
    return any(line.startswith("Commands (") for line in lines)


def entries(lines):
    """The command entries before the host commands: every indent-2 row."""
    end = next((k for k, line in enumerate(lines) if line.startswith(HOST)), len(lines))
    return [lines[k] for k in range(end) if lines[k].startswith("  ") and not lines[k][2:3].isspace()]


def summary_faults(lines, command_entries, before=None):
    """A summary-table row with no command entry (D-144), as (index, message)."""
    out = []
    for i, line in enumerate(lines if before is None else lines[:before]):
        if not line.startswith("Commands ("):
            continue
        j = i + 1
        while j < len(lines) and lines[j].strip() and not lines[j].startswith("  "):
            j += 1
        while j < len(lines) and lines[j].startswith("  ") and not lines[j][2:3].isspace():
            name = lines[j].split()[0]
            if not any(name in row_names(entry) for entry in command_entries):
                out.append((j, f'line {j + 1}: no entry in "{SECTION}" for `{name}`'))
            j += 1
        break
    return out


def description_column(lines, runs, column, has_section):
    """The description column most rows use; wrapped rows infer it when none do."""
    found = [c for c in column.values() if c is not None]
    if found:
        return Counter(found).most_common(1)[0][0], None
    if not column:
        return None, None
    deeper = [indent(lines[i]) for run in runs for i in run if indent(lines[i]) > 2]
    if has_section or not deeper:
        return None, "no command row has its description on the same line"
    return Counter(deeper).most_common(1)[0][0], None


def reference_root(path):
    """The Solace root holding the reference `path` belongs to, when one is above it."""
    return next((p for p in path.parents if (p / "src/reference.rs").is_file()), None)


def reference_order(root):
    """The reference's section files, in REFERENCE order."""
    source = (root / "src/reference.rs").read_text(encoding="utf-8")
    active = "\n".join(line for line in source.split("\n") if not line.lstrip().startswith("//"))
    return re.findall(r'include_str!\("(reference/[^"]+)"\)', active)


def joined_sections(path, text=None):
    """The command sections' text in reference order, with `text` in place of
    the file being checked; the workflow's summary table is not a section."""
    here = path.resolve()
    root = reference_root(path)
    order = reference_order(root) if root is not None else []
    if not order:
        order = [f"reference/{p.name}" for p in sorted(path.parent.glob("*.txt"))]
    parts = []
    for rel in order:
        sibling = (root / "src" / rel) if root is not None else path.parent / Path(rel).name
        if sibling.resolve() == here:
            if text is not None:
                parts.append(text)
        elif sibling.is_file():
            section = sibling.read_text(encoding="utf-8")
            if not has_summary(section.split("\n")):
                parts.append(section)
    return "".join(parts)


def reference_workflow(root, path, text):
    """The first REFERENCE section with a summary, with `text` in place of `path`."""
    here = path.resolve()
    for rel in reference_order(root):
        section = (root / "src" / rel).resolve()
        if section == here:
            candidate = text
        elif section.is_file():
            candidate = section.read_text(encoding="utf-8")
        else:
            continue
        if has_summary(candidate.split("\n")):
            return candidate
    return None


def faults(text, joined=None):
    """One message per fault, in line order."""
    lines = text.split("\n")
    section = lines.index(SECTION) if SECTION in lines else -1
    if section < 0 and has_summary(lines):
        if not joined:
            return []
        return [message for _, message in summary_faults(lines, entries(joined.split("\n")))]
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
    d, fault = description_column(lines, runs, column, section >= 0)
    if fault:
        return [fault]
    if d is None:
        return []
    out = []
    # Every command in the summary table needs its own entry in the section,
    # so `help <command>` answers one paragraph (D-144). `help` never shows the
    # host commands, so entries there don't count.
    if section >= 0:
        out.extend(summary_faults(lines, entries(lines[section + 1:]), before=section))
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
    path = Path(sys.argv[1])
    text = path.read_text(encoding="utf-8")
    lines = text.split("\n")
    workflow_write = has_summary(lines)
    root = reference_root(path)
    reference_dir = (root / "src/reference").resolve() if root is not None else None
    workflow = None
    if reference_dir is not None and reference_dir in path.resolve().parents:
        workflow = reference_workflow(root, path, text)
        joined = joined_sections(path, None if workflow_write else text)
    else:
        joined = joined_sections(path) if workflow_write else None
    found = faults(text, joined)
    if workflow is not None and not workflow_write:
        found += [message for _, message in summary_faults(workflow.split("\n"), entries(joined.split("\n")))]
    for fault in found:
        print(fault)
    sys.exit(1 if found else 0)


if __name__ == "__main__":
    main()
