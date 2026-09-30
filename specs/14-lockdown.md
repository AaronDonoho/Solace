# Spec 14: Lockdown — Tiers, Project Settings and Agent Commands

**Status:** accepted 2026-09-29 · **Implements:** D-16 (points 8–10), PROPOSAL.md §14.7 · **Needed by:** the D-16 work. Not in the PROPOSAL.md §18.2 table; numbered after it.

Solace is installed once and wraps a user's project. An agent does all of its work in that project through the Solace MCP tool. It has no shell and no file tools, only its own web access. Solace decides what each of the project's files is and what the agent may do with it. "Gleam:" marks what applies to a Gleam project, which is today's only target language.

Terms: a **validator** is a program declared for a set of files that decides whether a file's content is acceptable, such as an HTML linter or a JSON Schema checker. The **proposed text** is what a `write` would put in the file, before Solace has accepted it.

## 1. Tiers

Every file under the project root is in exactly one tier. The first rule that matches decides:

| # | Rule | Tier |
| --- | --- | --- |
| 1 | Source the store holds as symbols (Gleam: `src/**/*.gleam`, `test/**/*.gleam`) | 1 source |
| 2 | The target's manifest (Gleam: `gleam.toml`) | 2 manifest |
| 3 | `solace.toml` (§2) | settings |
| 4 | `.solace/`: the store, the task board's data (Spec 15), and Solace's working files | internal: no agent access at all (`E6006`) |
| 5 | `.git/`, the target's build and dependency output (Gleam: `build/`, `manifest.toml`), anything `.gitignore` ignores | 4 ignored |
| 6 | A path registered in `ignore` (§2) | 4 ignored |
| 7 | A path matching a `resources` pattern (§2), within `resource_limit` and not binary | 3 resource |
| 8 | Everything else, including resources over the limit or binary | 4 ignored |

- A file caught by rule 8 despite matching a `resources` pattern is marked `over limit` or `binary` wherever tiers are shown.
- Tiers 3 and 4 are readable and writable through file commands and never enter the symbol index (D-26). Solace keeps no permanent file history (source control does). It keeps the content hash last read or written in each session (§3), and, for direct task edits, the file as it was before the task first changed it (Spec 15 §5).

## 2. Project settings

`solace.toml` in the project root, versioned with the project. Import creates a starter file if absent and never overwrites existing settings (D-26). **Humans write it**, through the UX or an editor. Agents can read it (`read solace.toml`) but not write it. An agent that needs a change raises a decision request. Resource patterns classify files; they are not a file-write allowlist.

```toml
resource_limit = 262144          # bytes; default 256 KiB

resources = ["priv/**/*.html", "priv/**/*.css", "docs/**/*.md", "src/**/*.mjs"]
ignore    = ["priv/generated/", "data/dumps/"]

[validators]                     # optional, per resource pattern
"priv/**/*.html" = ["tidy", "-q", "-e", "{file}"]

[commands]                       # beyond the built-ins in §4
smoke   = { run = ["gleam", "run", "-m", "smoke"], timeout = 60 }
migrate = { run = ["gleam", "run", "-m", "migrate"], args = true }
```

Solace rereads the file whenever a command starts, so a change takes effect with no restart.

## 3. File commands

| Command | Tiers | Effect |
| --- | --- | --- |
| `files [<dir>]` | all | Tree with each entry's tier. A tier 4 directory is one line with a file count; it is not expanded |
| `read <path> [--lines a..b \| --tail n]` | 2, 3, 4, settings | File text, with line numbers when a range is asked for. Stays within the reply budget: over it, the reply stops at a whole line and says which `--lines` range comes next |
| `write <path>` (`text`) | 2, 3, 4 | Creates or replaces the whole file |
| `write <path> --replace` (`old`, `text`) | 2, 3, 4 | One exact replacement in UTF-8 text, the same rules as `submit --replace`: zero or several matches are refused |
| `remove <path>` | 3, 4 | Deletes the file |

- **Tier 1 is refused** by `read`, `write` and `remove` (`E6001`), with the `inspect`, `view` or `submit` command that does the same thing.
- **A new file** may land in tier 3 or 4 by the rules in §1. `README.md` and `bin/coolcalc` need no resource declaration. Source, settings and internal-file restrictions still apply. File writes do not set executable permissions; execution remains subject to §4.
- **Validation is reversible, in place.** Solace saves the file's current bytes to `.solace/pending/`, writes the proposed text to the real path, and runs the validator in the project root with `{file}` set to that path. This way a validator that needs the whole directory, such as `terraform validate`, sees the project as it would be. If the validator fails, Solace restores the saved bytes (or removes a new file), and the validator's output is the diagnostic (`E6003`). If Solace stops mid-write, it restores anything left in `.solace/pending/` on its next start. A file with no validator is accepted as plain text.
- **Tier 2's validator only checks that the manifest is well-formed** (Gleam: `gleam.toml` parses and has the fields Gleam requires). Resolving dependencies and type-checking happen at `check` and `build`, and breakage is reported there (as D-4 allows broken dependents).
- **Size.** A `write` whose result exceeds `resource_limit` is refused (`E6004`).
- **Stale writes.** A whole-file `write` to an existing file is refused (`E6005`) unless the file's hash still matches this session's last `read` or `write` of it. `--replace` needs no earlier read, because the exact match is the check.

