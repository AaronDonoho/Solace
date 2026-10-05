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
New tasks sit behind a button that keeps the draft; editing is on the Details tab. Task details are tabs (Details, Notes, Questions, Runs, Changes), newest first. Effort is chosen separately from the model; the run records it, and a changed effort starts a fresh conversation. A successful run marks the task finished ("Agent finished", "Report ready", "Review ready") until the next action. Notes take PNG, JPEG, GIF or WebP images (5 MB, content-checked, no SVG), stored in `.solace/board/` and attached to the agent's prompt as `[image N]`. The markdown editor is built in.

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
