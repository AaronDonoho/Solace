# Decisions

Settled questions: don't re-ask them. Each record keeps what was decided, and why when that isn't obvious. Every record is decided unless marked **pending**; a pending record gives its options instead of an answer. Details live in the specs, PROPOSAL.md and git history.

## D-1 Store and protocol before the language
Prototype the store and agent protocol (§4, §6) over Gleam and benchmark it against plain Gleam files. Build the Solace language only after that shows a measurable win. Why: the language, checker and backend are the most expensive and least novel part, and Gleam already has most of §11.

## D-2 BEAM target: Erlang source
Emit Erlang source, compiled by `erlc` (Core Erlang if finer control is needed). Not Gleam (a second type checker in the way), not Elixir (macros), not bytecode (unnecessary).

## D-3 Toolchain in Rust
The compiler, store and checker are Rust from Phase 1b. May be reopened before Phase 1b starts. Why: fast checks, one binary (builds still need OTP), solver bindings, and the Gleam compiler as a reference.

## D-4 Submits that break dependents
Accept a submit if the symbol itself checks, and track broken dependents in the open change; `commit` is refused while any remain. Why: otherwise a signature change can't be made one step at a time.

## D-5 Holes, propagation, fallback
Holes are `?name "question"`, propagation is `try expr`, and fallback is `expr else value`, on `Option` and `Result` only. Why: `?` stays reserved for holes; no overloading or truthiness (§11.13).

## D-6 Spec review choices
Contracts are checked at compile time where decidable, otherwise at run time, from Phase 1. An unused effect is a warning and is compiled out. Local variable names are excluded from the revision hash (bindings are hashed by position). State machines stay narrowed in Phase 1 (no data on states, no guards); revisit in Phase 1b.

## D-7 Benchmark sizes
Two sizes: the frozen small pilot (~2.5k tokens, 10 tasks) and `pilot-large` (~30k tokens of source, bytes ÷ 4). Tasks are written before the code. Why: at 2.5k, fixed per-call overhead hides any saving.

## D-8 The Solace arm uses the protocol only
No file tools on source; reading and editing go through `solace`. Why: the claim is "instead of files", and a stuck agent shows a gap in the protocol.

## D-9 Agents reach Solace through MCP
Agents get one MCP tool, `solace` (`command`, optional `text`), served by `solace mcp`, and no built-in tools. The CLI is for humans and scripts. Why: through a shell, agents read files with auto-approved commands, and heredoc quoting broke Gleam.

## D-10 Orientation
Module views are outlines (`--bodies` for code), `inspect a b c --context` batches several names, and `session start` expands open work records with context. Orientation is measured in model turns before the first edit (Gate 1: median ≤ 3), with tokens before the first edit second; tokens remain the overall metric. Why: every turn re-sends the whole context, so turns drive orientation cost.

## D-11 Benchmark agent
`bench.py run` uses Codex `exec`, `gpt-5.6-terra`, medium effort, and every run needs Aaron's go-ahead. Each run gets a fresh `CODEX_HOME` holding only the sign-in, with unrelated Codex features off. Files arm: shell and `apply_patch` in a workspace-write sandbox. Solace arm: no shell, a read-only sandbox, and the `solace` MCP server outside it. Metrics come from Codex's per-call usage records. Arms compared with each other must use the same agent.

## D-12 The session index lists names only
`session start` lists symbol names per module (~1k tokens on pilot-large); signatures come from `inspect` or `view <module>`. Why: the index with signatures (~8k) was re-sent on every turn.

## D-13 Task packet; exact replacement
`session start --task` gives a task packet in place of the index: the name, signature and doc line of symbols ranked deterministically from the task's words (rarity × field weight, named symbols first, one hop out to callers, callees, covering tests and their fixtures), within a byte budget, never bodies. `submit <name> --replace` makes one exact `old` → `text` replacement in one symbol, then checks the whole symbol.

## D-14 A reply budget instead of truncation or paging
A reply naming several symbols prints each signature once (`(above)`). Replies have a byte budget per agent, just below that agent's output cut (`solace mcp --budget`: 32,000 for Codex, 80,000 for Claude Code), shared by the commands in one call. Over budget, `inspect --context` cuts callers to a count, then tests to names, then Uses to names, never the symbols asked for, and lists what it left out with the command that shows it; `view` falls back to outlines. Why: truncated output gets requested again, and each page costs a turn.

## D-15 Build through the protocol
Agents build and maintain code through MCP alone (Gate 1). In order: a benchmark task that builds from scratch; creating large modules safely (drafts that survive rejection, diagnostics with lines); read-only dependency signatures from `build/packages/` through `inspect` and `find`. Deferred: package management, bundling, run and smoke commands, runtime errors mapped to symbols, `sync`.

## D-16 Lockdown, agent tooling and the human interface
1. Benchmarking is paused; gates aren't judged until it resumes.
2. Priorities: lockdown (agents change code only through Solace), agent tooling, and the human interface (the contract view and a channel to agents).
3. Gleam is a target, not necessarily the final language; prefer language-neutral forms.
4. A contract is the system's behavior and footprint: side effects, APIs, infrastructure and the logic that drives them. Humans see it from the top down and can run test data through it.
5. Humans reach agents asynchronously through a task board; any task can be paused; an agent's question blocks its task.
6. The board and Solace talk only through a minimal API that Solace defines; Solace ships a default board.
7. The interface is a web app.
8. Agents have no shell (D-8 kept): only the Solace tool and their own web access. Source changes only through the symbol protocol, other files through Solace's file commands (categories: D-52), and Solace runs a limited set of commands for them.

## D-17 Workflow and undo
The workflow (columns, allowed actions, required stamps) comes from `solace.toml`, with a default shipped. Task data lives in `.solace/board/`, out of agents' reach. Solace isn't tied to git: work lands in the project, and Solace records what each task changed and the state before it. `revert` restores that state, and is refused if a later task changed the same things. These records are invisible to agents and dropped when the task reaches a final column. `approve` is a stamp; a column can require one (`complete` requires `approved`).

