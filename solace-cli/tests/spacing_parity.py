#!/usr/bin/env python3
"""Checks the values the pages share from one place.

Two kinds of drift the UI polish pass kept finding:

- The spacing and control tokens. `solace_task.spacing_styles` sets them for
  every page, and the shell's `index.html` carries its own copy of the same
  list, because it loads before Gleam runs. A token in one and not the other,
  or with a different value, is a fault.
- The board's column labels. `solace-cli/src/default-board.toml` names them,
  and the pages show its words. A fixture, guide or README that spells a column
  the other way — "Cancelled" for Canceled, "Needs Review" for Review — is a
  fault: the board never shows that spelling, so what a test checks and what a
  person reads drifts from what ships.
Usage: spacing_parity.py [path ...]
With paths, only those files are checked for labels; the token check always
reads its own files. Exit 0 when they agree; exit 1 with one line per fault.
"""
import re
import sys
from pathlib import Path

CLI = Path(__file__).resolve().parent.parent
README = CLI.parent/'README.md'
BOARD = CLI/'src/default-board.toml'
TOKENS = CLI/'components/src/solace_task.gleam'
HTML = CLI/'frontend/index.html'
SKIP = {'target', 'node_modules', '.solace', 'build', 'dist'}

SHARED = ('--space-', '--control-', '--pad-', '--tab-')
HEADINGS = ('--h2', '--h3', '--h4')

# A fixture's column, as Gleam builds it and as a workflow answers it.
FIXTURES = [
    re.compile(r'Column\(\s*"([a-z_]+)"\s*,\s*"([^"]*)"'),
    re.compile(r'"id"\s*:\s*"([a-z_]+)"[^{}\n]*?"label"\s*:\s*"([^"]*)"'),
    re.compile(r"\bid:\s*'([a-z_]+)'[^{}\n]*?\blabel:\s*'([^']*)'"),
]

def rel(path):
    return str(path.relative_to(CLI.parent))

def labels():
    """The board's column labels, id to label, as default-board.toml sets them."""
    out = {}
    for block in BOARD.read_text(encoding='utf-8').split('[[board.columns]]')[1:]:
        found = re.search(r'id\s*=\s*"([^"]+)"', block), re.search(r'label\s*=\s*"([^"]+)"', block)
        if all(found): out[found[0].group(1)] = found[1].group(1)
    return out

def final_columns():
    """The ids of the board's final columns, as default-board.toml sets them."""
    out = set()
    for block in BOARD.read_text(encoding='utf-8').split('[[board.columns]]')[1:]:
        found = re.search(r'id\s*=\s*"([^"]+)"', block)
        if found and re.search(r'^\s*final\s*=\s*true\b', block, re.M):
            out.add(found.group(1))
    return out

def drift(board, finals):
    """The names a column's id suggests, when the board labels it otherwise.

    Derived from the board's own columns, so a new column's drift is caught
    without editing a list here. The id as words is a fixture's or prose's
    slip; `In <label>` covers the common "In Review" form for a column that
    isn't final.
    """
    out = {}
    for cid, label in board.items():
        named = cid.replace('_', ' ').title()
        if named != label:
            out[cid] = [named]
            if cid not in finals:
                out[cid].append('In ' + label)
    return out

def shared(name):
    """A token in the shared list: the spacing, control, padding, heading and tab sizes."""
    return name.startswith(SHARED) or name in HEADINGS

def token_faults():
    """One line per spacing token the shell's copy and spacing_styles differ on."""
    gleam = re.findall(r'(--[a-z0-9-]+):([^;}]+)', TOKENS.read_text(encoding='utf-8'))
    html = re.findall(r'(--[a-z0-9-]+):([^;}]+)', HTML.read_text(encoding='utf-8'))
    want = {n: v for n, v in gleam if shared(n)}
    have = {n: v for n, v in html if shared(n)}
    if not want: return [f'{rel(TOKENS)}: no spacing tokens in the components']
    out = []
    for n in want:
        if n not in have: out.append(f'{rel(HTML)}: {n} is in spacing_styles, not in the shell')
        elif have[n] != want[n]:
            out.append(f'{rel(HTML)}: {n} is {have[n]} in the shell, {want[n]} in spacing_styles')
    out += [f'{rel(HTML)}: {n} is in the shell, not in spacing_styles' for n in have if n not in want]
    return out

def label_faults(paths, board):
    """One line per label that drifts from the board's spelling of its column."""
    drift_names = drift(board, final_columns())
    names = [name for variants in drift_names.values() for name in variants]
    out = []
    for path in paths:
        for n, line in enumerate(path.read_text(encoding='utf-8').split('\n'), 1):
            for word in names:
                if re.search(rf'(?<!\w){re.escape(word)}(?!\w)', line):
                    who = next(cid for cid, variants in drift_names.items() if word in variants)
                    out.append(f'{rel(path)}:{n}: a column is named "{word}"; the board shows "{board[who]}"')
            for pattern in FIXTURES:
                for cid, label in pattern.findall(line):
                    want = board.get(cid)
                    if want and label != want and (
                        label.lower() == want.lower()
                        or label.lower() in {v.lower() for v in drift_names.get(cid, [])}
                    ):
                        out.append(f'{rel(path)}:{n}: column "{cid}" is labeled "{label}"; the board shows "{want}"')
    return out

def checked(args):
    if args: return [Path(a) for a in args]
    files = [README]
    files += [p for p in CLI.rglob('*')
              if p.is_file() and p.suffix in {'.gleam', '.mjs', '.js'} and not set(p.parts) & SKIP]
    return files + [p for p in CLI.rglob('README.md') if not set(p.parts) & SKIP]

def main():
    if not BOARD.exists(): sys.exit(f'no {rel(BOARD)}')
    faults = token_faults() + label_faults(checked(sys.argv[1:]), labels())
    for fault in faults: print(fault)
    sys.exit(1 if faults else 0)

if __name__ == '__main__':
    main()
