# Spec 14: Lockdown — File Categories, Project Settings and Agent Commands

**Status:** accepted 2026-09-29 · **Implements:** D-16 (points 8–10), PROPOSAL.md §6.8 · **Needed by:** the D-16 work. Not in the PROPOSAL.md §14.4 table; numbered after it.

Solace is installed once and wraps a user's project. An agent does all of its work in that project through the Solace MCP tool. It has no shell and no file tools, only its own web access. Solace decides what each of the project's files is and what the agent may do with it. "Gleam:" marks what applies to a Gleam project, which is today's only target language.

Terms: a **validator** is a program declared for a set of files that decides whether a file's content is acceptable, such as an HTML linter or a JSON Schema checker. The **proposed text** is what a `write` would put in the file, before Solace has accepted it.

## 1. File categories

Every file under the project root is in exactly one category, named for what Solace does with it (D-52). The first rule that matches decides:

| # | Rule | Category |
| --- | --- | --- |
| 1 | Source the store holds as symbols (Gleam: `src/**/*.gleam`, `test/**/*.gleam`; none in a project of plain files, D-60) | graph |
| 2 | `solace.toml` (§2) and `AGENTS.md` (D-45) | protected |
| 3 | `.solace/`: the store, the task board's data (Spec 15), and Solace's working files; and every file of another Solace project nested in this one (a folder below the root with its own `.solace/store.db`, D-57) | internal: no agent access at all (`E6006`) |
| 4 | `.git/`, the target's build and dependency output (Gleam: `build/`, `manifest.toml`), anything `.gitignore` ignores | unwatched |
| 5 | A path registered in `unwatched` (§2) | unwatched |
| 6 | A file over `watch_limit` or binary (not UTF-8, or containing a NUL byte) | unwatched, marked `over limit` or `binary` |
| 7 | Everything else, including the target's manifest (Gleam: `gleam.toml`) (D-28) | watched |

- Watched and unwatched files are readable and writable through file commands and never enter the symbol index (D-26). Solace keeps no permanent file history (source control does). It keeps the content hash last read or written in each session (§3), and, for direct task edits, the file as it was before the task first changed it (Spec 15 §5).

## 2. Project settings

`solace.toml` in the project root, versioned with the project. Import creates a starter file if absent and never overwrites existing settings (D-26). **Humans write it**, through the UX or an editor. Agents can read it (`read solace.toml`) but not write it. In a work run, `write solace.toml` proposes the change for a human to approve on the task board (D-38, Spec 15 §6). `unwatched` patterns classify files; they are not a file-write denylist. The keys `resources`, `ignore` and `resource_limit` were removed by D-52 and make the file invalid (`E6020`), with a message saying what to do.

A root project's `solace.toml` may also list `projects`, the folders below it that share its board (D-57, Spec 15 §4). Only humans change that list, from the board or with `solace create`: an agent's proposed `solace.toml` that changes `projects` is refused (`E6020`).

`AGENTS.md` in the project root holds the project's instructions for agents and is shown first in `session start` (Spec 3). It follows the same rule as `solace.toml`: humans write it, agents read it, and a work run's `write AGENTS.md` is a proposal the human approves (D-45).

```toml
watch_limit = 262144             # bytes; default 256 KiB

unwatched = ["priv/generated/", "data/dumps/"]

[validators]                     # optional, per file pattern
"priv/**/*.html" = ["tidy", "-q", "-e", "{file}"]

[commands]                       # beyond the built-ins in §4
smoke   = { run = ["gleam", "run", "-m", "smoke"], timeout = 60, checks = [] }
migrate = { run = ["gleam", "run", "-m", "migrate"], args = true }
e2e     = { run = ["npx", "playwright", "test"], timeout = 600, checks = ["api.handle"] }
```

`checks` (optional, D-44) marks a command as a test suite outside `gleam test` and lists the entry points it drives, by symbol name, or `"*"` for all. For `gleam run -m <module>`, that module's `main` is added, so `checks = []` is enough. A suite's reach is everything its entry points can run. `test`, `impact` and a submit's note name a suite that reaches a change ("Also reached: e2e (`run e2e`)"); Solace never runs a suite unasked. `checks` that isn't an array of strings is invalid (`E6020`).