## D-18 Serial work for now
One work run per project at a time (per scope since D-61). Kept open: isolating each task in the store (its own open change and compile directory, merged symbol by symbol on approve with Spec 3's conflict checks). Spec 15 states what revert and approve guarantee apart from how, so that change needs no API change.

## D-19 Agents wait; gates
An agent that asks a question waits for the answer, keeping its context; `new_agent` starts a fresh one instead, and `pause` holds it. Any column can require a stamp; the default gate is needs review, and `approve` only stamps. A task leaving a gated column any way but forward loses its stamp. If an agent can't keep waiting, a new one starts when the task continues. Claude's other defaults in Specs 14 and 15 are accepted.

## D-20 Guardrails and the human interface first; Solace ships
The primary goals are guardrails that keep agents working through Solace, and the human interface; tokens and speed are secondary. Order: lockdown, then the board, then the contract view. Solace is a product that wraps a user's project; `bench/` and the pilots don't ship.

## D-21 Session identity and launch configuration
Each MCP process is a session; the CLI uses `SOLACE_SESSION` (default `cli`; board runs use the run id), with one hash per file per session. `agent-config` writes isolated agent settings plus `launch.json` (arguments and environment). The CLI's exact replacement takes `--old <text>`.

## D-22 System settings, pausing, ending a review
Machine-wide settings (`[[models]]`, `task_timeout`, default 3600 s) live in `~/.config/solace/config.toml` or `$SOLACE_CONFIG`. A paused task stays in its column (`paused_from`); there's no Paused column. A review ends with `task note`, which is its whole result; `task done` is refused in reviews.

## D-23 Models and endpoints
Models run through external agent programs (the Codex CLI, Claude Code) with their own sign-in; Solace stores no credentials. Presets ship in the binary, and the system configuration adds, replaces (by `id`) or hides them. Custom endpoints: OpenAI-compatible through Codex, Anthropic-compatible through Claude Code, with the key read from an environment variable at launch. The model is chosen when a task is assigned. A model missing its program, sign-in or key is shown as unavailable and can't be assigned. A task whose agent fails or asks is marked as needing attention until a human acts.

## D-24 Following an agent's work
The board shows when an agent is working. A human can follow a run live or afterwards (messages, thinking, tool calls with input, output and status), read-only. Notes, messages and attention text are rendered as markdown; a failed run's note gives the agent's last message.

## D-25 Run kinds replace task types
Tasks have no type. Investigate runs (planning) explore, ask and report, changing nothing; work runs (assigned) are the only ones that write; review runs assess the changes. Planning allows `complete` for a task that only needs an answer. The report goes into the work run's brief. An investigate run that asks waits, and `investigate` continues it.

## D-26 Unwatched files are writable; create writes settings
Unwatched files can be read and written through Solace (unwatched means not searched, not read-only), under the size limit, stale-write checks, validators and task undo; replacing non-UTF-8 text is refused. `.solace/` stays out of reach and `solace.toml` human-owned. `create` (once `import`) writes a starter `solace.toml` when there is none, can be rerun safely, and grants no commands by default.

## D-27 Agent memory and skills
Memories are files in a directory named in `solace.toml`, read and written with the ordinary file commands. The brief lists them (name and first line), not their contents, and agents write one when a human asks. Skills work the same way from a human-owned directory. Any run may write memory, even a read-only one, and memory edits aren't task changes. Review and investigation reports are written by Solace to `.solace/board/`, not to memory. Later: extra MCP servers through a logged passthrough.

## D-28 Unlisted text files are watched
A file no rule names is watched when it's text within the size limit, and unwatched when it's binary or too large. Humans see each file's category and what it allows.

## D-29 Humans can delete notes
Any note, after confirmation. Deletion is permanent and unrecorded; notes are addressed by position.

## D-30 Answering continues the waiting agent
Submitting the last pending answer continues the same agent through the workflow's continuation (work and investigation runs). A paused run keeps the answer until Resume. Answering never starts a new agent, replaces a run, approves or reverts. Without a continuation, the answer is kept and the task's attention says what to do.

## D-31 Reassign from review
`reassign` from needs review keeps the code and before-states, removes approval, and starts a work run with the notes, review feedback and optional instructions, on the same model unless another is chosen. Refused while a review is running. `start_over` still reverts.

## D-32 Confirm cancel and complete
The board confirms both, naming the task: cancel stops agents and reverts; complete keeps the changes, closes the task and drops its undo records. Dismissing sends nothing and keeps the form.

## D-33 Conversations follow the author
Investigation, work and review fixes resume the author's conversation when compatible (native runner, same agent, model, endpoint and effort); each review starts fresh. Otherwise (changed configuration, `new_agent` or start over, a custom runner, missing state, a forced shutdown) the agent gets a handoff brief. A failed session load retries once, fresh, before any tool runs; quota and work failures aren't retried. Claude with macOS keychain-only sign-in gets fresh, non-persistent conversations. A finishing run gets up to 30 s to exit, follow-ups wait for it, and only an exited process's state is reused. Author state lasts until the task completes or is cancelled or replaced; reviewer state is deleted. A resumed brief carries only what changed: entities, new notes and answers, drafts, and the last check, test and build results.

## D-34 Rejected submits become drafts
A rejected submit's text is kept with its command under an id: `view --draft`, `submit --draft <id> [--replace]`, `drafts`. A successful retry removes it, finishing the task clears its drafts, and drafts aren't searched. A fingerprint stops stale retries (narrowed by D-46). A rejected new module leaves no file behind. Successful submits summarize warnings by symbol; `check --warnings` gives them in full.

## D-35 Contract view: facts, runs, review, tests
1. Facts come from the store (symbols, references, known effectful functions, every `@external`), marked as derived, never as guarantees. The graph format is language-neutral.
2. Runs aren't traced: a run shows data in, data out and effects. Why something happened is a question for the agent.
3. Review is the view filtered to a task, linked from the board with a one-line summary.
4. Humans run any public function ad hoc, with inputs from forms built from the types; results show as fields, state as before and after, files in the browser's viewers. A run can become a task carrying the input, the output and what it should have been. There are no human-owned tests.
5. Agents' tests are shown as data in → data out, one line per `assert`, with source on request.
6. Runs are remembered: review replays those that reach the task's changes, before and after, and shows the differences, without pass or fail.
7. Before a run the view lists the derived effects, and effects that leave the scratch copy need confirmation. Public functions nothing calls count as entry points.

## D-36 Specify before implementing
1. Every submit must type-check. A task is accepted only with no failing test and no `todo`.
2. Typed holes are Gleam's `todo`; a test that reaches one is unfinished, not wrong.
3. Tests, with `todo` stubs, come before the implementation. Each test's result is recorded as unfinished, wrong or passing; a test covers the symbols it references.
4. Changing an existing test's expected value while its subject has a body is flagged, across tasks.
5. A regression test must fail on the old code with the ticket's wrong value; one that passes on the old code is flagged.
6. Edits to tests or public signatures are flagged. Uncovered symbols need characterization tests first, passing on the old code.
7. These are flags, not refusals. The agent that triggered a flag can't resolve it, and resolutions are recorded.
8. Flagged expected values are reviewed blind: the reviewer works out the value before seeing the change, and only a disagreement goes to the human.
9. Solace ships an arithmetic skill: expected values are worked out from the stated inputs, never by running the code under test.
10. The benchmark gets a task whose existing expected value is genuinely wrong.
Why: tests written first can't be fitted to the code, and point 4 protects that without a second agent.

## D-37 Board layout, effort, note images
New tasks sit behind a button that keeps the draft; editing is on the Details tab. Task details are tabs (Details, Notes, Questions, Runs, Changes, and Scope in a workspace), newest first. Effort is chosen separately from the model; the run records it, and a changed effort starts a fresh conversation. A successful run marks the task finished ("Agent finished", "Report ready", "Review ready") until the next action. Notes take PNG, JPEG, GIF or WebP images (5 MB, content-checked, no SVG), stored in `.solace/board/` and attached to the agent's prompt as `[image N]`. The markdown editor is built in.

## D-38 Agents propose settings changes
An agent that is asked to change settings makes the change itself: in a work run, `write solace.toml` (or `AGENTS.md`, D-45) is validated (`E6020` if invalid) and becomes an Approve or Reject question carrying the old and new text; the run waits, and the human approves on the board. Don't hand the edit back to the human or stop at a plan, and don't ask in prose ("May I add…?") or with `task ask`: run `write solace.toml` with the exact new text, because only that call produces the diff the human approves. The file is the task's own project's `solace.toml` (D-57), never the root's `solace.toml`, which only lists projects. Approve writes it only if the file hasn't changed since. Refused in read-only runs (report the exact edit instead) and outside a run.

## D-39 Task diff and draft status
`task diff [name...]`, in any task run, shows each entity the task changed as a line diff against its state before the task. `drafts` marks each draft `current` or `stale`. A review brief starts with `task diff`.

## D-40 Many symbols per submit
The symbol is the unit of identity, not a limit per call. `submit --new <module>` adds every symbol in the text and creates a missing module whole; `submit <module>` and multi-block submits create modules too. A name defined twice is refused (`E0103`). Why: every call re-sends the whole context.

## D-41 `probe`
`probe` runs throwaway code from `text` (statements, or a module with `main`), with project imports added, as a temporary module that is always removed. It works in read-only runs and saves nothing.

## D-42 A workflow with examples
The reference opens with a numbered workflow and examples: read with one `inspect --context`; `submit`, then `test`, in one call; fix only what is reported; a passing `test` means done. Agents don't see host commands, and see task commands only in board runs; `help` has the full reference. The tool description says several commands go in one call.

## D-43 Probes from tests, a call tracer, self-referential asserts
`probe --from <fn>` starts from a zero-argument function's imports and body (`E1005` otherwise). `probe --trace <fn>...` shows each call's arguments and result, nested (values cut at 300 characters, 60 calls; `E1006` for constants, types and externals). A probe that is one expression is printed. A test asserting `a == b` with both sides read from the same local gets a note. Expected values still follow D-36.9.

## D-44 Reach and impact
One engine (`reach.rs`):
1. `test` runs the tests whose reach changed since they passed, that failed or never ran, or that reach process or network effects or externals; `test <name>` runs those that reach the names; `test --all` runs all of them.
2. Selection uses reach; "checked by a test" keeps D-36.3's meaning.
3. Data flow follows changes through bindings, record fields, callers, field readers (certain or possible) and helpers' field writes.
4. A submit adds a note of at most 3 lines: uses reached, the nearest untested ones, where data leaves, suites reached.
5. `impact [name...]` lists every use, nearest first, with its tests; `refs <module>.<Type>.<field>` lists a field's readers.
6. Suites outside `gleam test` are `[commands]` with `checks`; Solace names a suite a change reaches but never runs it unasked.
7. Review counts the task's tests and untested uses.
8. Task packet entries show the string literals that match the task, and a reproduction line names the best test and trace targets.
Reach and data flow are derived facts, not guarantees.

## D-45 AGENTS.md at session start
`session start` (and `--task`) begins with the root's `AGENTS.md`, introduced as the maintainers' instructions and cut at a whole line past 8,000 bytes. It's protected like `solace.toml`: work runs can only propose changes to it (D-38). Why: runners load it differently, and instructions are a guardrail only if agents can't rewrite them.

## D-46 Leaner replies for ripple changes
1. Submit and delete replies report the broken set by difference: symbols fixed, one line per symbol still broken, new errors in full once per message. `check` stays complete.
2. `inspect --context` on a type separates "Constructed by" (with source lines) from other users; callers show their call line, and `refs` does the same. Commands in one call share what's been printed (`(above)`).
3. `submit <name> --replace --all` replaces every occurrence, and `E0105` lists the occurrences. A draft's base is only the symbols it replaces and their module headers. Unchanged blocks are reported and not saved.
4. `session start` shows the short workflow; `help <command>` gives that command's paragraphs.
Deferred: tracking what was already sent in a conversation.

## D-47 Patches for submit
A plain `submit` takes an `apply_patch`-style patch (`*** Update: <symbol or module>` with `@@` hunks, `*** Add: <module>.<name>`), matched the way `apply_patch` matches and turned into per-symbol blocks, so every check works as before. Unmatched hunks, deletes, renames and patches that change nothing are refused (`E0108`). Why: whole-symbol blocks cost output, and Codex knows `apply_patch`.

## D-48 Tests at the reported level; certain uses first
The workflow asks for a test at the level the task reports the problem (a checkout ticket gets a checkout test) and one for each behavior the change shifts. `impact` and the submit note list certain uses first, untested first; possible uses become a count with the `refs` command that lists them.

## D-49 Solace ships the bench's Codex setup
`agent-config codex` writes `tool_output_token_limit = 30000` and the hint on calling Solace from `exec` (as `developer_instructions`). Why: the bench should measure what ships.

## D-50 Phase 1a checkpoint met
Met on 2026-10-03 (benchmark runs 15–21). Phase 1b, the Solace language core, is next; D-3 may be reopened first, and D-6's state machines are revisited during it.

## D-51 The final column is `complete`
The default workflow's `done` column is `complete`, matching its action; tasks saved in `done` load as `complete` unless the workflow still declares `done`. Why: not to be confused with `task done`.

## D-52 File categories
Named for what Solace does with a file:
- **graph:** source held as symbols.
- **watched:** other text files within the size limit, including `gleam.toml` (which keeps its validator and can't be removed).
- **protected:** `solace.toml` and `AGENTS.md`.
- **unwatched:** `.git/`, build and dependency output, `.gitignore`d files, paths listed in `unwatched`, and binary or oversized files; writable, not searched.
- **internal:** `.solace/`; no agent access.

In `solace.toml`, `resources` is gone, `ignore` is `unwatched` and `resource_limit` is `watch_limit`; the old keys are invalid (`E6020`).

## D-53 Contract view drawing
No title block: the project and task are in the header, the revision key in the legend. Dragging pans and never selects or opens; opening a node moves the sheet only to bring it into view. Keys: arrows or WASD move, with Shift they pan, Ctrl+Up/Down zooms, Space expands, C collapses, Enter opens, and Shift+Space or Shift+C expand or collapse everything. A changed node has a plain border in its revision color and the selected node a thick one; `+N` and `−` tabs show hidden or expanded connections (none on a node without any). Connectors name their relation on hover. The side panel is resizable. With a task, Show source gives the line diff.

## D-54 Contract view side panel
Tabs: Review (with a task), Details and Files; Close folds them into the header's corner. Files are grouped by category, with one line explaining each; a file opens read-only, with line numbers, in place of the drawing. Overview is the zoom tools' home button.

## D-55 Contract view opens on what the project does
1. Without a task, it opens on an outline by area (a module, its doc comment as the summary). Each public function shows its summary, what it takes and gives in words, what it reads or changes, and its test coverage; a line opens the drawing at that function.
2. An entry taking a project type with at least three data-carrying constructors lists them as the requests it handles; such entries and `main` come first.
3. A project type that functions in at least three modules both take and return is state: a function that takes and returns it changes it, one that only takes it reads it. Both are derived effects.
4. Boxes show no code: name, summary, and what it takes and gives in words; the signature is in the details. Missing descriptions get one button that makes a task to write them.
5. Details lead with examples from the tests, then effects, connections (Calls, Called by, Effects, Tested by), signature and source.
6. With more than 12 entry points, the overview shows one box per area.
7. A label every item would share is shown once, as a heading.

## D-56 Board editors, approval mark, notices
The new task form has no Cancel (Close keeps the draft), and Title is a placeholder. Descriptions and notes share one markdown editor. An approved card shows ✓; Details no longer lists stamps. The review summary and contract view link are on the Changes tab. A run ending or a new question raises a dismissible notice naming the event and the task, only for news: `/events` replays, then sends `ready`, and the replay raises none. Only text a human may copy can be selected, and only editor text can be dragged.

## D-57 One root, several projects, one board
1. Each project keeps its own store, `solace.toml`, `AGENTS.md`, lockdown and tasks. A root is one folder whose `solace.toml` lists its projects (`projects = [...]`, folders below it). A project with no root above it is its own root.
2. Nothing leaves the root: no `.` or `..`, no symlinks, no root inside another root. `serve` and `create` use the root that holds their folder.
3. `solace create` (once `import`) finds the Gleam projects in a root and offers each one; the board's Projects panel adds, removes and creates projects.
4. One collapsible lane per project, keyed by folder. The root's project answers at `/`, the others at `/p/<folder>`.
5. One `/events` stream merges every project's events, each naming its project.
6. Reads take no lock; agents read other projects with `--project`.
7. A folder below a project that has its own store is internal to that project.
8. One server per project (`.solace/serve.lock`). A project can't be removed while its agent works, and the root's own project can't be removed.
Why: separate setups keep every guardrail per project, and what's shared is a thin layer.

## D-58 Changes made outside Solace become reviewed work
1. Solace never overwrites a source file it didn't write last (`.solace/written`); a changed file is read into the store as a new revision. Solace checks at each command and, while the board runs, about once a second.
2. Changes made during a work run belong to that task; changes a command makes (`format`, declared commands) belong to that command.
3. Other changes go to an **Unexpected Changes** task in needs review, with the diff and compile result. Completing it keeps them, cancelling restores Solace's code, and while it's open no work run starts or continues (`E7012`).
4. Code that doesn't compile is still read in; Solace never reverts on its own.
5. Only graph files raise this.
6. Pulls and branch switches stay quiet when the code matches `.solace/approved`, the hashes as last approved, committed with the project.
Why: files on disk stay authoritative, so other tools keep working and a Solace bug never traps code, while outside work still waits for review.

## D-59 Migrating a project without touching it
Migration never touches the original's files. Solace copies the whole folder to `../solace-migration/`, sets Solace up in the copy, regenerates the source from the store, and reports on Coverage, Reconstruction (every difference explained; byte-for-byte isn't required), Behavior (build, tests and smoke checks against a baseline), Preservation (against an inventory) and Recovery (works without Solace). Each check is verified, failed or not checked, and later changes invalidate it. One migration runs at a time. Adoption needs the checks to pass and the human's approval, and is refused if the original changed (index-only git rewrites don't count; staging does). It swaps the folders so the project keeps its path, rechecks there, and swaps back on failure; abandon and undo are available. Commands run sandboxed with the original read-only (bubblewrap) where available, after the path check; symlinks into the original are pointed into the copy. Errors the original already has don't block adoption, and the guide says they carry over. `solace create` only makes new projects, after confirmation. Migration text is written for a user with no context: buttons say what they do (Make the copy, Continue, Run the checks again, Switch to the copy, Delete the copy, Switch back).

