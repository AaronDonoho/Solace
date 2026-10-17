# Spec 3: Agent Protocol and Diagnostics

**Status:** accepted 2026-09-27 · **Implements:** PROPOSAL.md §6 · **Needed by:** Phase 1a

The protocol is the agent's environment. It has few, rich commands that return compact text. Anything a human can see or do is available here (parity).

## 1. Transport

**Phase 1a addition (D-34):** Rejected submissions retain their raw text, command and source-store fingerprint outside the symbol index. `drafts` lists the current task's drafts (or `cli` scope outside a board run); `view --draft <id>` reads one. `submit --draft <id> --replace` applies an exact `old` → `text` edit and retries the original submission; `submit --draft <id>` retries unchanged. A successful retry removes the draft. A stale fingerprint refuses the retry (`E0107`) while preserving its text; since D-46 the fingerprint covers only the symbols the draft replaces and the headers of their modules (or of the module it adds to), so a caller's draft still retries after its callee changed; an unavailable or differently owned draft is `E0106`. Finalization or start-over clears task drafts. Syntax errors are retained too, and rejected new modules must not leave projected files behind. Successful submissions summarize warnings by symbol; `check --warnings` exposes full warning details. Errors remain detailed.

**D-46 additions:** `submit <name> --replace --all` replaces every occurrence of `old` and replies "Replaced N occurrences"; without `--all`, several occurrences are refused (`E0105`) with the line of each. A block whose text equals the stored symbol is reported unchanged and not saved; when nothing changed, the type check is skipped. A rejected submit whose every error is a call with the wrong arity or argument types to a project function it doesn't include says to submit that function in the same `submit`.

- A CLI, `solace <command>`, run through the agent's shell. Output is plain text by default; `--json` gives the same content as structured data.
- **One process per call, no daemon**, as long as the latency budget (§7) holds.
- **Agents use MCP** (D-9): `solace mcp` serves the same commands as one tool, `solace(command, text)`, so code is passed without shell quoting and the agent needs no shell.
- **Several commands in one call (D-119).** They run in order and share the reply budget. When a command that changes the project (`submit`, `delete`, `extract`, `merge`, `write`, `remove`, `move`) fails, the commands after it do not run, so a test never runs on code that was not saved: the reply ends `N of M commands failed: <commands>; skipped: <commands>`. A failed read does not stop the call (D-119 point 5). A `task done` after a `test` in the same call is skipped when that test fails, or passes and names a change of the task that no test reaches (`No test reaches <names>.`): the reply lists it among the skipped commands with the reason, `task done (the test failed)` or `task done (no test reaches <names>)`, as `Skipped: <commands>` when nothing failed, and asks the agent to send `task done` again (D-152). From the second call in a row that held one read and succeeded, the reply ends `Hint: put independent reads in one call, one per line.` (D-145).
- **Re-reads after compaction (D-157).** In a task run, the server keeps each symbol it printed in full, with the revision served, on the run in the task store. A call that prints a symbol again at the revision the run was served is taken as lost context: once per window, its reply adds the current views of the other symbols read or changed in the window, within the reply budget, then the run's steps so far, one line each (D-99), the latest that fit. A window runs from one change command (`submit`, `delete`, `extract`, `merge`, `write`, `remove`, `move`) to the next; a change keeps in it only the symbols it changed. A re-read of a symbol changed since it was served adds nothing; runs outside a task and strategy runs keep no reads. Views the reply can't fit are named.
- The store lives at `.solace/store.db` in the project root. `solace` finds it by walking up from the working directory.
- Exit codes: `0` success; `1` diagnostics (the command ran and reported problems); `2` usage error.

## 2. Commands (Phase 1)