## 4. Commands the agent can run

- **Built-in:** `check`, `test` and `format` (as today), plus `build`, which runs the target's build (Gleam: `gleam build`).
- **Declared:** `run <name> [args…]` runs a command from `[commands]`. Unknown names are refused (`E6010`), listing the declared ones.
- Commands run **without a shell**: the `run` array is the argument vector. The agent may append arguments only if the command declares `args = true` (otherwise `E6011`).
- **Dependencies stay in the project.** Commands fetch and build dependencies into the project's own directories (Gleam: `build/packages/`). They never install anything into the machine's global toolchain. A shared download cache (Gleam's Hex cache) is allowed, because it doesn't change how any other project builds.
- They run in the project root, on the projection of the current store, with a timeout (default 120 s). A timed-out command is killed, and the reply says so (`E6012`).
- The reply gives the exit code, then the output. Output over the reply budget keeps the first 20 lines and as much of the end as fits, and says how many lines were left out.

## 5. Agent configuration

`solace agent-config codex|claude <dir>` writes the configuration an agent is launched with:

- the Solace MCP server, and the agent's own web access;
- no shell, no file tools, and no sub-agents, plugins or hooks;
- for Codex, a read-only sandbox, as in the benchmark harness (D-11);
- for Claude Code, only the `mcp__solace__solace`, `WebFetch` and `WebSearch` tools.
- a tool-call timeout at least as long as the task timeout, so an agent can wait for a human's answer (Spec 15 §6).

The board's agent runner uses this configuration. `bench.py` moves to it too, so the benchmark arm and real use can't drift apart.

## 6. Diagnostics

The range `E60xx` is files, settings and commands, in Spec 3 §6's format. `E6001`–`E6006` cover files (§1, §3), `E6010`–`E6012` commands (§4), and `E6020` is an invalid `solace.toml`, reported by every command until it is fixed.

## 7. Out of scope

- Humans editing tier 1 files on disk. Solace doesn't detect it or keep such edits: it rewrites tier 1 files from the store whenever it needs to (D-16).
- Secrets and environment variables for commands (commands inherit Solace's environment).
- Network limits on commands; more than one project per server.

## Tests (acceptance)

- A1. A fixture project covering all eight rules in §1 gives a golden `files` output. A `resources` match over `resource_limit` is shown as `over limit`, and one that `.gitignore` ignores is tier 4.
- A2. `read` or `write` on a tier 1 file returns `E6001` and names the matching `inspect`/`submit` command. Any file command on `.solace/` returns `E6006`.
- A3. `read --tail 10` on a 100 MB tier 4 log returns 10 lines within the budget. A plain `read` of it stops within the budget and names the next range.
- A4. `write` creates a new resource or ignored file. Tier-4 files can be read, replaced and removed, never appear in symbol searches, enforce stale-write and size checks, and direct task edits can be reverted. Text replacement of non-UTF-8 bytes is refused unchanged. Import creates valid default settings if absent and preserves existing settings byte for byte.
- A5. A failing validator leaves the file byte-for-byte unchanged (a new file is removed) and returns `E6003` with its output. A passing one leaves the new text. A validator that reads a second file in the directory sees the proposed text in place. A file left in `.solace/pending/` is restored at the next start.
- A6. `write gleam.toml` with a syntax error returns `E6003`, and `gleam.toml` is unchanged. A well-formed `gleam.toml` naming a dependency that doesn't exist is written, and the following `check` reports the failed resolution.
- A7. A file changed on disk after the session read it: a whole-file `write` returns `E6005`, while a `--replace` whose `old` still matches succeeds.
- A8. `write solace.toml` is refused. A human's edit to `solace.toml` applies to the next command.
- A9. `run smoke extra` without `args = true` returns `E6011`, and `run nope` returns `E6010`, listing the declared commands. A declared command that sleeps past its timeout is killed and returns `E6012`.
- A10. The `agent-config` output for both agents matches golden files, and has no shell or file tool enabled.