## D-60 The root is a project of plain files
A root with a store is a project, with or without `gleam.toml`. Without one it has no symbols, but has its own settings, `AGENTS.md`, lockdown, lane and tasks, and Gleam commands give `E6013`. Every root is set up this way by default (on `serve`, or when a project is listed); an unmanaged Gleam root waits for migration. The root and its subprojects read each other with `--project`.

## D-61 Tasks across projects; one workflow per board
1. A task's scope is the projects it may change, chosen at creation (by default the lane it starts from), at least one. Its card shows in each of their lanes, in one column.
2. The human edits the scope while no agent works; a project the task already changed, and the last project, can't be removed.
3. One workflow per board, from the root's `[board]`.
4. A task is stored in its home project, where it was created; removing the home moves it to the first remaining project.
5. A work run claims every project in its scope when it starts (`E7005` if another task's work run holds one, `E7012` with Unexpected Changes open).
6. The agent changes another project in its scope with `--project <folder>` on any command, settings proposals included (named by their path from the root). Anything outside the scope is refused (`E6007`), and the agent asks the human.
7. Changes are recorded in each project's store under the task; review, diff, undo and completion span the whole scope.

## D-62 Shell project and Lustre app contract
A shell project hosts the board, migration screen and contract view as Lustre components: `/` is the board, `/migrate` migration, and `/view` and `/p/<dir>/view` the contract view; `/migration` stays the JSON API. Each app exposes `register() -> Result(Nil, lustre.Error)`, declares its tag and the attributes it reads (`base` for the project prefix), and owns its CSS.

## Migration component verification commands
The migration component's test commands (two Bun test files and an optional browser fixture) are declared in `solace-cli/migration/solace.toml`, not at the root.

## D-63 A local model's context window comes from its endpoint
For a Claude run on a model with an `endpoint`, Solace reads `meta.n_ctx` from the endpoint's `GET /v1/models` and passes it as `CLAUDE_CODE_MAX_CONTEXT_TOKENS`, so Claude Code compacts before the server refuses the prompt. There's no config field. Only plain-`http` servers that report `meta.n_ctx` (llama.cpp, Strata). Why: the server knows its window, and a copy in config would drift.

## D-64 `grep` searches the project's files
`find` stays the search of source symbols. `grep <text>` searches watched files and the protected settings, case-insensitively, in every project (plain-files ones included) and through `--project`; unwatched files stay out (D-52). Why: agents couldn't search documents such as this file; the watched set is small, so searching it doesn't cost Solace much time.

## D-65 Signatures carry the whole doc comment
Outlines, `inspect --context`, `refs` and task packets show a symbol's whole `///` comment above its declaration, not just its first line; a doc-only edit doesn't count as a signature change. Why: Gleam docs needn't open with a summary line (redline's give one rule per line), so the first line was often half a rule; the token reason for cutting went with the signatures in the per-turn index (D-12).