Solace rereads the file whenever a command starts, so a change takes effect with no restart.

## 3. File commands

| Command | Categories | Effect |
| --- | --- | --- |
| `files [<dir>]…` | all | Tree with each entry's category, for each path in the order named; no path is the project root. Each file other than graph source ends with its line count (a UTF-8 file) or its size in bytes (any other, or one over `watch_limit`) (D-119). An unwatched directory, or one holding nothing graph, watched or protected, is one line with a file count; it is not expanded |
| `grep <text> [--in <path>] [--context n]` | watched, protected | Case-insensitive search of file text: each matching file with up to 10 matching lines and their numbers, long lines cut around the match, at most 40 files; a file with more matches heads with `(N matches, showing 10)`. `--context n` (1 to 5; another number is a usage error, the one refusal besides a missing `--in` path) adds n lines before and after each match, marked `12-` against a match's `12:`, touching windows merged and gaps shown as `…`. `--in <path>` limits the search to one file or folder, checked as `read` checks a path; an unwatched or internal one is named as not searched. Both are recognized only as the last words of the query, in either order, so the text before them is searched as written. The reply stops at a whole file within the shared reply budget, says how many files it left out and to narrow with `--in` (D-119). Unwatched files aren't searched, and graph source is `find`'s. Works in every project, plain-files ones included. The text is searched as written and never refused for syntax; quotes are dropped only when they wrap the whole query as one term. A miss whose query looks like a shell habit (a leading option, `\|`, `\"`, a quoted term with more after it, a trailing `--path <x>` or file path) adds one line with the text searched and the syntax, and for the path forms says to use `--in <path>` (D-109, D-119) |
| `read <path>… [--lines 3,7,10..12 \| --tail n]` | watched, unwatched, protected | File text for each path in the order named, with line numbers when lines are asked for. `--lines` takes lines and ranges (`150,156,200..210`), shown in file order with overlaps merged and a `…` line at each gap (D-105); a `--lines` or `--tail` range takes one path, and with several paths is refused. Stays within the reply budget: over it, the reply stops at a whole line and says which `--lines` lines come next. Every path is checked before any is read: one that doesn't exist is refused, as a line range (`1340-1400`, `a.rs:5-9`) with the `--lines` form to use, as a symbol-like name (no `/` or `.`) with `grep` and `find`, or as no such file (D-109) |
| `write <path>` (`text`) | watched, unwatched | Creates or replaces the whole file; an existing file must have been read in full or by `--lines` first (a `--tail` read doesn't count). Patch text is refused (usage error; it goes to `write` with no path), and so is a text that would leave fewer than half the lines of an existing file of 200 or more (`E6002`; `remove` it first to replace it). The reply gives the lines before and after (D-109) |
| `write <path> [--replace]` (`old`, `text`) | watched, unwatched | One exact replacement in UTF-8 text, the same rules as `submit --replace`: zero or several matches are refused. An `old` text makes it a replacement with or without `--replace`. Patch text is refused here too (D-109) |
| `write` (`text`: a file patch) | watched, unwatched | Adds, updates and deletes several files, whole or nothing (D-104): `*** Add File: <path>`, `*** Update File: <path>` (`@@` hunks, no earlier read needed) and `*** Delete File: <path>` sections. A refused patch is kept as a draft; `write --draft <id> [--replace [--all]]` (`old`, `text`) retries it (D-120) |
| `remove <path>…` | watched (except `gleam.toml`), unwatched | Deletes each file, whole or nothing (D-102) |
| `move <from> <to> [--to-project <folder>]` | watched (except `gleam.toml`), unwatched, on both sides | Moves a file or folder in one rename that never replaces anything; `--to-project` names another project of the task's scope (D-84) |

- **Moves (D-84)** are whole or not at all. Every file of a moved folder must be movable at both places, or nothing moves. Anything at the destination is refused (`E6002`), and so is a move between file systems. A repeat of a move the session made changes nothing. In a board run, each moved file is recorded as removed in its source project and added in its destination project.
- **Lists (D-102).** `files` and `read` take a list of paths and answer each in the order named. With several paths, `read` heads each file with `==== <path> ====` and shares the reply budget: a file cut short ends the reply, and the paths not read are named in a `Next: read <paths>` line. A whole-file read cut short doesn't count as reading the file for a whole-file `write`. `remove` takes a list too, whole or nothing: every path is checked first, and if any is refused (graph, protected, internal, `gleam.toml`, no such file) nothing is removed and the reply names each refused path. `write <path>` changes one file and refuses a list (usage error); several files change in one `write` with no path, by patch.
- **File patches (D-104).** `write` with no path takes `text` as a patch in `apply_patch` form, a section per file. Update hunks match as `submit`'s do (Spec 3); lines added with no `@@` line go at the end. Every section is checked and applied in memory before any file changes: a graph file (`E6001`), a protected or internal one (`E6002`, `E6006`), an Add onto an existing file, an Update or Delete of a missing one, a hunk that doesn't match (`E0108`), a path named twice or a `*** Move` refuses the whole patch, and each refused section is named after `nothing written:`. Then every file is written before any is validated, so a validator sees the project as it will be; if one fails, every file goes back. In a board run each file is recorded as the task's change.
- **File patch drafts (D-120).** A refused file patch is kept as written, as a draft of the task (or `cli`), and the refusal names it. `write --draft <id> --replace` (`old`, `text`, `--all` as for `submit`) edits the draft and retries it; `write --draft <id>` retries it as is. A retry is the whole patch again, under the rules above; one that applies removes the draft, one refused keeps the edited draft. A draft has no fingerprint: its hunks are matched against the files as they are at the retry. `drafts` and `view --draft <id>` list and read it, in plain-files projects too; `submit --draft` refuses it (`E0106`) and names the `write` form. In a board run, a retry records every file the draft names, before and after the edit.

- **Graph files are refused** by `read`, `write` and `remove` (`E6001`), with the `inspect`, `view` or `submit` command that does the same thing.
- **A new file** is watched or unwatched by the rules in §1. Graph, protected and internal restrictions still apply. File writes do not set executable permissions; execution remains subject to §4.
- **Validation is reversible, in place.** Solace saves the file's current bytes to `.solace/pending/`, writes the proposed text to the real path, and runs the validator in the project root with `{file}` set to that path. This way a validator that needs the whole directory, such as `terraform validate`, sees the project as it would be. If the validator fails, Solace restores the saved bytes (or removes a new file), and the validator's output is the diagnostic (`E6003`). If Solace stops mid-write, it restores anything left in `.solace/pending/` on its next start. A file with no validator is accepted as plain text.
- **The manifest's built-in validator only checks that it is well-formed** (Gleam: `gleam.toml` parses and has the fields Gleam requires). Resolving dependencies and type-checking happen at `check` and `build`, and breakage is reported there (as D-4 allows broken dependents).
- **Size.** A `write` whose result exceeds `watch_limit` is refused (`E6004`).
- **Stale writes.** A whole-file `write` to an existing file is refused (`E6005`) unless the file's hash still matches this session's last `read` or `write` of it. `--replace` needs no earlier read, because the exact match is the check.

## 4. Commands the agent can run

- **Built-in:** `check`, `test` and `format` (as today), plus `build`, which runs the target's build (Gleam: `gleam build`).
- **Declared:** `run <name> [args…]` runs a command from `[commands]`. Unknown names are refused (`E6010`), listing the declared ones.
- Commands run **without a shell**: the `run` array is the argument vector. The agent may append arguments only if the command declares `args = true` (otherwise `E6011`).
- **Dependencies stay in the project.** Commands fetch and build dependencies into the project's own directories (Gleam: `build/packages/`). They never install anything into the machine's global toolchain. A shared download cache (Gleam's Hex cache) is allowed, because it doesn't change how any other project builds.
- They run in the project root, on the projection of the current store, with a timeout (default 120 s). A timed-out command is killed, and the reply says so (`E6012`).
- The reply gives the exit code, then the output, condensed (D-111): failures with their messages and tracebacks, errors, panics and summary lines (`test result:`, `N tests, M failed`) stay, in order; build progress, passing-test lines and the text of warnings are left out, and warnings stay as a count. Output over the reply budget keeps the first 20 lines and as much of the end as fits, and says how many lines were left out.

## 5. Agent configuration

`solace agent-config codex|claude <dir>` writes the configuration an agent is launched with:

- the Solace MCP server, and the agent's own web access;
- no shell, no file tools, and no sub-agents, plugins or hooks;
- for Codex, a read-only sandbox, as in the benchmark harness (D-11);
- for Claude Code, only the `mcp__solace__solace`, `WebFetch` and `WebSearch` tools.
- a tool-call timeout at least as long as the task timeout, so an agent can wait for a human's answer (Spec 15 §6).

The board's agent runner uses this configuration. `bench.py` moves to it too, so the benchmark arm and real use can't drift apart.

## 6. Diagnostics

The range `E60xx` is files, settings and commands, in Spec 3 §6's format. `E6001`–`E6006` cover files (§1, §3), `E6007` a read in another project refused (Spec 3 §2, D-57), `E6010`–`E6012` commands (§4), `E6013` a Gleam command in a project of plain files (D-60; for `view`, `inspect` and `refs` it names `view <file>` and `view <file>::<name>`, which read watched `.rs` and `.py` files by item, D-121), and `E6020` is an invalid `solace.toml`, reported by every command until it is fixed.

## 7. Out of scope

- Humans editing graph files on disk. Solace doesn't detect it or keep such edits: it rewrites graph files from the store whenever it needs to (D-16).
- Secrets and environment variables for commands (commands inherit Solace's environment).
- Network limits on commands; more than one project per server.

## Tests (acceptance)

- A1. A fixture project covering all seven rules in §1 gives a golden `files` output. A file over `watch_limit` is shown as `unwatched (over limit)`, `.git/` and an `unwatched` directory as one collapsed line each, and an unlisted text file as `watched`. `resources`, `ignore` or `resource_limit` in `solace.toml` is `E6020`.
- A2. `read` or `write` on a graph file returns `E6001` and names the matching `inspect`/`submit` command. Any file command on `.solace/` returns `E6006`.
- A3. `read --tail 10` on a 100 MB unwatched log returns 10 lines within the budget. A plain `read` of it stops within the budget and names the next range.
- A4. `write` creates a new watched file, such as `README.md` or `bin/coolcalc`. Watched and unwatched files can be read, replaced and removed (`remove gleam.toml` is refused), never appear in symbol searches, enforce stale-write and size checks, and direct task edits can be reverted. `move` takes a file or folder whole, refuses a destination that exists and graph, protected or internal files, and a repeat changes nothing. Text replacement of non-UTF-8 bytes is refused unchanged. Import creates valid default settings if absent and preserves existing settings byte for byte. Repeating import on an existing store succeeds, creates missing settings, and does not reimport source or change stored symbols.
- A5. A failing validator leaves the file byte-for-byte unchanged (a new file is removed) and returns `E6003` with its output. A passing one leaves the new text. A validator that reads a second file in the directory sees the proposed text in place. A file left in `.solace/pending/` is restored at the next start.
- A6. `write gleam.toml` with a syntax error returns `E6003`, and `gleam.toml` is unchanged. A well-formed `gleam.toml` naming a dependency that doesn't exist is written, and the following `check` reports the failed resolution.
- A7. A file changed on disk after the session read it: a whole-file `write` returns `E6005`, while a `--replace` whose `old` still matches succeeds.
- A8. `write solace.toml` outside a work run is refused. A human's edit to `solace.toml` applies to the next command.
- A9. `run smoke extra` without `args = true` returns `E6011`, and `run nope` returns `E6010`, listing the declared commands. A declared command that sleeps past its timeout is killed and returns `E6012`.
- A10. The `agent-config` output for both agents matches golden files, and has no shell or file tool enabled.