| Command | Purpose |
| --- | --- |
| `session start [--scope <ns>…]` | Orientation: the project's `AGENTS.md` first, if there is one (D-45; cut at a whole line past 8,000 bytes, with the `read` range for the rest), then language reference version, policy for the scope, pending and recent decisions, and a symbol index: the names of every symbol in scope, grouped by namespace with its one-line description, with "N more" beyond a per-namespace cap (D-12). Open work records are expanded with the context bundles of the symbols they name (D-10). In a board run, `--task` also gives the task's **Depends on** section when one of its dependencies is unfinished, as the run prompt does (Spec 15 §6, D-110). Outside a strategy run, the last line names a first call to send: with `--task`, `inspect` of its top three symbols; otherwise an example (D-145) |
| `inspect <name>… [--context]` | Each symbol's text, signature, effects, description, verification status. `--context` adds the bundle in §3. Several names return several bundles in one document (D-10) |
| `view <ns> \| <name>… \| --callers <name> \| --callees <name>` | Multi-symbol document (Spec 2 §3). A namespace is shown as an outline (signature, effects, description, verification status per symbol); `--bodies` gives full text (D-10) |
| `view` / `inspect <dependency module>[.<name>]` | A name not in the store is looked up in the dependencies' source (`build/packages/*/src/`), read-only and never stored (D-15): a module gives the first paragraph of its doc and each public item's first doc line and declaration (functions without bodies, opaque types without constructors); a symbol gives its whole doc and declaration. Private items stay out (`E1001`). Store names win |
| `view <file>` / `view <file>::<name>` | A watched `.rs` or `.py` file, by its path or an unambiguous path suffix, read by a line scanner rather than the store (D-121): an outline, one line per item with its line range, signature and first doc line (`Type::method`, `tests::name`, `Class.method` indented under their block), or one item's numbered source with its docs and attributes, headed `path:start..end`. A name matches in full or by suffix; an ambiguous one lists its candidates (`E1002`), a miss names `grep --in` (`E1001`). Over the reply budget the outline gives names, then cuts at a whole line |
| `help [<command>]` | Over MCP (D-46): the full reference without host commands, and task commands only in a board run; with a command, the reference paragraphs that list it |
| `find <query>` | Search names, signatures and descriptions. Ranked, at most 20 hits, one line each |
| `submit <name>` / `submit --new <ns>` / `submit --view` | Text on stdin (Spec 2 §2, §3). Since D-47 a plain `submit` also takes a patch in `apply_patch` form (`*** Update: <symbol or module>` hunks, `*** Add: <module>.<name>`), applied to the stored text and submitted as the blocks it changes; a hunk that doesn't match refuses the whole patch (`E0108`). When the first unmatched line is found only inside one longer line, `E0108` quotes that line (with its number in a file) and says a patch line is a whole line, pointing to `--replace` (D-120). A hunk line matches exactly, trailing space aside: leading space is part of the line. A hunk that doesn't match names the first hunk line with no exact match in the target: when the target has that line only with other leading space, `E0108` names its number and how much space it has (D-142); when every hunk line is in the target, `E0108` says the lines aren't adjacent. Every line of an `*** Add:` (and of `write`'s `*** Add File:`), empty ones too, starts with `+`; `*** Add: <module>` creates a module with its module comment, imports and code, the one way the brief teaches. An `E0108` whose fault is in the whole patch, not in one line (an Add with no `+` line, no section, a line before the first header, an unknown header, an Add that defines nothing, a file section sent to `submit`, a patch that changes nothing) says to send the corrected patch again whole, and its draft footer leaves out `--replace`; `E1003` for an unknown module points to `*** Add: <module>` (D-141) |
| `rename`, `copy`, `merge`, `delete`, `extract` | Spec 1 §5. `delete <name>…` takes a list, checked over the whole list before anything is retired (D-102). `extract <symbols…> --to <module> [--to-project <folder>]` moves symbols in one type-checked transaction (D-103), with the JavaScript files their FFI paths name (`E5007` when one can't go, D-124). `merge <survivor> <retired> [--to-project <folder>]` redirects every use of the retired symbol to the survivor in one type-checked transaction; `extract`'s `E1004` names it when the target already has the name (D-117) |
| `check [<name>…]` | Fast check; the default is everything changed in the current change |
| `test [<name>…] [--all]` | The focused plan (D-44): the tests whose reach (everything they can run, with its revisions, module headers and the project and dependency manifests) changed since they last passed, plus those that failed or never ran and those that reach other processes, the network or foreign code. With names, the tests that reach those symbols; `--all`, every test. The reply says how many ran of how many, and names declared suites (Spec 14 §2) that reach a change |
| `impact [<name>…]` | What a change reaches (D-44): callers, and readers of the record fields its data flows into, certain ones nearest first and possible ones as a count (D-48), with its own tests (D-36.3 coverage) and where the data leaves the program. No names: the changes since tests last passed, or the task's changes in a board run, where `impact` with no names is `task diff` with no names (Spec 15 §6, D-102). A successful submit gives the first three lines of it |
| `refs <name>…` | Everything that uses each named symbol, in the order named (D-102) |
| `refs <ns>.<Type>.<field>` | The code that reads one record field, certain or possible (D-44) |
| `diff` | Semantic diff of the current change: symbols added, modified, renamed and retired; signature, effect and contract changes listed first |
| `commit -m <msg>` | Publish the current change (§4) |
| `work open\|update\|close\|list` | Work records (§5) |
| `decide request\|list\|answer` | Decision requests (§5) |
| `export`, `import` | Spec 2 §4, §5 |

Every command that takes a name resolves it per Spec 1 §4 and fails with the candidate list.

**Other projects (D-57).** `view`, `inspect`, `refs`, `find`, `files` and `read` take `--project <folder>` to read another project under the same Solace root (Spec 15 §4, Workspaces): the folder as the root lists it, `.` for the root's own project. The read uses that project's store and file categories, takes none of its locks, and changes nothing in it. Any other command with `--project`, or a folder that isn't one of the root's projects, is refused with `E6007`, except in a board run whose task covers that folder (D-61): there every command but `task` runs in that project (a settings proposal names that project's file), locking it and the task's own in folder order, and its changes are recorded on the task. A folder outside the task's scope is refused with `E6007`. When the root has other projects, `session start` names them after the work records.

## 3. Context bundle

`inspect <name> --context` returns one document with these sections, always in this order, omitting empty ones:

1. **Symbol:** canonical text, with its kind, qualified name and short identity
2. **About:** description and rationale; possibly-stale prose is marked `[stale]`
3. **Uses:** signatures (not bodies) of every symbol it references, grouped types → constants → functions
4. **Used by:** direct callers with their signatures, each with its call line (`  > line N: …`); at most 20, then `…and N more`. For a type (D-46): **Constructed by**, each construction or pattern that lists every field (no `..`) as `-- <symbol>, line N: <source line>`, then a count of record updates and patterns with `..` (which need no change), then **Other users** by name
5. **Verification:** examples and tests covering it, last results, verification debt, holes
6. **Decisions:** decision records attached to it or its namespace
7. **Work:** open work records that touch it

The target size is under 2,000 tokens for a typical pilot function. Nothing in a bundle requires a second call to interpret.

Several names give one document (D-14): each signature appears once, and later bundles refer to it as `(above)`. Since D-46 this holds across the commands of one MCP call: `inspect`, `view` and `refs` share what was printed, and `refs` gives the same Constructed by / Other users split for a type. A requested symbol appears in full once. A reply stays within a byte budget set per agent (`solace mcp --budget`, default 32,000, about 8k tokens), shared by all commands in one call. Over that, sections are trimmed in order: Used by (to a count), the test example (to names), Uses (to names). The requested symbols' text is never trimmed. Names that still don't fit are listed at the end with the command that shows them. `view` falls back from bodies to outlines the same way.

## 4. Changes and transactions

- Each session has one **open change**. `submit` and the structural commands apply to it immediately, and each is atomic.
- A submitted symbol must pass its own fast check. Dependents it breaks, such as callers of a changed signature, are tracked as the change's **broken dependents**. They are listed in `diff`, and each submit reports the changes in that set (D-46): the symbols fixed, one line per symbol still broken with the errors already shown, and new errors grouped as in §6. `check` lists every error in full. They clear as each one is fixed. This allows refactors one step at a time.
- `commit` is refused (`E5103`) while the change has broken dependents.
- A change records its **base revisions**: the revision of each symbol it touched, as first read.
- `commit` publishes the change to the main line. If another commit changed any base revision in the meantime, the commit is rejected with `E5101`, listing the symbols and a semantic diff of what changed. Nothing is merged silently.
- **Semantic conflicts:** if another commit changed the signature, effects or contracts of a symbol this change *uses*, even one it didn't edit, `commit` re-runs the fast check on the change and rejects it on failure (`E5102`).
- Phase 1 has no write-scope declaration; the base revisions serve that role. Declared scopes arrive with multi-agent work in Phase 6.
- An `extract` into another project is written ahead: a record in each of its projects before any file changes. If it is interrupted, the next command, revert or cancel in any of those projects undoes it in all of them, keeping files edited outside Solace since, and notes the task (D-135). When another command holds one of those projects, the command is refused with `E7028`; try again.

## 5. Work records and decisions

- **Work record:** goal, steps done, steps remaining, blockers, and the symbols it touches. `session start` and `inspect --context` show open records. A record belongs to the store, not to the session, so any later session can continue it.
- **Decision request:** a question, 2–4 options each with its consequences, and the affected symbols. It stays `pending` until a human answers. `decide answer` records the chosen option, who chose it, when, and an optional rationale. A hole with a question (Spec 4) raises a decision request automatically.

## 6. Diagnostics

Every diagnostic has:

```
<code> <title>
  at <qualified name>[ #<expr>]: <source excerpt>
  <one or two sentences of explanation, naming the actual and expected>
  fix: <mechanical fix, if there is one>
  affects: <symbols whose checks this blocks>
```

- **Stable codes**, never reused. Ranges: `E00xx` parse and projection, `E10xx` names, `E20xx` types, `E30xx` effects, `E40xx` contracts, examples and holes, `E50xx` store and transactions, `W` prefix for warnings.
- **Root cause first.** Diagnostics caused by an earlier one (for example, a failed type that makes every caller fail) are grouped under it as `caused by above: N more`, not listed separately. Phase 1a (D-46): errors with the same message are printed in full once, followed by `same error: N more` and one line each, `at <symbol>, line N: <source line>`.
- **Fixes** are either replacement text for the symbol or a command. The agent applies them explicitly; the store never auto-applies.
- `--json` gives the same fields.

## 7. Latency budget

On a store the size of the pilot, run on a developer laptop:

| Operation | Budget |
| --- | --- |
| `inspect`, `view`, `find`, `session start` | 200 ms |
| `submit` of one symbol, including the fast check | 1 s |
| `commit` | 2 s |

## 8. Phase 1a note

Over Gleam, the fast check calls `gleam check` on a generated project materialized from the store. If that misses the 1 s budget, the budget is recorded as missed rather than relaxed. It is a Phase 1b concern, since Solace has its own checker.

## Tests (acceptance)

- A1. Every command in §2 has a golden-output test, in text and `--json`.
- A2. A commit whose base revisions changed is rejected with `E5101`, and nothing is published.
- A3. A contract or signature change to a used symbol causes `E5102` on the other change's commit.
- A4. A work record opened in one session appears in the next session's `session start`.
- A5. A single type error in a type used by 5 functions yields one root diagnostic with `caused by above: 5 more`, not 6 separate ones.
- A6. The latency budgets in §7 are met on the pilot, measured in CI.
- A8. In a root with projects `a` and `b`, an agent working in `a` reads `b` with `view`, `find` and `read … --project b` while an agent works in `b`; `submit --project b` is refused with `E6007`, and `b`'s source is unchanged (`tests/workspace_acceptance.py`). A task covering `a` and `b` changes and tests `b` with `--project b` (Spec 15 W4).
- A7. Changing a signature records the new revision and lists its callers as broken dependents; `commit` is refused with `E5103` until they are fixed.