## D-66 Agent task responses
Task briefs guide agents to use Markdown with relevant sections and short headings, and to explain why they ask a question. Reviews begin with **Assessment: Looks good** or **Assessment: Needs work**, supported by findings and verification. The human chose a label in the review message, not a separate stored field or board badge.

## D-67 Reviews may run marked commands
In a review, `run <name>` works only for a `[commands]` entry marked `allowed_in_reviews = true` in that project's `solace.toml`; other commands, and `run` in investigations, stay refused. Why: reviewers need `bun test` or a bundle step to verify, and only a human can mark a command, since agents can't change `solace.toml` unapproved.

## D-68 `.gitignore` is human-owned; dependencies are never watched
Every `.gitignore` in a project is protected like `solace.toml`: in a work run, `write` proposes the change for approval. `node_modules` folders are unwatched by default, as `build` is. Why: what a `.gitignore` ignores is unwatched and so unreviewed, and an agent could hide its own work that way; the dependency rule means installing packages is never recorded as the task's changes, whatever the order of steps.

## D-69 Follow-up reviews use completed review checkpoints
Keep ordinary `task diff` cumulative; `task diff --since-review` compares against the last completed review's start state, naming its run and falling back explicitly when the checkpoint is absent or scope changed. Only `task note` makes a review checkpoint eligible; record changes during review without treating finish-time content as reviewed. Follow-up briefs include prior feedback, subsequent work and human notes, changed entities and verification freshness, while reviewer conversations stay fresh. Unchanged code and context may carry forward the prior assessment, never upgrading Needs work merely because nothing changed; human requests for a full review take precedence. Checkpoints are internal and cleared on revert/start-over and finalization.

## D-70 SQLite stays bundled
`rusqlite` keeps the `bundled` feature; the install path doesn't detect, link or install a system SQLite. Why: on 2026-10-05 a cold release build took 47.8 s, and 39.6 s of that was SQLite's C compile. But one self-contained executable with minimal install steps matters more than first-build time, and warm builds don't pay that cost.

## D-71 Acceptance tests run with an independent Solace context
The human-approved root `rust-acceptance` command unsets `SOLACE_TASK`, `SOLACE_RUN`, `SOLACE_SESSION`, `SOLACE_CONFIG` and `SOLACE_BIN` for its child process and remains allowed in reviews. Why: acceptance fixtures create independent temporary projects and must run `migrate` and `serve`; inheriting the invoking agent's task context wrongly refuses those commands with E7010. This specific command is authorized to run test code with normal host permissions, not filesystem isolation. The tests do not strip context themselves, and other commands retain their existing restrictions.

## D-72 Stop and Discuss
Two built-in actions join pause and resume (Aaron, 2026-10-05). `stop` interrupts any open run (work, investigate, review or discussion), after the board confirms it: the run ends `stopped`, its agent is killed and its session thrown away, so a later run starts a fresh agent with the notes. Changes are kept and the task stays in its column. `discuss` is a board button that sends the composer's text to start a read-only `discuss` run with the selected agent, between runs only; the agent works from high-level context and replies with `task note`, which waits for the human's next note. `end_discussion` (or stop) ends it, and Solace replaces the discussion's notes with one verbatim transcript note, each turn labeled with its author. The label stands on its own line, so a turn's Markdown renders. A discussion's brief keeps every note whole (no command reads a cut note's full text), lists the decision titles, and leaves out the verification record (Aaron, after a trial discussion).

