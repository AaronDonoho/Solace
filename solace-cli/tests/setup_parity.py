#!/usr/bin/env python3
"""Checks the setup guide and the first-run checklist from one place.

`solace-cli/src/board/installs.json` names the tools the checklist checks and
the install steps it shows for each system. The README's "What Solace uses"
table names the tools, and its per-system sections show the same install
steps. A tool, command or package in one and not the other is a fault: the
app and the install guide drift apart.

Usage: setup_parity.py
Exit 0 when they agree; exit 1 with one line per fault.
"""
import json
import re
import sys
from pathlib import Path

CLI = Path(__file__).resolve().parent.parent
README = CLI.parent/'README.md'
INSTALLS = CLI/'src/board/installs.json'
PLATFORMS = {
    'macos': 'macOS',
    'debian': 'Ubuntu or Debian',
    'arch': 'Arch Linux',
}
PACKAGE_INSTALLS = ('brew install ', 'sudo pacman -S --needed ', 'sudo apt install -y ')

def rel(path):
    return str(path.relative_to(CLI.parent))

def install_table():
    """The checklist's tools and install steps, as board::prerequisites uses them."""
    return json.loads(INSTALLS.read_text(encoding='utf-8'))

def install_commands(table):
    """Each platform's install commands, from the table's matching install arms."""
    out = {}
    for arm in table['installs']:
        commands = [step['text'] for step in arm['steps'] if step['command']]
        for platform in arm['platforms']:
            if platform in PLATFORMS:
                out.setdefault(platform, []).extend(commands)
    return out

def homebrew_command(table):
    """The Homebrew installer command, as the table sets it."""
    return next((step['text'] for step in table['homebrew'] if step['command']), None)

def setup_tools(table):
    """The checklist's tool labels, in the order it asks for them."""
    return [tool['label'] for tool in table['tools']]

def readme_setup_section():
    """The README's "What Solace uses" section, or nothing when it is missing."""
    text = README.read_text(encoding='utf-8')
    if '### 1. What Solace uses' not in text: return ''
    return text.split('### 1. What Solace uses', 1)[1].split('\n### ', 1)[0]

def readme_setup_tools():
    """The tools in the README's "What Solace uses" table, in order."""
    section = readme_setup_section()
    cells = [cell.strip() for cell in re.findall(r'^\| ([^|]+?) \|', section, re.M)]
    return [cell for cell in cells if cell != 'Tool' and not cell.startswith('-')]

def readme_platform(platform):
    """One platform's setup commands in the README's "What Solace uses" section."""
    heading = PLATFORMS[platform]
    found = re.search(rf'\*\*{re.escape(heading)}[^*]*\*\*(.*?)(?=\n\*\*|\Z)', readme_setup_section(), re.S)
    return found.group(1) if found else ''

def readme_packages(section):
    """The tools the README installs with a package manager, by command prefix."""
    out = {prefix: set() for prefix in PACKAGE_INSTALLS}
    for line in section.splitlines():
        line = line.split('#', 1)[0].strip()
        for prefix in PACKAGE_INSTALLS:
            if line.startswith(prefix):
                out[prefix].update(line[len(prefix):].split())
    return out

def shell_commands(section):
    """The whole commands a README section runs, including a `# or:` alternative.

    Splitting on `&&` and `;` compares commands rather than substrings, so a
    short `cd -` does not match inside `cd /tmp`.
    """
    out = []
    for line in section.splitlines():
        line = line.strip()
        if not line or line.startswith('```'):
            continue
        code, _, comment = line.partition('#')
        for part in (code, re.sub(r'^\s*or:\s*', '', comment)):
            part = part.strip()
            if not part:
                continue
            out.append(part)
            out.extend(piece.strip() for piece in re.split(r'&&|;', part) if piece.strip())
    return out

def setup_faults(table):
    """One line per tool the checklist and the README's install section disagree on."""
    want, have = setup_tools(table), readme_setup_tools()
    out = []
    if not want: out.append(f'{rel(INSTALLS)}: no tool labels found')
    missing = [tool for tool in want if tool not in have]
    extra = [tool for tool in have if tool not in want]
    if missing: out.append(f'{rel(README)}: the setup list is missing {", ".join(missing)}')
    if extra: out.append(f'{rel(README)}: the setup list adds {", ".join(extra)}')
    if want and not missing and not extra and want != have:
        out.append(f'{rel(README)}: the setup tools are out of order')
    out += install_faults(table)
    return out

def install_faults(table):
    """One line per install step the checklist runs for a platform and the README omits."""
    commands = install_commands(table)
    out = [] if commands else [f'{rel(INSTALLS)}: no install commands found']
    for platform, heading in PLATFORMS.items():
        if platform not in commands:
            out.append(f'{rel(INSTALLS)}: no {heading} install commands found')
            continue
        section = readme_platform(platform)
        if not section:
            out.append(f'{rel(README)}: no {heading} setup section')
            continue
        packages = readme_packages(section)
        shown = shell_commands(section)
        wanted = {prefix: set() for prefix in PACKAGE_INSTALLS}
        for command in commands[platform]:
            prefix = next((prefix for prefix in PACKAGE_INSTALLS if command.startswith(prefix)), None)
            if prefix:
                wanted[prefix].update(command[len(prefix):].split())
            elif command not in shown:
                out.append(f'{rel(README)}: the {heading} setup is missing "{command}"')
        for prefix in PACKAGE_INSTALLS:
            missing = wanted[prefix] - packages[prefix]
            extra = packages[prefix] - wanted[prefix]
            if missing:
                out.append(f'{rel(README)}: the {heading} setup is missing {", ".join(sorted(missing))} from "{prefix.strip()}"')
            if extra:
                out.append(f'{rel(README)}: the {heading} setup adds {", ".join(sorted(extra))} to "{prefix.strip()}"')
    brew = homebrew_command(table)
    if not brew:
        out.append(f'{rel(INSTALLS)}: no Homebrew installer command found')
    elif brew not in readme_platform('macos'):
        out.append(f'{rel(README)}: the macOS setup is missing "{brew}"')
    return out

def main():
    try:
        table = install_table()
    except (OSError, ValueError) as e:
        sys.exit(f'{rel(INSTALLS)}: {e}')
    faults = setup_faults(table)
    for fault in faults: print(fault)
    sys.exit(1 if faults else 0)

if __name__ == '__main__':
    main()