## D-73 Board look
The board takes the contract view's drafting-sheet look (Aaron, 2026-10-05): ink on paper, square controls, tabs colored by kind, one set of color tokens for both screens, light and dark. The board's background is plain, with no grid lines, and swimlanes touch, separated only by their borders. Links to other pages (Guide, Contract view) are bordered buttons with an arrow. The dark set is a deep, saturated indigo (page #1f2136, from the #292b37 starting point) with dark indigo borders (`--edge`) and saturated kind colors on card tabs; it lives in `board/index.html`, chosen by `data-theme` on the page (the saved choice, else the system's), and a header button flips it and saves the choice. The font is IBM Plex Sans, with IBM Plex Mono for code, served from `board/priv/fonts/` (SIL Open Font License); Solace Drawing is gone.

## D-74 Removing a project can delete its folder
Extends D-57 §8 (Aaron, 2026-10-06). The board's Remove asks whether to also delete the project's folder and files. No unlists it as before (off the root's `solace.toml`; its `.solace` stays, and Add lists it again). Yes shows a second warning that this can't be undone and asks for the project's name; only a matching name enables **Delete permanently**. The server repeats the checks: `DELETE /projects/{dir}` with `{delete: true, name}` unlists the project, then deletes its folder, and refuses with `E7014` unless `name` is the folder or package name, the folder is reached without symlinks and holds no other listed project; `.`, `E7003` and `E7013` still refuse. Why: deleting is the one step with no undo, so it takes two confirmations and a typed name; unlisting already has its undo.

## D-75 Model and effort changes keep the session
Amends D-33 and D-37 (Aaron, 2026-10-06). Changing a model's name or its effort keeps the author's conversation: compatibility compares provider, endpoint and runner, not model or effort. Why: model and effort are launch flags (`-m`/`--model`, `model_reasoning_effort`/`--effort`), not conversation state, so a run that switches either resumes the same session. A changed provider, endpoint or custom runner, `new_work`, revert and stop still start fresh.

## D-76 A plain-text discussion reply is a turn
Amends D-72 (Aaron, 2026-10-07). A discussion agent that replies in plain text instead of `task note` and exits cleanly no longer fails the run: its last message becomes its note, the run waits, and the human's next note starts a fresh agent briefed with every note. An exit with an error, or without a message, still fails. Why: the reply was fine and only the tool call was missing, yet the board flagged it as a crash.

## D-77 A task server owns one task store
Amends D-17 and D-61 point 4; builds on D-18 (Aaron, 2026-10-07).
1. A task server (`solace-cli/src/tasks.rs`, a module inside `serve`, API `/tasks/...`) owns the task store, the single source of truth for tasks, at `<root>/.solace/tasks/`. It is the only writer of task files and applies operations one at a time, in order, each file written by write-then-rename. Clients don't depend on how it is built, so it could later move to Gleam/BEAM. The board is a client; the archive and Strategy pages will be too.
2. Tasks have no home project and never move. Run logs, conversations and review checkpoints stay in each project's `.solace/board/<tid>/`. Existing tasks move into the root store once, on `serve`, under the serve lock.
3. Agents send messages, not writes (`task note`, `task done`, `ask`, verification, settings proposals). A message is in `<root>/.solace/tasks/inbox/` (written, synced, renamed) before it is acknowledged, and is applied again after a restart. With no server running, messages wait until `serve` starts.
4. What a task changed, and how to undo it, lives only in each project's `store.db`, keyed by task id (`board_touches` gains `run` and `name`; the task file drops `changes`). Symbol changes and their undo rows commit in one SQLite savepoint. A file's before-state is written ahead (atomically) in `.solace/pending/` before the file changes, and recovery at the next command or `serve` finishes or drops it, so a crash leaves a change with its undo or neither. Stores use WAL with a busy timeout, so the task server reads changes without the project lock. Undo goes by task id alone.
5. Task-only actions take no project lock: create, edit, move, note, stop, cancelling a task that never had changes, scope and answers. `agent_guard` reads run state from the task store. Actions that change code take the store lock of each project they touch: undo, moving into a final column when that undoes or settles the task's changes, approving a settings proposal, accepting outside changes, adding or removing a project. A scope change stays lock-free: a project with the task's changes can't leave its scope (`E7013`), and a project that leaves takes its lock with it; which task changed what is told apart later (Aaron, 2026-10-07).
6. Create is idempotent: `POST /tasks` carries a client `request` id, and the same id returns the same task.
7. Addition to point 5 (Aaron, 2026-10-07): `task done` settles nothing in the agent. The server settles (approves, drops undo records) after it applies the move into a final column, as a step of its own off the writer thread that takes the scope locks like a human action, re-reads the task and settles only if it is still final. It can run again, and `serve` runs it for final tasks that still have undo records. Undo and settle first adopt any pending record of the task under the locks, and a pending record makes cancel take them.
Why: every board write took the project lock, so a long agent command blocked task-only actions, and repeated clicks queued up and made duplicate tasks (four on 2026-10-06).

## D-78 A new board connection starts from a snapshot
Amends Spec 15 `/events`; builds on D-77 (Aaron, 2026-10-07).
1. A `/events` connection with no valid `Last-Event-ID` first reads the task log's position and each project's workflow-event position, then each project's workflow and tasks from the files, and sends `{workflow}` and `{tasks: [...]}` (the list `GET /tasks` returns) for each project, then `ready`. Reading the positions first means a change made in between arrives again as an event. A project added to the list later gets the same pair after `{projects}`. A reconnect with a valid id still gets only what it missed.
2. The board takes its tasks from the stream, not from `GET /tasks`; it holds a message for a project the list has not named yet.
3. The task server writes the file first. A failed log append after the rename is reported on stderr and doesn't fail the change, so a client doesn't retry it and repeat a note or action; the next connection shows the file.
Why: the board showed the last logged copy of each task, so a file written without an event (a crash between rename and log, or `adopt_tasks`) was stale or missing, and every page load replayed the whole log. Compacting and pruning the log, and one SQLite connection per stream, are left for later.

## D-79 Task messages go in `text`, never in `command`
Amends Spec 15 §6 (Aaron, 2026-10-07).
1. Every agent prompt says that `text` and `options` are tool arguments beside `command`: for `task note|done|ask`, `command` is just the command and the whole message goes in `text`, never as a `--text` flag.
2. `task note|done|ask` whose first word after the subcommand starts with `--` is refused (`E7015`) and sends nothing; the error says how to resend. The refused call returns at once, so the agent stays in its turn and retries.
Why: a discussion agent sent `task note --text "### …"`; the command was cut at its first line and the human got `--text "### What I'm suggesting` as the reply.

## D-80 The desktop app runs `solace serve` as a sidecar
Builds on D-57 point 8 and D-77 point 1 (Aaron, 2026-10-07).
1. The Tauri app (`solace-app/`, Linux and macOS first) starts the bundled `solace serve --port N` as a child process with the chosen folder as its working directory. It does not call `board::serve` on a thread. The app depends only on a process contract: `serve --port N`, the `Solace listening on` line or the lock file, and HTTP/SSE on 127.0.0.1. The server could later move to Gleam/BEAM without touching the app.
2. A server writes `{"port":N,"pid":P}` into every `.solace/serve.lock` it holds. A port read from a lock file is trusted only while that lock is held. An app that finds a held lock with a port attaches to that server instead of failing.
3. `SOLACE_BIN`, when set in the server's environment, is the CLI that agents and MCP call, in place of the server's own executable.
4. The server refuses (403, `E7016`) any request whose `Host` is not `127.0.0.1:<port>` or `localhost:<port>`, or that sends an `Origin` other than `http://127.0.0.1:<port>` or `http://localhost:<port>`. Requests without those headers (curl, tests) pass. This stops other sites' no-preflight POSTs and DNS rebinding, for plain `solace serve` as well.
Why: browser mode makes any website a possible client of the local server, and the server and agent binaries can differ once the server is not the CLI. The app, start screen, tray and installer follow in later steps.

## D-82 Blocked is not a column; a held or stopped agent is attention
Amends Spec 15 §1, §2, §3 and §6 (Aaron, 2026-10-08).
1. The default workflow has no `blocked` column. `asked` and `failed` have no target: the task stays in its column (normally `assigned`) and gets `attention`. Answering the last question continues the agent directly (D-88). A workflow may still name targets for `asked` and `failed`; only `finished` must have one.
2. Attention from a held question (work or investigation), a settings proposal, or a failed or restarted work run carries `hold: true`. It stays through other actions and clears when the run continues (an answer, `investigate`), is restarted (`new_agent`, `start_over`, `reassign`), is stopped, or the task reaches a final column. Any other attention clears on the next action, as before.
3. A task saved in `blocked`, under a workflow without that column, loads in `assigned` with held attention (its own, or "the agent needed an answer or stopped").
4. `blocked` is reserved for a task that depends on another task. Nothing sets it yet and the task has no field for it.
Why: every known case of "blocked" is an agent needing a human, which attention already says; a column for it moved the card away from the work it belongs to.

## D-83 Tests are short and bounded
(Aaron, 2026-10-08.)
1. A test is kept short. A full suite that takes long to run is a defect to fix, not to wait out: a slow test is shortened, or split, when it is found.
2. Every wait in a test (a loop, a join, a poll for a file or a reply) has a deadline and fails with a message when it passes. No test can hang the run.
3. Tests don't share process-wide state: a global such as the task server acts only for the root it serves, so tests running in parallel can't see each other's.
Why: a `tasks::` run timed out because one test's global server made other tests wait forever; the whole run, not just that test, was lost.

## D-84 Agents move files and folders
Amends Spec 14 §3 (Aaron, 2026-10-08).
1. `move <from> <to> [--to-project <folder>]` moves a watched or unwatched file, or a folder, within a project or into another project the task's scope covers (`--project` picks the source's, as for any command). Outside a board run, a move stays in one project.
2. Graph files (a projection of the store), protected files, internal files, `gleam.toml` and symlinks never move, named directly or inside a folder.
3. A move never replaces anything: when something exists at the destination, it is refused (`E6002`) and nothing changes.
4. Atomic: everything is checked before anything changes, and the move is one rename that refuses to replace (`renameat2` with `RENAME_NOREPLACE`). Undo records go first, so a crash after the rename still records the move. A move between file systems is refused, since it can't be one step.
5. Idempotent: repeating a move the session already made, with the source gone and the destination there, changes nothing and says so.
6. The task records each moved file as removed at the source and added at the destination, in each side's project; revert and cancel put it back.
Why: agents had to rewrite a file and remove the original to move it, which loses binary and unwatched files and is not atomic; the human asked for moves within the task's scope that never overwrite.

## D-85 The contract view is its own app
Amends D-62 (Aaron, 2026-10-08).
1. The contract view is its own Gleam project, `solace-cli/contracts` (package `contracts`, JavaScript target), like the migration page. Its component is `<solace-contracts>`, with the optional `base` attribute it had as `<solace-view>`.
2. The shell depends on it by path and registers it. The page is `/contracts`, and `/p/<dir>/contracts` for a workspace project; both render `<solace-contracts>` (with `base` for a project) below the shell's menu bar. `/view` is gone: the server serves the shell at `/contracts`, the board's "Contract view" links and a task's review link (`/contracts?task=<id>`) point there, and an unknown path such as `/view` shows the board. Amends Spec 16's `GET /view`.
3. The menu bar on every page has a **Contracts** link (`/contracts`), marked current on any contract page, including a project's. The menu is 56px high; the contract page fills the rest of the window through `--contracts-height`.
4. The board project holds only the board. The contract view's FFI (`layout.mjs`, its part of `api.mjs`) and the dagre dependency live in `contracts`; the shell keeps dagre because the FFI is copied into its build.
5. Saved browser state keeps its keys (`solace-view-positions:v2:…`, `solace-view-panel-width`), so node positions and the panel width survive the move.
Why: each page is its own app with its own build and tests, as the migration page already is; the tag names the app it belongs to.

## D-86 The shell finds text in the page
(Aaron, 2026-10-08.)
1. Ctrl+F (Cmd+F) opens a find bar on every page, also from a text box; the shell prevents the browser's own find. Enter and Shift+Enter step through matches with wrap-around and centre the match in each scroll area and the page where there is room, when it isn't already fully in view. A new query starts at the first match in view, else the first below it. Visually hidden text (1px `.sr-only` labels) doesn't count. Esc closes from anywhere in the bar (or the page when nothing has focus) and returns focus to the element inside the component that had it. A live re-search keeps the current match when its text is still there; opening or closing a `<details>` re-searches. The bar shows "n of m" or "No matches".
2. The search walks the document and every open shadow root, case-insensitively, skipping `style`/`script`/`template` text and text with no boxes on screen. A match lies within one text node; a phrase split across elements is not found. At most 1000 matches.
3. Matches are marked by boxes drawn over the text from `range.getClientRects()`, in a fixed layer with `pointer-events: none`, never by changing the DOM Lustre renders and never by changing `user-select`: cards stay unselectable and draggable. Boxes are clipped to every scroll area around the match, skipped where something else covers the text (a sticky lane header, a modal, a notice, the find bar), and redrawn on scroll (captured on the document and each shadow root, since scroll events don't cross shadow boundaries), resize and re-search. Not the CSS Custom Highlight API: WebKit doesn't paint it on `user-select: none` text (WebKit bug 278455).
4. The bar and the marks layer are plain DOM owned by `shell.mjs`, outside `#app`; the decisions (`find_opens`, `next_match`, `count_text`) live in `frontend.gleam` and are tested. Changes inside a component's shadow root re-run the search (debounced); the bar's and the marks' own changes don't.
Why: Tauri's webview has no find, and native find in WebKit misses shadow DOM, where every page lives.

## D-87 Features and milestones are root records kept by the task server
Amends Spec 15 §1 and §4 (Aaron, 2026-10-09).
1. A **milestone** is a planned release. A **feature** is a capability of the product. Both are records at the Solace root, kept by the task server, which is their only writer. They are not tasks (D-25) and have no column, runs, scope or workflow.
2. Links are optional. A task has at most one feature and at most one milestone. A feature has an optional *target* milestone. A task given a feature with no milestone of its own takes the feature's milestone; after that the two can differ. A feature appears in every milestone that holds one of its tasks.
3. Tasks gain `feature`, `milestone`, `dependsOn` (task ids), `closedTime` (when the task first reached a final column; never cleared) and `archivedTime` (when the task was archived; null otherwise). How a task is archived, and how the board hides it, is decided later.
4. `feature` and `milestone` can be changed on a task in a final column. Other edits there stay refused (`E7004`).
5. Deleting a feature or milestone clears every link to it in the same operation. It never deletes tasks.
6. `dependsOn` is recorded and validated (existing tasks, no self-reference, no cycles), but nothing enforces it yet. D-82 point 4 keeps `blocked` for its later use.
7. Their events go on the root's stream only, with no `project` field; `/p/{dir}/events` leaves them out.
8. Milestones and features share one list of states: `proposed | planned | active | complete | canceled`. A new record starts `proposed` unless its create names a state. Any state can change to any other. A `canceled` record keeps its links to tasks; only a delete clears them.
Why: the Feature/Milestone store and the Strategy page need a settled model to be built and reviewed against.

## D-88 Answering continues the agent; there is no Unblock
Amends D-30, D-82 and Spec 15 §2, §6 (Aaron, 2026-10-09).
1. Answering a waiting agent's last open question hands the answers straight to its run, in any column and any workflow. No workflow action is looked up, so none is guessed, moved, stamped or replaced. A paused task waits for `resume`.
2. The default workflow has no `unblock`. After a failed or cut-off work run, `assigned` offers `reassign`, which brings back the previous conversation when it is compatible (D-33) and otherwise starts a fresh agent with the notes. `new_agent` always starts fresh.
Why: `unblock` did nothing a human could see, yet answering secretly pressed it; removing it from a workflow silently stopped answers from reaching the agent.

## D-89 A question is alone in its call
Amends D-38 and Spec 15 §6 (Aaron, 2026-10-09).
1. In a run, `task ask`, and a `write` of `solace.toml`, `AGENTS.md` or a `.gitignore` (a settings proposal), must be the only command in their tool call. Otherwise the whole call is refused with `E7017` and nothing runs.
2. A settings `write` takes a `reason` argument; the question reads "Change <file>", a blank line, then the reason. The explanation belongs to the question, not to a note beside it. Agents are told so in their brief.
Why: agents posted a `task note` next to a question, because a settings question had no place for its why.

## D-90 Strategy is its own app at `/strategy`, with tabs
Like D-85 (Aaron, 2026-10-09).
1. The Strategy page is its own Gleam project, `solace-cli/strategy` (package `strategy`, JavaScript target). Its component is `<solace-strategy>`. The shell depends on it by path and registers it.
2. The page is `/strategy`; the server serves the shell there. There is no `/p/<dir>/strategy`: features and milestones are root records (D-87). The menu bar has a **Strategy** link, marked current on the page.
3. The page has tabs, one component per tab: **Vision**, **Features**, **Roadmap**, **Tasks**. The tab shown is kept in the URL as `?tab=<name>` (`/strategy?tab=tasks`), written with `replaceState`, so a reload returns to it; a missing or unknown value shows Vision.
4. "Archive" is the Tasks tab: there is no separate archive project, and `solace-cli/archive` is removed.
Why: each page is its own app with its own build and tests; planning grows into several views that belong on one page.

## D-91 The vision is one internal Markdown document kept by the task server
Amends Spec 15 §1 and §4 (Aaron, 2026-10-10).
1. The root has one vision: Markdown at `<root>/.solace/strategy/vision.md`. It has no store and no records. It is internal, so agents can't reach it (Spec 14 §1); the task server is its only writer, and writes it atomically.
2. `GET /vision` returns `{text, revision}`; `revision` is `sha256:` and the hex SHA-256 of the text, so a missing file reads as `""` and its hash. `PUT /vision {text, base}` writes only if `base` is the current revision; otherwise it is refused with `E7026` (409) and nothing is written, so two editors don't overwrite each other. Both answer at the root only.
3. The root's `/events` sends `{vision: {text, revision}}` after every write, and a new connection's replay sends it after `{features}`. `/p/{dir}/events` leaves it out.
4. Investigate, work and discuss briefs carry the vision, at most its first 1000 characters (cut at a line end when one is in reach, and marked as cut). Review briefs don't: a review judges the diff.
5. The Strategy page's Vision tab shows it as Markdown and edits it with the shared Markdown editor; a draft begun from an older revision says so and offers to reload.
Why: agents should work toward what the product is for, and the human writes that down once, where the page and every brief read it.

## D-92 The vision is at most 1000 characters
Amends D-91 and Spec 15 §1 and §4 (Aaron, 2026-10-10).
1. `PUT /vision` refuses text over 1000 characters with `E7027` (400) and writes nothing. Characters are Unicode code points, as Rust's `chars().count()`; the page counts the same way.
2. While editing, a line below the text box reads "973 / 1000 character limit". Past the limit it is an alert, and **Save** is disabled.
3. Briefs carry the whole vision. Only a vision saved before the limit, and longer than it, is still cut as D-91 point 4 says.
Why: agents should always read the whole vision, so it has to be short enough to go into every brief.

## D-93 Strategy runs: agents plan with the human on the Strategy page
Amends D-90 point 3 and Spec 15 §1, §4 and §6 (Aaron, 2026-10-10).
1. A **strategy run** is not a task's run. It is a root record at `<root>/.solace/strategy/runs/<id>.json`, written only by the task server: `{id, kind: "strategy", model, effort?, state: running|waiting|ended, started, ended?, outcome?, log, context, notes, questions}`. `context` is what it was started on: `{kind: "page"}` or `{kind: "task"|"feature"|"milestone", id}`. The human starts it from the Strategy page with a first message; a second open run on the same context is refused (`E7003`).
2. Its scope is everything on the Strategy page. It reads tasks, features, milestones and the vision by following their links (`strategy list`, `strategy show <kind> <id>`), and reads any project's code with `--project`. It changes no code: commands that change a project are refused (`E7010`), and it takes no project lock.
3. It is discuss-like (D-72, D-76). The agent replies with `strategy note`, which waits for the human's next message; `task done` and `task ask` are refused. The human ends it (outcome `planned`), and its notes become one transcript note, as `end_discussion` does.
4. **Every change needs the human's approval.** `strategy task|feature|milestone|vision` proposes creating or editing a record, or a new vision text. Each proposal is a question on the run, with options Approve and Reject, and the agent waits for the answer. Approve applies it as the page's own request would be applied, with the same refusals (`E7003`, `E7004`, `E7023`–`E7027`); a refused apply changes nothing, and the answer says why. Reject changes nothing.
5. Decisions are logged where they apply: every answer, applied or not, is on the run's transcript, and a task created or changed by an approved proposal gets a `solace` note naming the run.
6. A feature or milestone is done when its tasks are, and can come back to planning for new tasks. A proposal that links a task to a `complete` or `canceled` feature or milestone also sets that record to `planned`; the question says so, and both are approved together. Completing a feature or milestone by itself when its tasks are done is not part of this decision.
7. The page has no Plan tab. A planning panel, one component docked beside every tab and collapsible, holds the run: Start takes its context from what is selected (the whole page when nothing is), and the panel shows the transcript, a message box and the proposals with Approve and Reject. It stays open across tab switches. A record with an open run shows a marker, and the panel lists past runs to reopen.
Why: planning is a conversation about the page's records, not a task; the human keeps every change to the plan in their own hands, and the conversation sits beside what it is about.

## D-94 A proposed task may name projects that don't exist yet
Amends D-93 point 4 (Aaron, 2026-10-10).
1. A strategy run's task proposal takes its projects from `scope`; without one it goes in the root's own project. Its home is the first folder in `scope` that is a project.
2. A folder in `scope` may not be a project yet. When none is, the task is still created, with the first folder as its home: it is in no lane, so the board doesn't show it, until that project exists.
Why: planning runs ahead of the code; a task for a project not created yet should still be part of the plan.

## D-95 A finished task is archived from its board card
Amends D-87 point 3 and Spec 15 §1 and §4 (Aaron, 2026-10-10).
1. A card in a final column has an **Archive** button in its upper right corner. It sends `POST /tasks/{id}/archive`, which sets `archivedTime` to now; archiving again keeps the first time. A task in another column is refused (`E7001`).
2. The board leaves archived tasks out of its lanes and counts. They are not deleted: they stay in `GET /tasks` and the event stream, so the Strategy page's Tasks tab (the archive, D-90 point 4) still lists them.
3. A task that leaves the final columns has its `archivedTime` cleared, so it is back on the board.
4. No archived marker or unarchive button for now.
Why: finished tasks crowd the board, and the human wants to clear them one at a time without losing them.

## D-96 Strategy runs are told only what they can do; `grep` refuses what it can't search
Amends D-93 and Spec 14 and 15 §6 (Aaron, board task, 2026-10-10).
1. In a strategy run, `solace mcp` gives its own tool description and hints (reads, `strategy` commands, proposals, `strategy note`), and refusals (E6006, E6013, E6007) give no task-run advice.
2. A strategy run's brief lists the 10 most recent decision titles, then a line saying `grep D-` finds the rest.
3. `grep` refuses a query starting with `-`, containing `\|`, or with words after a quoted term, saying its syntax: literal text, already case-insensitive, one term per command, `--project` picks the project.
Why: agents wasted calls on commands they couldn't use, and `grep -i x` silently searched for the wrong text.

## D-97 Settings is its own app at `/settings`, with a store and pub-sub
Aaron, board task, 2026-10-11.
1. The Settings page is its own Gleam project, `solace-cli/settings` (JavaScript target), component `<solace-settings>`. The shell depends on it by path, registers it and links it in the menu; the server serves the shell at `/settings`.
2. Settings are kept in the browser's `localStorage`, each under its key. `settings.save` stores a value and publishes it; `settings.subscribe(key, handler)` hears changes from this page (a `solace-setting` event) and from other windows (`storage`). A project that needs a setting subscribes to it.
3. The theme is the first setting (`solace-theme`: `light`, `dark`, or unset for the system). The Dark mode switch on Settings replaces the Board's sun/moon button. Each `index.html` still applies it before first paint.
Why: settings will affect many pages; one store with pub-sub lets each learn of a change without depending on the others.

## D-98 A task can be linked to from any text by its id
Amends Spec 15 §4 (Aaron, board task, 2026-10-11).
1. In any markdown Solace renders, `[label](task:<id>)` and a bare `task:<id>` (the task's 16 hex digits) are links to `/task/<id>`. Code keeps them literal; copying gives the markdown back.
2. `GET /tasks/find/{id}`, at the root, says where a task is stored and whether it is archived: `{id, dir, archived}`, or 404 with `E7023`. The server serves the shell at `/task/{id}`.
3. Clicking a task link focuses the task on the board when it is on it; an archived task opens on the Strategy page's Tasks tab; otherwise a toast says "Task does not exist" for a few seconds.
4. A task's details have a **Copy ID** button beside the title, which copies `task:<id>`.
Why: tasks refer to each other in their text, and a human should be able to follow a reference without searching for it.

## D-99 A strategy run reads a task's run log by step
Amends D-93 point 2 and Spec 15 §6 (Aaron, board task, 2026-10-11).
1. `strategy show task <id>` stays compact; its `links` gain `runs`, the ids of the task's runs.
2. `strategy show run <task> <run>` gives the run without its `log` path and its transcript as one line per step: `<n> <kind>: <first line, cut to 100 characters>`, a tool with its status. Steps are numbered from 1; token-usage entries are not steps.
3. `strategy show run <task> <run> <steps>` gives those steps in full (`3,7,10..12`, in the order named, each with its `step` number); `all` gives every step in full. A step outside the transcript is `E7001`; an unknown task or run is `E7023`. A reply is cut at the reply limit as any other.
Why: the log path is unreadable to the agent, and a whole transcript is too long for `show task` or a brief; the agent looks at the outline, then reads the steps it needs.

## D-100 A task's feature and milestone can change while a run is going
Amends D-87 point 4 and Spec 15 §4 (Aaron, 2026-10-09).
1. `PATCH /tasks/{id}` with only `feature` and `milestone` is accepted while a run is going, as in a final column. Any other field during a run is still refused (`E7003`), `dependsOn` included.
Why: the links are for planning and don't affect the agent; a long run or an unanswered question shouldn't hold planning up.

## D-101 A strategy run reads the plan in compact lines
Amends D-14, D-93 point 2, D-99 point 1 and Spec 15 §6 (Aaron, board task, 2026-10-11).
1. `strategy show task <id>` gives the task's own fields — `id`, `title`, `body`, `column`, `scope`, `feature`, `milestone`, `dependsOn`, `closedTime`, `archivedTime` — its `notes` as one line each (`<n> <author> <time>: <first line, cut to 100 characters>`), its `questions` as one line each (the question's first line, then its answer), and its `runs` as one line each (`<id> <kind> <state> <model>`). It carries no `conversation` and no run `log`s; `links` stays first. `strategy show` for a feature or a milestone is unchanged.
2. `strategy show note <task> <notes>` gives those notes in full, each with its `note` number; `all` gives every note. A number outside the notes is `E7001`, an unknown task `E7023`.
3. `strategy list` is one line per record: `Tasks:` with `<id> <column> f:<feature|-> m:<milestone|-> <title>`, `Features:` with `<id> <state> m:<milestone|-> <title>`, `Milestones:` with `<id> <state> #<rank> <title>`, then the vision. A task with an `archivedTime` is left out; `strategy list --archived` includes them.
4. The Claude Code reply budget is 40,000 bytes, not 80,000 (`solace mcp --budget`). Why it is lower: Claude Code saves a reply above its own MCP output limit to a file the agent cannot read, so a reply cut at 80,000 never reaches the agent.
Why: `show task` and `list` returned whole records, and 80,000 is past Claude Code's own limit, so the agent got a file it could not open; it reads an outline and asks for the notes and steps it needs.

## D-102 Commands take a list of arguments, and the reference splits them by purpose
Amends D-44, D-46, D-93 point 2, D-101 point 1, Spec 1 §5, Spec 3 §2, Spec 14 §3 and Spec 15 §6 (Aaron, board task, 2026-10-11).
1. `refs <name>…` gives each named symbol's users, in the order named. `delete <name>…` retires each named symbol in order; the reference check runs over the whole list first, so a symbol used outside the list stops it and nothing is deleted. `files [<dir>]…` and `read <path>…` answer a list of paths in the order named, and a `--lines` or `--tail` range takes one path; with several paths, `read` heads each file with its path, and names the files past the reply limit in a `Next: read` line instead of cutting them. A whole-file `read` cut short does not count as reading the file for a whole-file `write`. `remove <path>…` removes a list whole or nothing: every path is checked first, and a refused path removes none and is named. `write` carries one `text`, so it changes one file and refuses a list. `strategy show <task|feature|milestone> <id>…` gives one record for one id, as today, and for several ids an array of those replies in the order named, each with its `links` and `record`.
2. In the reference's command list, `find` and `grep` are one line each, and `files`, `read`, `write` and `move` are one line each.
3. The reference gives the file tiers as a table, each tier with what an agent may do with its files, and says that a file a command refuses is not a file the agent can't change: graph files change with `submit`, protected ones with `write` and the human's approval.
4. In a task run, `impact` with no names is `task diff` with no names; `task diff` stays.
5. The session-start reference is under 3,600 bytes, not 3,000 (D-46), so its command list can end with a four-line summary of the file tiers; `help` keeps every option and the full tier table.
Why: agents repeated one command per record, `find` and `grep` on one line were confused with each other, and the file tiers as prose left agents hesitating about what they may do.

## D-103 `extract` moves symbols to another module or project
Amends Spec 1 §5 (Aaron, 2026-10-10).
1. `extract <symbols…> --to <module> [--to-project <folder>]` moves symbols with their identity. It brings along private helpers that only they use, rewrites every remaining reference and import, moves tests that reach only moved code, and refuses a module or package dependency cycle with `E5002`, naming the boundary uses. A moved symbol keeps its revision unless its text changes: rewriting references inside it, or making it `pub` because the other side now uses it, gives it a new revision under the same identity.
2. It runs in one transaction and is gated on `check_all`: any new error anywhere, compared with what was broken before, leaves the store unchanged.
3. `rename` changes only a symbol's name; `extract` changes its module or project. This replaces Spec 1 §5's Phase 1b Extract row. An operation that creates `derived-from` identities has no verb until one is needed.
Why: an agent extracting components rewrote references by hand and a submit failed. The human chose `extract` over `move` and `rename`, accepting the change to Spec 1.

## D-104 `write` with no path takes a patch of several files
Amends D-102 point 1 and Spec 14 §3 (Aaron, board task, 2026-10-11).
1. `write` with no path takes `text` as a patch in `apply_patch` form, a section per file: `*** Update File: <path>` with `@@` hunks matched as `submit`'s are (D-47), `*** Add File: <path>` with `+` lines, `*** Delete File: <path>`. An update needs no earlier `read`, since the hunks matching is the check; lines added with no `@@` line go at the end of the file. `*** Move` is refused (`move` does that), and so is a path named twice.
2. Whole or nothing: every section is checked (tier, reach, an Add onto an existing file, an Update or Delete of a missing one, hunks, size) and applied in memory before any file changes; then every file is written before any is validated, and if a validator fails, every file goes back. Each refused section is named, after `nothing written:`.
3. In a board run, each file the patch names is recorded as the task's change, so cancel undoes them all. `remove`'s snapshot covers every path of its list, not only the first.
4. `write <path>` is unchanged. A `submit` patch that names a file other than `.gleam` source says to use `write`. Code and files in one patch is left for later: it would need the store savepoint and the file undo records to commit together.
Why: a call carries one `text`, so each file took its own tool call. The patch's section headers mark where each file starts, in a format `submit` already takes and agents know. The human chose it over a structured `writes` argument.

## D-105 Strategy runs: proposal fields listed, notes and steps given whole, `--lines` takes a list
Amends D-99 point 3, D-101 point 2, D-102 point 1, Spec 14 §3 and Spec 15 §6 (Aaron, board task, 2026-10-11).
1. A strategy run's brief and `help strategy` list each proposal kind's fields: a task `title, body, feature, milestone, dependsOn`, and `scope` only to create; a feature `title, body, milestone, state`; a milestone `title, body, targetDate, state`; `rank` only to edit. A proposal naming any other field is refused (`E7001`), naming it and those the kind takes, so a misspelt field isn't silently dropped. The brief and `help` also say that the Approve view shows `body` as Markdown and the other fields as plain text, and that text outside `strategy note` shows only in the run's live transcript.
2. `strategy show note` and `strategy show run … <steps>|all` stop at the end of the last note or step that fits the reply limit and name the rest in a `Next: strategy show note <task> 15..17` line (or `… run <task> <run> …`). A first note or step over the limit by itself comes in parts: as much of its longest text field as fits, marked `part: "<field>, characters a..b of n"`, and a `Next:` line ending in `--from <b>`; `--from <n>` starts the first one named at character `n` of that field.
3. `read <path> --lines` takes lines and ranges, `150,156,200..210`: shown in file order, overlaps merged, a `…` line at each gap. A reply cut at the limit names the rest in its `Next: read` line. A range still takes one path.
4. The reference and the strategy brief ask agents to cite code by name (`board.rs::within`, `checkout.pay`), not `file:line`, since line numbers move.
5. Batch proposals (several records approved as one) are deferred until single approvals prove a burden.
Why: a strategy run planning the `extract` feature had to read the code to find the proposal fields, re-read notes cut midway, had `--lines 150,156` refused (as agents often expect it to work), and cited line numbers that moved within a day.

## D-106 Strategy runs: short proposal titles
Amends Spec 15 §6 (Aaron, board task, 2026-10-11).
1. The strategy brief and `help strategy` say a proposal's `title` names the work in a few words (about 3–7), prefixed with the feature's short name when that helps (`extract: JS FFI files`); mechanism, conditions, error codes and lists go in the `body`.
Why: strategy agents wrote titles that summarised the body, which read badly on board cards and in `strategy list`.

## D-107 A menu tab flashes when its page needs attention
Aaron, board task, 2026-10-11.
1. A page tells the shell something needs the human with an `attention` event on its element. The board sends it when a live update raises a notice; Strategy when a strategy run goes waiting (its agent replied) or asks a new proposal. A replay sends nothing.
2. The shell flags that page's menu tab unless it is the page shown. A flagged tab flashes (steady under reduced motion) until its page is shown.
3. The board and Strategy are open from the start, hidden, so each listens on its own stream; other pages still open on first visit.
Why: an agent's reply on a page the human wasn't looking at went unnoticed. Each page keeps its own rules for what needs attention, and the shell opens no stream of its own (the human chose this over a shell stream).

## D-108 A canceled task goes back to the backlog by a workflow action
Amends Spec 15 §2 and §3 (Aaron, board task, 2026-10-11).
1. A final column may list actions. It offers only those: no `pause`, `resume`, `discuss`, `stop` or `end_discussion`, since no agent works there. Edits there stay refused (`E7004`).
2. The default `cancelled` column lists `to_backlog`; `complete` lists none, so a completed task stays as it is. Actions are per column, so nothing else is needed to keep it to `cancelled`.
3. An action may have a `label`, which its button shows instead of its name; `to_backlog`'s is "Back to backlog".
4. An action that moves a task out of a final column adds a `solace` note: "The human brought this task back from <column> to <column>." The move clears `archivedTime` (D-95) and keeps `closedTime` (D-87); the task keeps its notes, runs, links and `dependsOn`. Cancel already reverted and settled it, so nothing is undone and the next run starts fresh.
Why: a canceled task couldn't come back. The human chose a workflow action over a dedicated request: the workflow already decides what each column offers.

## D-109 Commands refuse misuse that loses work and say what to type instead
Amends D-96 point 3, D-104 point 4 and Spec 14 §3 (Aaron, board task, 2026-10-11).
1. `write <path>` refuses patch text (`drafts::is_patch`) with a usage error and changes nothing: "this text is a patch; send it with `write` and no path". It mirrors `write` with no path refusing text that isn't a patch (D-104).
2. `write <path>` with a whole text refuses one that would leave fewer than half the lines of an existing file of 200 lines or more (`E6002`), and changes nothing. There is no flag to allow it: to replace such a file, `remove` it first, then `write` it new. A replacement with `old` and a `write` patch are not checked. A `--lines` read still allows a whole-file write, so the size is the check.
3. A `write <path>` reply gives the file's lines before and after: `Wrote a.rs (1855 → 1860 lines)`, or `Wrote a.rs (43 lines)` for a new file.
4. `read` checks every path before reading any. A path that doesn't exist is refused with what it likely meant: a line range (`1340-1400`, `1340..1400`, `1340:1400`) "is not a file; for lines use `read <file> --lines 1340..1400`"; `a.rs:5-9` the same, for `a.rs`; a name with no `/` and no `.` adds that `read` takes files, not symbols, and that `grep` and `find` find them; any other is named (`E6002`, no such file).
5. `grep` searches its text as written and never refuses for syntax, replacing D-96 point 3. Quotes are dropped only when they wrap the whole query as one term (`"write" =>` keeps them). When nothing matches and the query looks like a shell habit (a leading option such as `-i`, `\|`, `\"`, a quoted term with more after it, or a query ending in `--path <x>`, `--in <x>` or a file path), the reply adds one line: the text searched, then the syntax; for the path forms it also says grep has no path filter and searches every watched file. A plain miss gets no hint.
6. A batched call where commands failed ends its reply with `N of M commands failed: <commands>`. The run log keeps the step `failed` and, when only some failed, says so in its title: `solace (1 of 3 failed) …`. A new status would show as running in the shared transcript view, so the title carries it.
Why: an agent's whole-file `write` with patch text replaced a 1,855-line file, and other replies to shell habits (`grep -i`, `read a.rs 1340-1400`) didn't say what to type; D-96's refusals also blocked real code (`"write" =>`, `-> Result`, Markdown tables). The human chose the shrink guard with no override flag.
