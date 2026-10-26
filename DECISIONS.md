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

Built: skills. The `skills` key in solace.toml names the skills folder, `skills/` by default. Each skill is a subfolder with a `SKILL.md`, or a single `<name>.md` file at the folder's top, with `name` and `description` in front matter; a skill with no description is listed by its first line. Every run's brief lists the project's skills (name, description, path to `read`) and carries no skill contents. The folder is protected like AGENTS.md: agents read skills and change none, and Solace never runs a skill's scripts — a skill that needs a tool names a `[commands]` entry the human adds. Memory is not built.

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
5. Task-only actions take no project lock: create, edit, move, note, stop, cancelling a task that never had changes, scope and answers. `agent_guard` reads run state from the task store. Actions that change code take the store lock of each project they touch: undo, moving into a final column when that undoes the task's changes or leaves some to settle, approving a settings proposal, accepting outside changes, adding or removing a project. A final move's settle runs after the reply, on its own thread when a server is running (point 7). A scope change stays lock-free: a project with the task's changes can't leave its scope (`E7013`), and a project that leaves takes its lock with it; which task changed what is told apart later (Aaron, 2026-10-07).
6. Create is idempotent: `POST /tasks` carries a client `request` id, and the same id returns the same task.
7. Addition to point 5 (Aaron, 2026-10-07): `task done` settles nothing in the agent. The server settles (approves, drops undo records) after it applies the move into a final column and replies; when a server is running, that settle runs after the reply on a thread of its own, and it takes the scope locks like a human action, re-reads the task and settles only if it is still final. It can run again, and `serve` runs it for final tasks that still have undo records. Undo and settle first adopt any pending record of the task under the locks, and a pending record makes cancel take them.
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
7. The page has no Plan tab. A planning panel, one component docked beside every tab and collapsible, holds the run: Start takes its context from what is selected (the whole page when nothing is), and the panel shows the transcript, a message box and the proposals with Approve and Reject, and Approve to Planning for a new task with no feature (D-201). It stays open across tab switches. A record with an open run shows a marker, and the panel lists past runs to reopen.
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

## D-110 A run's brief names its unfinished dependencies
Amends D-87 point 6, Spec 3 §2 and Spec 15 §1 and §6 (Aaron, board task, 2026-10-11).
1. When a task has a `dependsOn` task that is unfinished, as the board means it (not in a final column, so complete and canceled are finished; a reopened task counts again), the run prompt and `session start --task` in that task's run list a **Depends on** section. It names each dependency with its id, title and column (and its project when that is not the task's home), and marks the unfinished ones. Dependencies are read from the root's task store, from any project.
2. The section ends with: "This task depends on unfinished work. Check whether you can do your part without it. If you can't, ask the human (`task ask`) before building on code that doesn't exist yet." It is advice; nothing is refused.
3. With no dependencies, or only finished ones, the brief says nothing about them.
4. The section is task data, outside the session-start reference, so D-102 point 5's 3,600-byte cap is unchanged.
Why: an agent built on work from a task that wasn't finished, without knowing it was missing.

## D-111 `run` and `build` replies are condensed
Amends Spec 14 §4 (Aaron, board task, 2026-10-11).
1. A `run` or `build` reply gives the exit code, then the output without build progress (cargo's and Gleam's indented status lines: `   Compiling`, `    Finished`, `     Running`…; libtest's `running N tests`), passing tests (`test x ... ok`, `... ignored`, `run_parallel.py`'s `ok    <test>`, lines of dots), `test result` lines that ran nothing (one is kept when no other is) and the text of warnings (a `warning:` line with cargo's indented or numbered lines after it, up to a blank line; Gleam's, framed by `┌─`, up to the next status line). Only these forms match, so other programs' output passes through. Failures with their messages and tracebacks, errors, panics and summaries stay, in order; warnings stay as cargo's `generated N warnings` line, or `N warnings not shown`. Then the reply budget applies as before, so its first 20 lines are now failures, not build progress. `format` replies are not condensed. A build script's warning (`warning: solace@0.1.0: …`), one line naming the step to run, stays whole (D-159).
2. The full output is not saved; for detail, run a narrower command (`rust-unit <filter>`).
3. `test`'s run of `gleam test` is read whole before its failures are parsed; only its reply is cut to the budget.
Why: test runs' passing lines and warnings filled agents' context, up to the whole reply budget per run, and the budget's cut kept the first 20 lines, which were build progress.

## D-112 A completed task's review tech debt goes to its feature's Tech Debt task
Amends D-93 point 4 and Spec 15 §1, §4 and §6 (Aaron, board task, 2026-10-11).
1. A review of a task that has a feature is asked to put minor, non-blocking findings that later work in the feature should fix under a `### Tech debt` heading in its feedback, one finding per bullet with its location. No new command: the review still ends with `task note`. A task with no feature isn't asked.
2. When a task is completed (an action moves it into a final column without reverting it, so not a cancel), Solace reads the `### Tech debt` section of its last review: the agent's note of its last review run that ended `reviewed`. Each top-level bullet, with its indented lines, is one finding; the section ends at the next heading.
3. The findings go to the feature's open **Tech Debt task**: a task with `techDebt: true` and that `feature`, not in a final column. With none, Solace creates one in the first column of the completed task's project, titled `<feature title>: Tech Debt`, with the feature, its milestone and the completed task's `scope`. A completed or canceled Tech Debt task is never reopened: the next finding makes a new one.
4. Each finding is one `solace` note on the Tech Debt task naming its source: `Tech debt from the review of task:<id> (review run <rid>):` and the finding. The completed task gets a `solace` note naming the Tech Debt task. Both tasks are written in the same operation, the Tech Debt task first. A task with no feature, or a review with no section, files nothing.
5. This needs no proposal or approval (amending D-93 point 4): it records findings and schedules no work.
6. The board marks a Tech Debt task's card "Tech debt". There is no open count, no resolving of items and nothing in other runs' briefs.
7. A task brought back from a final column and completed again files its last review's tech debt again; accepted.
Why: reviewers' minor findings sat only in the reviewed task's notes, so later tasks in the feature never saw them and they piled up as hidden tech debt. The human chose a section read at completion over a review command, and kept the scope to filing.

## D-113 Tasks are colored by their feature, in the order features started
Amends Spec 15 §1 (Aaron, board task, 2026-10-11).
1. A feature gains `startTime`: when it first became `active`, in seconds. The server sets it, on a create or edit that leaves the feature `active` with none, and never clears it; clients can't set it. A feature saved before it was kept takes its file's time if it is already `active` or `complete`, and null otherwise.
2. Each started feature gets a color by `startTime` (ties by id): the n-th takes place `(n × 5) mod 16` of a 16-color palette in rainbow order. Each step skips 4 colors, so features started one after another are easy to tell apart, and all 16 are used before one repeats.
3. A board card's stripe is its task's feature's color, gray for a task with no feature or one not started. The stripe no longer shows the agent's status; the badge, attention text, stamp and hazard mark still do. The Features tab shows each started feature's color on its edge.
Why: the human wanted to see at a glance which feature each task serves. They chose the start date over the creation date, so colors follow when work on a feature began and don't shift when features are reordered.

## D-114 A feature keeps its color slot from when it started
Amends D-113 point 2 and Spec 15 §1 (Aaron, board task, 2026-10-12).
1. A feature gains `colorSlot`: the server sets it when it sets `startTime`, one past the highest slot any feature holds (0 for the first), and never changes it; clients can't set it. A feature takes color `(colorSlot × 5) mod 16` of the D-113 palette, so deleting a feature changes no other feature's color.
2. A started feature saved before slots were kept takes one after every kept slot, in `startTime` order (ties by id), when it is read; the server saves those slots before it next writes or deletes a feature, so they don't move after.
3. A board card also names its feature in its tooltip and for screen readers, so color is not its only sign. The palette's orange, yellow, green and cyan are darkened to at least 3:1 contrast against white.
Why: the review of D-113 found that numbering features by start order shifted every later feature's color when one was deleted; the human chose to keep each color fixed once assigned.

## D-115 A proposal carries the record's current values; the human sees only what changes
Amends Spec 15 §6 (Aaron, board task, 2026-10-12).
1. A proposal with an id (an edit) carries `before`: the record's current value of each field the proposal names, as the request's values are; `strategy vision` carries the vision's text under `before.text`. A create carries none. Approve applies the request as before, and ignores `before`.
2. The human sees only what changes. An edit shows the fields whose proposed value differs from `before`: a short one as `old → new`, a record id by its title, and the `body` as a paragraph diff — a paragraph both bodies share collapsed to one muted line, removed lines struck, added lines marked — with a toggle to see the new body as Markdown. An edit whose fields all match says "No change". The vision's new text diffs against the vision the same way.
3. A create, and a proposal saved before `before` was kept, show every field as they do today. The question's `text` the agent writes is unchanged.
Why: a proposal restated the whole record, so the human read the same text each time to find the change.

## D-116 A proposal's body shows as a line diff in its order; the vision says No change
Amends D-115 point 2 and Spec 15 §6 (Aaron, board task, 2026-10-12).
1. The body diff is a line diff in the body's order: the lines the two bodies share at the start and at the end collapse to one muted line each, a line only in the old body is struck, one only in the new one is marked, each shown where it is. Lines are compared with their surrounding space removed and blank lines are left out, so a change only in the space around lines, or a paragraph break added or removed, doesn't show; moving a line break does. A run of one line reads "1 line unchanged".
2. A vision proposal whose text equals the vision says "No change", as an edit whose fields all match does. A body or vision text that differs only in what the diff ignores counts as unchanged, so it can say "No change" too.
3. A vision proposal saved without `before` shows its text as Markdown, as before D-115.
Why: the paragraph-set diff lost the body's order, so a paragraph edited in the middle showed after every unchanged one, and a vision proposal that moved nothing showed collapsed lines instead of "No change".

## D-117 `merge` folds a duplicate into its copy; `extract` stays a pure move
Amends D-102 point 5, D-103, Spec 1 §5 and Spec 3 §2 (Aaron, board task, 2026-10-12).
1. `merge <survivor> <retired> [--to-project <folder>]` covers the duplicate case: the survivor keeps its identity; the retired symbol stops being live, with a `merged-into <survivor id>` history entry; every reference to it, tests and the dependent projects the task covers too, is redirected through `extract`'s rewrite (qualified uses, unqualified items, constructor-only `{Task}`, no invented alias), and imports left unused are removed. Rewritten symbols get new revisions. `--to-project` names the survivor's project, which is only read; the command runs in the retired symbol's project. When the survivor isn't found here but is in exactly one other project, the refusal gives the command with `--to-project`.
2. Refused before anything changes, one line each with what to do instead: names that don't resolve (`E1001`/`E1002`); one symbol named twice; a type and a function or constant (`E5008`); types whose constructors differ by name, arity or labels (`E5008`, listing each difference); a private survivor used from other modules, and a user project that doesn't depend on the survivor's through gleam.toml (`E5009`, naming the modules and the line to add); a survivor that uses the retired symbol, and a redirect that closes a module or package loop (`E5002`, naming the uses); a dependent project outside the task that uses the retired symbol (`E5003`, as `extract`). Private helpers only the retired symbol used are named as now unused, not deleted.
3. One transaction, gated on `check_all` like `extract`: a result that doesn't type-check changes nothing and gives the first errors. The task's undo records cover the source and its dependents, so revert and cancel restore both symbols.
4. `extract`'s E1004 for a same-kind name taken in the target ends with `to fold <retired> into it, use merge <survivor> <retired>` (with `--to-project` for another project).
5. The reply gives names and counts: `Merged board.Task into solace_task.Task (components)`, `Redirected 9 references in 3 modules (2 projects)`, `Now unused: …`.
6. `merge` is a change: refused in reviews, investigations and strategy runs. The session-start reference's cap is 3,700 bytes, not 3,600 (D-102 point 5), for its `merge` line; it should still grow only when orienting agents needs it.
Why: an extract into components was refused because `solace_task.Task` already existed there, and nothing could move `board.Task` onto it. The human left the command's form to the agents who use it; they chose `--to-project`, the flag `extract` already uses.

## D-118 A strategy run's conversation is in order and follows the newest message
Amends Spec 15 §1 (Aaron, board task, 2026-10-12).
1. A strategy run's question carries `time`: when it was asked, in seconds, set by the server as the proposal lands on the run. A question saved before it was kept has none.
2. The planning panel draws the run's notes and questions as one conversation by time; a question without a time comes after the notes, in the questions' order. The agent transcript is a collapsed section at the top of the run, never between the newest message and the message box.
3. Opening a run shows its newest message. A new message or proposal scrolls to it when the human was at (or near) the bottom; otherwise a "New message ↓" button shows, and pressing it scrolls down.
Why: the human couldn't see the planning agent's last message: notes were drawn before every proposal, so a note written after a proposal landed above it, and nothing scrolled to it.

## D-119 File tools: grep context and path, files line counts, a call stops at a failed change
Amends D-109 point 5, Spec 3 §1 and Spec 14 §3 (Aaron, board task, 2026-10-12).
1. `grep <text> [--in <path>] [--context n]`. `--in <path>` limits the search to one file or folder, checked as `read` checks a path (inside the project, no symlinks; no such path is refused and named); an unwatched or internal path is named as not searched. `--context n` (1 to 5; another number is a usage error) adds n lines before and after each match, marked `12-` against a match's `12:`; windows that touch merge, gaps show as `…`, and context lines are cut around long lines as matches are. Both options are recognized only as the last words of the query, in either order, so the text before them is searched as written and the query is never refused for syntax (D-109 point 5). No glob, no regex.
2. Up to 10 matching lines per file (was 6) and 40 files. A file with more matches heads with `(N matches, showing 10)`. The reply stops at a whole file within the call's shared reply budget and says how many files it left out and to narrow with `--in`.
3. D-109 point 5's hint for a path at the end of the query, `--path <x>` and `--in <x>` now says to use `--in <path>`; "grep has no path filter" goes.
4. `files` ends each listed file other than graph source with its line count (a UTF-8 file) or its size in bytes (any other, or one over `watch_limit`). Folders stay one line with a file count.
5. When a command that changes the project (`submit`, `delete`, `extract`, `merge`, `write`, `remove`, `move`) fails, the commands after it in the call do not run. The reply ends `N of M commands failed: <commands>; skipped: <commands>`. A failed read does not stop the call (D-109 point 6). The session-start reference's step 2 then extends "change and test in one call" to plain-file projects: `write`, then `run <name>`.
6. The brief and the `task ask` line of the reference say to check the task's Questions and Notes for what is already answered before asking: a discussion run cannot `task ask`, so its answers are notes.
7. Kept: the shrink guard (D-109 point 2) with no override, hunk matching, `--tail`, the reply budget on `read`, and D-111's condensing, which already keeps cargo's `error:` blocks whole.
8. Rejected: regex; a path given as a positional word (a query ending in a file name would silently become a filtered search); `write --dry-run` (a patch is already checked whole before anything is written and names every refused section, and a dry run cannot run the validators); `condense = off` per command (`cargo --timings` writes its report to a file, so the screen holds only progress; `read` the report); whole-file reads past the budget; sed-style range writes.
9. The reference changes land with the code that makes them true, so the reference never describes a command that does not exist: follow-on tasks for (a) grep options with their counts and budget stop, (b) `files` counts, (c) the call stopping at a failed change, with `board.rs` reading the new last line for the run log's title, (d) the step 2 and `task ask` wording and the brief line. Each has `rust-unit` tests and `rust-acceptance` for the new options; the session-start reference stays under D-117's 3,700 bytes.
Why: `rg -C` returns a match with its surroundings in one call and Solace's grep returned one cut line, so each hit cost a `read --lines` turn; `files` gave no sizes, so agents probed ranges blind; and a call that carried on after a failed `write` ran the tests on the old code. The red team (the task's notes) cut `--dry-run`, regex and `condense = off`; the human agreed with the rest.
Note: point 5's "(D-109 point 6)" for "a failed read does not stop the call" is a wrong citation; D-109 point 6 only sets the summary line, and the rule is point 5's own. Spec 3 §1 cites D-119 point 5 (task "Batch stops after a failed change", 2026-10-12).

## D-120 An unmatched hunk names its line; a refused `write` patch is kept as a draft
Amends D-104 point 2, Spec 3 §2 and Spec 14 §3 (Aaron, board task, 2026-10-12).
1. When a hunk's lines aren't found and its first unmatched `-` or context line (trimmed) appears inside exactly one line of the target, `E0108` adds: "`<text>` is only part of line N: `<line>`. A patch line is a whole line: copy all of it, or change just that text with `write <path>` and `old` and `text`." For a symbol, "a longer line" and `submit <name> --replace`. This covers `submit` and `write` patches.
2. A refused `write` patch is kept as written, as a draft, and the refusal names it, as `submit` drafts are (D-34, D-47). `write --draft <id> --replace` (`old`, `text`, `--all`) edits it and retries; `write --draft <id>` retries it as is. The retry is the whole patch, whole or nothing as before; one that applies removes the draft and records each file as the task's change. A file draft has no fingerprint: the hunks matching the current files is the check. `drafts` and `view --draft` work in plain-files projects.
3. The reference's `write` and `submit` patch entries say a patch line is a whole line and point to `old`/`--replace` for part of one; the `write` entry says to fix a refused patch's draft, not resend it.
4. Rejected: saying which sections matched (the agent still resends them), resending only the bad section (the same tokens), one call per file (loses D-104's atomicity across files), and telling investigate runs to plan edits as whole lines (the brief is in every context window; points 1 and 2 make the slip cheap wherever it starts).
Why: a work run's six-file `write` patch had one hunk whose `-` line was the end of a line; the reply gave no hint, nothing was written, and the agent resent the whole ~200-line patch to fix one line.

## D-121 `view` reads Rust and Python files by item
Amends Spec 3 and Spec 14 (Aaron, board task, 2026-10-12).
1. `view <file>` gives a watched `.rs` or `.py` file's outline: one line per item in file order, with its line range, signature and first `///` line (a Python docstring's first line). Items of `impl` and `trait` blocks are `Type::method`, those of an inline `mod` `tests::name`, a Python class's `Class.method`, each indented under its block. `#[path = "x.rs"] mod m;` names the file. Over the reply budget the outline gives kind and name only, then cuts at a whole line, saying what it left out to fit the reply (D-14).
2. `view <file>::<name>` gives one item's source, with the docs, attributes and comments right above it, numbered as `read --lines` numbers lines and headed `path:start..end`, so a `write` patch or `read --lines` anchors on it. A name matches an item's full name, else a suffix (`board.rs::within`, `board.rs::Store::view`); a type wins over its `impl`. An ambiguous name lists its candidates with their ranges (`E1002`); a miss says so and names `grep <name> --in <file> --context 3` (`E1001`). Several operands share the reply, as `read`'s paths do.
3. `<file>` is a watched file's path or an unambiguous path suffix (`board.rs`). An operand is a file form when it has `::`, or ends in `.rs` or `.py` and names a watched file; in a project with Gleam code, a `.rs` name that names no file stays a symbol. Files and symbols don't mix in one command.
4. Items are found by a line scanner, with no new dependency. It skips strings (raw strings too), char literals and comments, counts brackets, and takes a head from a line that starts in code at its block's depth: `fn`, `struct`, `enum`, `union`, `trait`, `type`, `const`, `static`, `mod`, `impl` or `macro_rules!`, after visibility and qualifiers. An item ends where its brackets close, or at its `;`. Functions nested in a body are not items. Python items are `def` and `class` at the top level and in classes, each ending before the next line no deeper than its head.
5. In a project of plain files, E6013 for `view`, `inspect` and `refs` names the file forms.
6. Deferred, each to its own task and decision if Rust runs still spend turns on them: `refs` and `inspect --context` for files, a command→test mapping, automatic Rust `test`, a session-start packet for plain-file projects, and a tree-sitter index.
Why: after D-119, reading a whole function still took two turns: `grep --context` found where it started but not where it ended, so a guessed `read --lines` followed, and seeing what a file holds took several greps. The task first proposed ending an item at the first `}` in its column; tests here hold Gleam source in raw strings with `fn` and `}` at column 0, so the scanner tracks strings and brackets instead.

## D-122 Features and milestones are linked to as tasks are; a bare link shows the title
Amends D-98 and Spec 15 §4 (Aaron, board task, 2026-10-13).
1. In any markdown Solace renders, `[label](feature:<id>)`, `[label](milestone:<id>)` and bare `feature:<id>`, `milestone:<id>` are links to `/feature/<id>` and `/milestone/<id>`, as task links are (D-98 point 1). The server serves the shell at both addresses.
2. A bare link (`task:<id>`, `feature:<id>`, `milestone:<id>`) shows the record's title once the shell has it; until then, and when the record isn't found, it shows its reference. A labelled link keeps its label. Copying a link gives its markdown back.
3. `GET /tasks/find/{id}` adds `title`; `GET /features/find/{id}` and `GET /milestones/find/{id}` give `{id, title}`, 404 with `E7023` for no such record.
4. Clicking a feature link opens the Strategy page's Features tab with the feature's card shown; a milestone link the Roadmap tab with its card. An unknown one toasts "Feature does not exist" or "Milestone does not exist".
Why: tasks, features and milestones refer to each other in their text; a reference by id alone says nothing to a human until followed.

## D-123 A run starts where its task's symbols are, and the session says where that is
Amends D-57, D-61 and D-77 (an agent no longer always works from the task's home), D-102 point 5, D-117 and D-119 point 9 (the session reference's byte cap) (Aaron, board task, 2026-10-13).
1. A task run starts in the project of its scope that holds the most symbols its title and body name as `module.name`; on a tie, or with none, at the task's home as before. A resumed conversation keeps the project it started in (kept in the task's `conversation`). The task stays stored and at home where it was: each command finds the home from the task, and also locks the project the agent works in. The brief's "You work in" names the start project.
2. `session start --task` ranks symbols from all the task's projects, with word weights counted over all of them; one from another project is labelled `-- module.name (<folder>)`. The modules list and the reproduction hint stay with the current project.
3. The session's project section starts with `You are in <folder> (<name>).`, then, in a task run, `This task can change: …` apart from `Read only: …` (the root last), the `--project` rule (a folder as listed, from the Solace root, with an example and its `../x` counterpart; this line only; reads, `grep` included, reach every listed project, other commands only the task's) and the path rule (paths in `read`, `grep --in`, `files`, `write` and `move` are relative to the project the line runs in).
4. An `E1001 unknown name` line is followed by `found in <folder>: add --project <folder>` when another project has exactly one symbol of that name: the task's projects first, then the root's others. It is looked up only when a reply has the error, so `resolve` and naming stay cheap.
5. Reference: restored a line that each line runs in one project (yours, as session start names it, or its `--project`) with paths relative to it, and that options belong to one command (`--lines` is `read`'s) with `help <command>` listing them; `read`'s line shows `--lines`. Cut as repetition: "`help` has the table" (the options line says it), and wording in the opening and `drafts` lines. The `--project` entry in the full reference states the same rule as point 3.
6. The session-start reference has no byte cap (ends D-102 point 5's 3,600 and D-117's 3,700), and its size test is removed. Its purpose is that an agent understands the tools at session start and avoids mistakes with them; it stays as brief as that allows. Text is cut when it repeats or nothing depends on it, not to meet a size.
Why: an investigation of a task across `components` and `strategy` started in components, its relevant symbols were all from there, and 7 of its 9 failed commands (`view plan --project ../strategy` first) came from not knowing where it was or how folders and paths resolve.

## D-124 `extract`'s JavaScript files: `E5007` and the reply lines
Amends Spec 1 §5 and Spec 3 §2 (Aaron, board task, 2026-10-13).
1. `E5007`: into another project, `extract` refuses when it can't carry a JavaScript file that a moved FFI function uses: the path leaves the package, or the file is in neither `src/` nor `test/`; two files would land on one path; the destination already has a file there; staying code uses a file the moved code takes; a file that stays imports one that goes, or one that goes imports one that stays (a relative import can't reach another package). Each refusal names the files and what to do.
2. The reply names each carried file, `Moved file <from> to <project>: <to>`, adding `(its imports rewritten)` when its relative imports changed for its new folder, and each FFI path that follows its module, `Rewrote FFI paths: <name> <old> -> <new>, …`. FFI paths aren't type-checked, so the reply names the declared suites that reach moved code (`Also reached: …`); when FFI paths or files moved and none does, it says so and asks to run the JavaScript tests or declare them with `checks` (D-44).
Why: these came with the JavaScript FFI work on `extract` and were recorded only in the code; a review filed them as tech debt. Splitting `E5008` and `E5009` by case was declined: each message already names its case and what to do.

## D-125 A refused delete or rejected submit with a same-name copy names `merge`
Amends D-117 (Aaron, board task, 2026-10-13).
1. `delete`'s `E5001` ends with one hint when a refused type has a type of the same name in another module of this project or in another project of the run's scope: `Hint: <copy> [(<folder>)] has the same name. To fold <gone> into it and redirect its N uses, use merge <copy> <gone> [--to-project <folder>]`. Functions and constants get none: shared names like `new` rarely mean duplicates, and `merge` checks no signature.
2. A submit rejected with errors that name two types of one name (`Task` or `board.Task` beside `solace_task.Task`) ends with the same hint, once per reply.
3. When the two types' constructors differ, the hint lists the differences and says to make them match, then gives the merge; a private copy is named as private, to make pub first (`merge` refuses it, `E5009`). Several copies are listed with `merge <copy> …`.
4. The hint names only `merge`: both triggers need the copy to exist, so an `extract` hint for a free name could never show.
Why: an agent that rewrote references by hand to a copy got errors that didn't point to `merge`. The human chose a merge-only hint that names what to fix when `merge` would refuse.

## D-126 Paths: root paths for reads, refusals that name the base
Amends D-123 point 3 (Aaron, board task, 2026-10-13).
1. `read`, `files` and `grep --in` take a path from the Solace root into another project (`solace-cli/strategy/src/api.mjs`) when it isn't in the current project, including a nested project's path read from the root; it is read there, headed `==== <path> (<folder>) ====`. `write`, a file patch's sections, `remove` and both ends of `move` refuse it, naming `add --project <folder>`.
2. An `E6002` for a missing path says `(paths are relative to <folder>)` and, when another project has it, `it is <path> in <folder>: add --project <folder>`.
3. `--project ../x` or `./x` resolves against the current project when that lands on a project under the root; otherwise as a folder from the root. The refusal says `--project` takes a folder from the Solace root and lists them. Session start no longer names `../x` as wrong.
4. `view`, `inspect`, `refs`, `files` and `read` refuse an option they don't take by name (`view has no --lines; read takes --lines`), listing their options from the reference's syntax lines. `grep` and `find` search their text as written and are not checked.
Why: an agent in `components` had six commands refused while guessing what paths and `--project` were relative to.

## D-127 A task's scope widens at any time and narrows only while no agent works
Amends D-61 point 2 (Aaron, board task, 2026-10-13). The human adds projects to a task's scope at any time, during a run too; an open work run claims each added project at once (`E7005` if another task's work run holds it). Projects are removed only while no agent is working: between runs, paused, or waiting on a question or a discussion reply (`E7003` otherwise). A project the task changed, the last project, and the home while a run is open still can't be removed. The Scope tab disables only the switches that are on while the agent works.
Why: an agent that asks for another project should get it without the human stopping its run.

## D-128 A feature's state follows its tasks; its tasks show on the board once it is active
Amends D-87 point 8, D-93 and Spec 15 §1 (Aaron, board task, 2026-10-13).
1. `planned` is renamed `upcoming`, for features and milestones: "plan" already names other things here. A record saved `planned` reads as `upcoming`; a request naming `planned` is refused (`E7001`). A strategy run's outcome `planned` is unchanged.
2. The states mean: **proposed**, suggested and not agreed; **upcoming**, agreed and queued, waiting its turn (a manual gate before `active` may come later); **active**, work has started; **complete**; **canceled**.
3. An `active` feature with at least one task becomes `complete` by itself when every task linked to it is in a final column: canceled tasks and its Tech Debt tasks (D-112) count, so an open Tech Debt task keeps the feature active. A `complete` feature with a task out of the final columns, reopened or newly linked, is `active` again. Nothing else changes a state by itself; a feature with no tasks stays as it is. It is checked when a task is created, moves column or changes feature.
4. The board shows a task with a feature only while that feature is `active` or `complete`: tasks of a `proposed`, `upcoming` or `canceled` feature are off the board, and stay on the Strategy page's Features tab. A task whose feature isn't loaded yet shows.
5. A task proposal linked to a `complete` or `canceled` feature or milestone sets it back to `upcoming` on Approve, as D-93 set it `planned`.
Why: the human wanted features to finish without being closed by hand, and a feature's tasks kept off the board until work on it starts.

## D-129 A proposed task's scope names projects; a task with no project shows on the Strategy page
Amends D-94 (Aaron, board task, 2026-10-13).
1. Every folder in a proposed task's `scope` must be a project of the workspace. A proposal naming a folder that isn't one is refused when it is sent, before the human is asked, naming the folder and the workspace's projects. To plan for a project that doesn't exist yet, leave `scope` out, or use `.` and name the future project in the body.
2. A task whose home is not a project is in no lane on the board. The Strategy page lists every task in the root's store whatever its home: such a task shows on the Tasks tab and in search, opens from a link, and is marked as having no project. The human can cancel and archive it; changing its scope comes in a later decision.
Why: a strategy run proposed tasks with a scope folder that isn't a project; under D-94 they were created with a home in no lane, invisible on the board and the Strategy page, and links to them were refused.

## D-130 A task records when it was created
Amends Spec 15 §1 (Aaron, board task, 2026-10-14). A task gets `createdTime` (seconds) when it is created; edits, moves between projects, bringing it back and archiving never change it. A task saved before this gets it when it loads: the earlier of its first note's time and its first run's start, or else its file's modified time. A strategy run's task brief carries it, and the Strategy page's Tasks tab shows it in a **Created** column.
Why: the human wanted to see when each task was made.

## D-131 A project declares its build in `solace.toml`'s `[build]`
Amends Spec 14 §2 and Spec 15 §4 (Aaron, board task, 2026-10-14). First task of the Build tab feature.
1. A project's `solace.toml` may hold a `[build]` table, its recipe: `steps`, an array of at least one `{name, run}` or `{name, command}`, run in order; `after`, the projects built before this one, as the root lists them; `outputs`, artifact paths in the project. A `command` step reuses that entry of `[commands]`, so today's `build-release` commands are named, not copied. A step may set `timeout` (seconds); otherwise it takes its command's, else 120 s, as `run` does.
   ```toml
   [build]
   after = ["solace-cli/board"]
   outputs = ["priv/app.js"]
   steps = [
     { name = "install", run = ["bun", "install"] },
     { name = "bundle",  command = "bundle" },
   ]
   ```
2. `E6020` names the field at fault (`build.steps[1].name`): an unknown field; a step without a name, with a repeated name, with neither or both of `run` and `command`, with an empty `run`, a `command` that `[commands]` doesn't have, or a timeout that isn't positive; `after` and `outputs` that aren't paths inside the root and project. Every command checks the shape. Whether `after` names other projects of the workspace is checked when an agent proposes the file and when the plan is read, since the file alone doesn't know the workspace.
3. A project with no `[build]` gets a default from its files, in this order: `package.json` → install (`bun install` with a `bun.lock`, else `npm ci` with a `package-lock.json`, else `npm install`); `gleam.toml` → `gleam build --target <its target>`; a `build` script in `package.json` → run it with the same tool; `Cargo.toml` → `cargo build --release --locked`. A project with none of these has no recipe and is skipped. A default has no `after` and no `outputs`.
4. The plan is every project's recipe, each after the projects its `after` names, else in the root's order. A cycle is refused with `E6020`, naming it (`a → b → a`).
5. `build plan` (CLI) and `GET /build/plan` (root only) give the plan, each recipe marked declared or default. `build plan` only reads: it is no change, verification or process, and works in read-only runs. `build` with any other argument is refused.
6. Not yet moved (D-159 moved them): the frontend projects' `build-release` commands and `solace-cli/build.rs` stay as they are. Their default (`bun install` then `gleam build`) is not their release build, which also bundles `priv/app.js`; the self-build feature declares their `[build]`. Solace's own Rust crate is in `solace-cli/`, which is not a project, so it has no default; the root will declare it.
Why: the Build tab must run any project, so each project needs one place that says how it builds, readable by the page.

## D-132 The Strategy page plans tasks and puts them back on hold; a task with a feature stays with its feature
Amends D-108, D-112 point 3 and D-128 (Aaron, board task, 2026-10-14). Part of the Backlog-off-the-board change.
1. `to_backlog` is refused (`E7001`) for a task with a feature, in any column: Planning and Canceled alike. It is not among the actions the server lists as allowed for such a task. A canceled task with a feature comes back through its feature, such as a strategy proposal; one with no feature comes back as D-108 says.
2. `<solace-task>` takes a `strategy` attribute, which the Strategy page sets. Back to backlog shows from a non-final column (Planning) only with it, and never for a task with a feature; from Canceled it shows on any page for a task with no feature.
3. The Strategy page's Tasks tab row shows **Plan** when its project's workflow offers `plan` in the row's column; a task with no project (D-129) has none. It sends the board's actions API, as the task's details do, which also offer Plan.
4. An active feature plans its tasks: when a feature becomes `active` by a human edit or an approved proposal, each of its tasks in `backlog` moves to `planning`, with a `solace` note naming the feature. A task linked to, or proposed in, a feature that is already `active` moves the same way. A state the feature's tasks set (D-128 point 3) moves none. Columns are found by id: a workflow without both `backlog` and `planning` is left alone.
5. A feature's Tech Debt task (D-112 point 3) is created in `planning`, or in the first column of a workflow without one.
Why: once the board no longer shows Backlog, the Strategy page is where a task goes active or back on hold; a feature's tasks follow their feature (D-128). Backlog is a status meaning *not active*, so an active feature's tasks belong in Planning, the board's first column.

## D-133 The board leaves out Backlog; a task goes back to planning
Amends D-108 points 2 and 3, D-128 point 4, D-132 points 1 and 2, and Spec 15 §1 and §3 (Aaron, board task, 2026-10-14). Part of the Backlog-off-the-board change.
1. `backlog` stays a task status, meaning *not active*. The board draws no `backlog` column and shows none of its tasks, with or without a feature, so `planning` is its first column. The Strategy page's Tasks tab lists every task, Backlog ones included, and plans them (D-132).
2. D-128 point 4 still holds in every column: a task of a `proposed`, `upcoming` or `canceled` feature is off the board.
3. New action `to_planning`, labeled "Back to planning", listed on `assigned`, `needs_review` and `cancelled` in the default workflow. It replaces `cancelled`'s `to_backlog`; `complete` lists none, so a completed task stays as it is. `to_backlog` stays on `planning` for the Strategy page; the board's details don't show it there (D-132 point 2).
4. It is offered to every task, with or without a feature. It keeps the task's changes and clears `approved`, as `reassign` does. It is refused (`E7001`) while any run of the task is open, running or waiting: the human stops it first. From `cancelled` it brings the task back as D-108 point 4 says.
5. A canceled task with a feature also comes back through Back to planning, not only through its feature as D-132 point 1 says (Aaron, Tech Debt task, 2026-10-14).
Why: the human wanted a task with no feature to become active without giving it a feature. Backlog means *not active*, so the board, which shows active work, leaves it out; a task that was moving on comes back to Planning rather than to a hidden column.

## D-134 An active feature's tasks are never in Backlog
Amends D-132 point 4 and Spec 15 §1 (Aaron, board task, 2026-10-14). Part of the Backlog-off-the-board change.
1. A task whose feature is `active` is never in `backlog`. When the server starts, and whenever a task or a feature is saved, such a task moves to `planning` with a `solace` note naming the feature, as D-132 point 4 does. This covers tasks already stored, and a task whose own save makes its feature `active` again (D-128 point 3). An approved task proposal that sets a complete feature back to `upcoming` sets that state before it writes the task, so that task stays in Backlog.
2. Columns are found by id: a workflow without both `backlog` and `planning` is left alone. A task with no feature, or whose feature is not `active`, is untouched.
3. D-132 point 1 still holds: Back to backlog is refused for a task with a feature.
Why: Backlog is off the board (D-133), so tasks stored before their feature went active, or linked to it some other way, stayed invisible. Planning by state, not only by the feature's change, moves them without a click.

## D-135 An interrupted extract is always undone
Amends D-103 and D-77 point 4 for extract records, and Spec 3 §4 (Aaron, board task, 2026-10-14).
1. An extract into another project is written ahead: a record in each of its projects before any file changes. If it doesn't finish, the next command, revert or cancel in any of those projects undoes it in all of them, as if it never ran. Solace never finishes a stopped extract and never holds one for a human; `E7029` is removed.
2. Undo puts back only what the extract wrote. Files edited outside Solace while it was stopped are kept, and read in as any outside change (D-58).
3. The task gets a note: the extract was interrupted and undone, run it again if still needed, and which files were kept.
4. When another command holds one of the projects, recovery refuses with `E7028` (try again), as before.
5. Files the extract created are deleted only by its rollback; the general cleanup of written files skips them.
Why: the human decided (task 6c746fe1f9058f87, note 18) that a half-done extract must never be finished or left frozen; undoing it is simple to understand and always safe to retry.

## D-136 Undo keeps folders that were there before the task
Amends D-103 (Aaron, board task, 2026-10-14).
1. Undoing a task leaves alone any folder that existed before the task, even an empty one. Removing a file the task added still removes the folders that leaves empty, but only those the task made, and never the top folder (`src/`, `test/`).
2. The saved before-state of a file that didn't exist becomes `{"kept": "<folder>"}`: the nearest folder above it that existed before the command (`""` is the project's own folder), taken from the project's folders listed before each command. Undo stops removing folders there.
3. An old record (`null`), or a change made outside Solace, has no folder entry; its undo removes every folder left empty, as before.
Why: the human agreed (task 6c746fe1f9058f87, note 18) that undo must not remove what the task didn't make.

## D-137 Undo keeps folders for module files and after a crash too
Amends D-136 (Aaron, board task, 2026-10-14).
1. D-136 covers Gleam module files as well as plain files: a module the task added is saved before as `{"kept": "<folder>"}`, and undo stops removing the folders its file leaves empty there.
2. An undo record written before a change (a file's, or a declared run's) carries the folders as they were, so a change recovered after a crash keeps D-136 too.
3. A folder the project ignores (such as `target/`) is listed as there but not looked inside: undo stops at it.
Why: the human answered (this task's question) that D-136 should apply to module files too; the rest closes gaps the reviews found.

## D-138 A run's brief names the task's feature and its latest direction
Amends D-91, D-110 and Spec 15 §6 (Aaron, board task, 2026-10-14).
1. A task with a feature gets a **Feature** section in its brief, after the task and before the vision: the feature's title, then the most recent of the task's investigation report (the agent's note from an investigate run), assignment note (the human's note outside any run) and review feedback (the agent's note from a review run), labeled by kind.
2. The note is cut at a line end within 1,000 characters (as the vision, D-91), ending "(cut short; the whole note is in the task's notes on the board)". With none of the three notes, the section is the title alone. A task with no feature, or whose feature is gone, gets no section.
3. Every run kind gets it (work, investigate, review, discuss), and so does `session start --task` in a board run, beside Depends on (D-110). The feature's body, milestone and other tasks are not inlined.
4. Like D-110, this is task data, outside D-102 point 5's 3,600-byte reference cap.
Why: constraints written for a feature never reached the agent doing the work. The human chose the latest direction note over measuring feature sizes and past runs.

## D-139 The layout audit runs in reviews; the root's `solace.toml` holds commands too
Amends D-38 and D-71 (Aaron, approved on board task "ui: Layout audit command", 2026-10-14).
1. The root `layout_audit` command (`solace-cli/tests/layout_audit.py`) unsets `SOLACE_TASK`, `SOLACE_RUN`, `SOLACE_SESSION`, `SOLACE_CONFIG` and `SOLACE_BIN` as `rust-acceptance` does (D-71), takes an argument (one page), and is allowed in reviews. Like `rust-acceptance`, it runs with normal host permissions: it runs `cargo build` (unless `SOLACE_TEST_BIN` names a binary), starts a fixture `solace serve`, and drives a headless Chromium through Playwright. It writes only its temp fixture and the screenshots under `solace-cli/target/layout-audit/`.
2. D-38's "the root's `solace.toml`, which only lists projects" is out of date: the root's `solace.toml` also holds `[commands]` (and its other settings). When a task's own project is the root (`.`), the root's `solace.toml` is the task's own file, and a work run proposes changes to it the D-38 way. A task in another project still never proposes to the root's file.
Why: the human approved `layout_audit` with `allowed_in_reviews = true` so reviewers can compare measurements, and approved its line in the root's `solace.toml`; D-38's wording could make an agent refuse that proposal.

## D-140 The guide is the shell's Knowledgebase page
(Aaron, board task "knowledgebase: page with the Guide", 2026-10-15). The first piece of milestone:208dbeb9095a5a4b.
1. The guide moved from `board/guide.html` into the frontend's `knowledgebase` module, shown by a **Knowledgebase** menu tab (after Contracts, before Settings) at `/knowledgebase`. It is drawn in the shell's light DOM, with the shell's fonts and light and dark theme, and its section headings form a table of contents at the top.
2. `/guide` answers with a permanent redirect (301) to `/knowledgebase`, so old links still work; the shell also maps `/guide` to the page and replaces it in the history.
3. The board's header loses its Guide link.
4. A later Knowledgebase task may turn the page into its own app.
Why: the guide had no menu, its own styles and no dark mode; inside the shell it reads as part of the app.

## D-141 One strict Add form; whole-patch faults are resent
Amends D-47, D-104 and D-120, and Spec 3 (Aaron, board task "submit: new modules without a syntax spiral", 2026-10-15).
1. Every line of a patch's `*** Add:` and of `write`'s `*** Add File:`, empty ones too, starts with `+`, as before. A lenient Add (plain code when no line has `+`, blank lines read as empty) was considered and rejected: one way to write it, no undocumented behavior, and the `apply_patch` form agents already lean toward.
2. The one documented way to create a module is a patch `*** Add: <module>` (or `*** Add: <module>.<name>` sections) with its module comment, imports and code; the brief says so in step 2. `E1003` for an unknown module (`submit --imports <module>`) points to it. `submit --new` and `submit <module>` still work and are not promoted.
3. An `E0108` whose fault is in the whole patch, not in one line, says to send the corrected patch again whole, and its draft's footer leaves out `--replace`: an Add with no `+` line, no section, a line before the first header (blank or not), an unknown `***` header, an Add that defines nothing, a file section sent to `submit`, a patch that changes nothing. A one-line fault names its line and keeps the `--replace` advice; an Add line missing its `+` says every line, empty ones too, starts with `+`.
Why: run 8b3697864d7244e0 spent 10 of its first 27 calls creating one module. The strict rule cost one call; the `--replace` advice that followed cost the rest. The human chose consistency and one way over leniency.

## D-142 A patch line's leading space is part of it
Amends D-120, and Spec 3 (Aaron, board task "submit: new modules without a syntax spiral", 2026-10-15).
1. A hunk line matches the target's line exactly, or with trailing space ignored. The third level, trimming both sides, is dropped: it matched a line the patch only resembled, and the splice rewrote that line with the patch's own indentation, so a patch could re-indent a line it merely quoted.
2. When a hunk doesn't match, `E0108` names the first hunk line with no exact match in the target; when the target has that line only with other leading space, it names the line and how many leading spaces it has, so the patch copies it whole. When every hunk line matches somewhere in the target, `E0108` says the lines aren't adjacent in the file.
Why: a reworded brief paragraph was written one space right of its block, and probes built on the trimming level reported the file as aligned, which cost two review rounds. Leading indentation is the kind of change a patch should not make silently.

## D-143 The reference's layout is checked
(Aaron, board task "reference: layout validator", 2026-10-15).
1. `solace-cli/tests/reference_layout.py <file>` checks the columns of the reference's "Commands in full" section. It is the root's validator for `solace-cli/src/reference.txt`, so a `write` that misaligns a line is refused, naming the line and column. A `main.rs` unit test runs it on the embedded reference, so `rust-unit` catches a change made some other way.
2. A command row's same-line description starts at the description column, the one most rows use. Rows next to each other may share another column (a table, like the file tiers); a row whose term fills that column with one space takes its neighbors'. A continuation line starts where its row's description starts. Lines two or more columns deeper than that are an example block only right after a line ending in `:`, and the whole block shares one column; one column deeper is always a slip. Headings, prose and blank lines are left alone.
3. No width limit: the section's widest lines are quoted tool output, so its widest line would limit nothing (the human dropped it).
Why: in task e86828d20aaab5a2 one leading space cost six runs. `read`, `grep` and `write`'s `old` don't show leading space, so only a check can catch it.

## D-144 `help <command>` answers one section
(Aaron, board task "help: one command's section only", 2026-10-15).
1. Amends D-46 point 4: `help <command>` answers the paragraph holding the command's entry — a row indented by exactly two naming it — not every paragraph that mentions the word. A command with no entry answers `No command` and the run's command names, not the whole reference.
2. The reference gives every summary-table command its own entry, and `build`, `format`, `run` and `work` their own paragraphs.
3. Amends D-143: the layout check names a summary-table command with no entry in "Commands in full".
Why: `help work` printed nearly the whole reference: a common word matched every paragraph.

## D-145 Batching guidance where the agent plans
(Aaron, board task "batching: guidance at session start and after single reads", 2026-10-15).
1. Every session start reply (symbol index, `--task`, plain files) ends with a `Next:` line naming a first call to send: with `--task`, `inspect` of the agent's project's top three ranked symbols with `--context`; otherwise the reference's example `inspect`, or `grep` and `read` in a project of plain files. Strategy runs get no such line: they get no workflow.
2. Over MCP, from the second call in a row that held one read and succeeded, the reply ends `Hint: put independent reads in one call, one per line.` Any other call (a batch, a failure, a change, `help`) starts the count again. A failed call never gets it, so D-109's last line is unchanged, and the hint is no failure.
3. A failed probe whose compiler output says `Unknown module value` adds the line "Hint: print a value with `echo value`." after that output (the human's note).
Why: the tool description's "never one read per call" didn't change behavior: agents still sent about one command per call, though run 27 sent 82 commands in 11 calls. A small model acts on the end of a reply and on the reply to its mistake.

## D-146 The Migration page is the Workspace page, at `/workspace`
Amends D-62 (Aaron, board task "workspace: rename Migration page", 2026-10-16). Part of the Workspace page feature.
1. The **Migration** menu tab and page become **Workspace**, at `/workspace`. The menu follows a user's journey through a project: Workspace, Strategy, Board, Contracts, Knowledgebase, Settings.
2. The page's heading is **Workspace**; migration is one section of it, "Bring in an existing project". The `solace-cli/migration` package and its `<solace-migration>` tag keep their names.
3. `/migrate` still serves the shell, which shows the Workspace page and replaces `/migrate` with `/workspace` in the history. `/migration` stays the JSON API.
4. The address is not `/projects`: that is the projects JSON API, and a page address can't share a path with an API.
Why: the page is growing into the place to manage the workspace and its projects, of which bringing in existing code is one part.

## D-147 The human never edits files
Amends D-38 (Aaron, board task "agents: never ask the human to edit files", 2026-10-16). Part of the Improved Agent Integration feature.
1. The human never edits files or runs commands. A needed change is made by the agent with `submit` or `write`; a settings file (`solace.toml`, `AGENTS.md`, `.gitignore`) with `write` and `reason`, which the human approves (D-38); a change in a project outside the task's scope after `task ask` has the human add the project. A question never asks the human to change a file.
2. D-38's "report the exact edit instead" in read-only runs means reporting it for whoever does the work: an investigation's plan assigns every file change to the worker, never to the human.
3. `gleam.toml` is a watched file changed with `write` (validated, recorded with the task); it can't be moved or removed.
Why: in task c0795803428e089d the reference's "(not gleam.toml)" read as "can't be written", and an investigation planned for the human to edit it.

## D-148 Records are named as links in agent text
(Aaron, board task "strategy: refer to records as links", 2026-10-16). Part of the Improved Agent Integration feature.
1. The strategy brief and the reference tell agents to name tasks, features and milestones in notes and proposals as `task:<id>`, `feature:<id>` or `milestone:<id>`, written bare and never inside backticks: a bare reference renders as a link with the record's title (D-98, D-122), while a code span stays literal.
2. An approved create answers `created task:<id>`, and the same for a feature and a milestone, so the agent copies the link form from its own reply. `strategy show`'s `links` stay plain ids: they are data.
Why: strategy agents wrote bare ids and code spans, which a human can't follow; the link form already renders, so the agent only needs to write it.

## D-149 Every native model has a default effort
Amends Spec 15 §1 and §6 (Aaron, board task "agents: a default effort for every model", 2026-10-16). Part of the Improved Agent Integration feature.
1. A model's default `effort` is the one its `[[models]]` entry or its agent names (Codex's `defaultReasoningEffort`), else `medium` when the model offers it, else the middle of the efforts it offers. Custom runners and models offering no effort have none. Endpoint models count as native.
2. A run with no chosen effort passes and records that default, so a run's effort is never blank when the model offers efforts.
Why: Claude's models endpoint names no default, so Claude runs left the effort to Claude Code and showed only "Default".

## D-150 A model shows by one name; configured models join their family
(Aaron, board task "Improved Agent Integration: Tech Debt", 2026-10-16). Part of the Improved Agent Integration feature.
1. The model pick shows each model by its label alone, the model's own name ("Opus 5.5", "GPT-6 Sol"): no agent name and no model string beside it. Presets and discovered models are labeled that way; an alias reads "Opus (latest)" until a run names the model it used, then that model's label, or its id when no entry runs it.
2. A configured `[[models]]` entry whose `claude-` id names a family (`claude-opus-local`) belongs to that family, so the pick's one entry per family may stand in front of it; one whose id names no family keeps its own entry.
3. A remembered pick the list no longer shows that is a Claude alias (`claude-opus`) becomes its family's listed model rather than the first one.
Why: the pick read "Claude Code · Opus 5.5 · claude-opus-5-5", three names for one model; the label is the one name every entry has (a custom runner has no model string).

## D-151 A fresh run's brief holds the session start reply
Amends D-145 point 1 and Spec 15 §6 (Aaron, board task "briefs: session start in the brief", 2026-10-16). Part of the Agent Tooling Efficiency feature.
1. For a fresh work or investigate run, the board runs `session start --task` with the task's title and description in the project the run works in, and puts the reply at the end of the brief in place of the line asking the agent to run it. The agent starts with the call the reply's `Next:` line names. Resumed, review and discussion briefs are unchanged; a folder with no store keeps the line.
2. `session start` keeps no state, so the agent's own process needs nothing from the board's call: the session functions take the run's task id rather than reading it from the environment. The brief's reply leaves out the feature and dependencies, which the brief has already.
3. Every session start shows open work records whole, within 4,000 bytes, and says how many it left out; `work list` shows them all.
4. The tool description says a board run's brief holds the reply and that `session start --task` gives it again, for example after the conversation is compacted. The board doesn't send it again after compaction, which happens inside the agent.
Why: an agent's first turn was always the same call; the reply in the brief saves it. One task's open work records alone were over 10k characters.

## D-152 A batched task done waits for a passing test
Amends D-119 point 5 and Spec 3 §1 (Aaron, board task "batch: task done waits for a passing test", 2026-10-16). Part of the Agent Tooling Efficiency feature.
1. In a board run, a passing `test` names the task's changed symbols in its project that no test reaches: `No test reaches <names>.`
2. In a call, a `task done` after a `test` is skipped when that test fails or names such a change. The reply names it with the reason, `task done (the test failed)` or `task done (no test reaches <names>)`, among D-119's skipped commands, or as `Skipped: <commands>` when nothing failed, and asks the agent to fix what the test reports and send `task done` again with an updated summary. An untested change alone makes no error.
3. A clean pass finishes the task in the same call.
4. The summary is written before the result is known, so the reference says to send `task done` in a call of its own when the result may need a caveat.
Why: `submit`, `test` and `task done` in one call saves a turn per work run, but `test` isn't a change, so a `task done` ran even after a failing test; a pass with untested changes is also not done.

## D-153 Questions show example outcomes
Amends D-66 (Aaron, board task "agents: questions show example outcomes", 2026-10-16). Part of the Improved Agent Integration feature.
1. A question's `text` gives each option a short example of the outcome the human would see: what a user sees, what a command prints, what a page shows, what a file or record ends up as; not how the code would change. The option's name is bold in the example; the `options` stay short labels matching the bold names.
2. The work and investigate briefs' asking guidance, the message guidance every brief shares (so discussion runs asking in notes too), and the reference's `task ask` entry say this.
Why: D-66 explains why agents ask, but the human still had to picture what each option would do.

## D-154 A feature starts from its card
(Aaron, board task "Start feature button", 2026-10-16). Part of the General UI Polish feature.
1. On the Strategy page's Features tab, a `proposed` or `upcoming` feature's card has a **Start** button. It sends one `PATCH /features/{id} {state: "active"}`; the server does the rest, as for any edit to `active`: it keeps the start time and color (D-114) and plans the feature's Backlog tasks (D-134).
2. A feature in any other state (active, complete, canceled) has no Start button: it is left out, not disabled.
Why: starting a feature took opening its form and picking the state; it is the step that sets the feature's work going, so it gets a button of its own.

## D-155 A card shows its feature's name on a line of its own
Amends D-114 point 3 (Aaron, board task "Feature on task card", 2026-10-17). Part of the General UI Polish feature.
1. A board card shows its feature's name, with no "Feature" prefix, on its own line under the title. The line is visible text, so screen readers read it too.
2. The card's tooltip keeps "Feature: <name>".
Why: the feature was only in the tooltip and screen-reader text, so it wasn't evident on the card; on its own line the name needs no prefix.

## D-156 Briefs ask for the smallest complete change, per role
(Aaron, board task on efficient, role-specific briefs, 2026-10-17). Part of the Agent Tooling Efficiency feature.
1. The embedded agent reference's numbered workflow asks for the smallest complete change: reuse the code's helpers, then the standard library and installed dependencies; add nothing the task doesn't need; complete includes its tests and every caller; never drop validation, data-loss handling or security; fix a reported bug once, at its cause. It also says `test` doesn't reach docs, config, JavaScript or golden files.
2. Each role gets only what it acts on. A strategy run proposes the smallest version that meets the need, names what it left out, and says once if part of a request isn't justified. An investigate plan is the smallest complete change, names every module or symbol it changes and every non-source file it reaches, offers a smaller option with `task ask` when it would change what the human gets, and reports what it didn't check. A work summary names, if any, what was skipped and the risks. A review flags code the task didn't need, places it missed, and removed validation, data-loss handling or security. Discussion briefs don't change.
Why: asking for the smallest complete change and giving each role only what it acts on makes agents more efficient without losing quality; these practices filled gaps in Solace, worded so they don't contradict step 3 ("fix only what the reply reports"), test coverage or the rule that specs change only through a decision.

## D-157 A re-read of an unchanged symbol brings back the run's recent work
Amends Spec 3 §1 (Aaron, board task "replies: catch re-reads after compaction", 2026-10-17). Part of the Agent Tooling Efficiency feature.
1. In a task run, the MCP server keeps the run's reads: each symbol it printed in full (`view`, `inspect`), with its project and the revision served. It saves them on the run in the task store, through the inbox, so a resumed run goes on from them. The run log records steps but no revisions, so it can't serve.
2. A call that prints a symbol again at the revision the run was served means the conversation was most likely compacted. Once per window, the reply adds the current views of the window's other symbols, within the reply budget (D-14), then the run's steps so far, one line each (D-99), the latest that fit.
3. A window runs from one change command (`submit`, `delete`, `extract`, `merge`, `write`, `remove`, `move`) to the next. A change starts a new one that keeps only the symbols it made differ from what was served, so the catch adds little right after compaction.
4. A re-read of a symbol changed since it was served adds nothing. Runs outside a task and strategy runs keep no reads.
5. It is one turn late: the first call after compaction is planned from the summary alone, so only the re-reads after it are saved.
6. `refs` shows references, not a symbol in full, so it records nothing. A change to symbols the run never read starts no window. A store that won't open is no change, and views the reply can't fit are named.
7. Amends points 2, 4 and 5 (task:dc959b53b88e089f): no detection. After a compaction the agent asks with `session remember` (task, notes, feature, dependencies; the window; the outline; within D-14; repeatable; refused in a discussion with E7010).
Why: run 11 made four separate re-reads after compaction. The server can't see the agent compact, and the guess came one turn late; the agent can see it and ask.

## D-158 Rust projects are held by symbol
Amends Spec 1 §4, §8 and its tests, Spec 2 §1, §2 and its tests, and Spec 14 §1, §4, §7 and its tests (Aaron, board task "Rust: record the store decision", 2026-10-17). Part of the Rust symbol store and lockdown feature; proposed in task:d41626ab51473623, measured in task:4f32c653e33b9caf.
1. A Cargo package of one crate is a Rust project. Its `.rs` files in `src/` and `tests/` are symbols: items by module path (`board::agents::cached`), methods as `Type::name`, trait impls as `<Type as Trait>` (their functions `<Type as Trait>::f`), inline `mod` blocks as modules.
2. The subset: every item has its own name, once per module (platform differences go inside the body); at module level a macro only defines (`macro_rules!`) or is `thread_local!`; no `include!` (`include_str!` and `include_bytes!` are data); a `#[path]` module is a `.rs` file beside its parent. Every `unsafe` has a `// SAFETY:` comment: a review rule, not a refusal; the review lists new `unsafe`. `submit` refuses code outside the subset, keeping a draft (D-46) and giving the rewrite:
   - same name twice: "Not saved: `f` is defined twice in `board` (`#[cfg(unix)]`, `#[cfg(windows)]`). Keep one `f` and put the platform code inside it: `#[cfg(unix)] { … }` or `if cfg!(unix) { … }`."
   - item-making macro: "Not saved: `lazy_static! { … }` at module level makes items Solace can't hold as symbols. Write them out: `static X: LazyLock<T> = LazyLock::new(|| …);`."
   - `include!`: "Not saved: `include!("x.rs")` pastes in code Solace doesn't hold. Make it a module (`mod x;`); for data, use `include_str!("x.txt")`."
   - `#[path]` elsewhere: "Not saved: `#[path = "../x.rs"]` is outside its parent's folder. A `#[path]` module is a `.rs` file beside its parent: `#[path = "board_tests.rs"] mod tests;`."
   - at import: "`x` is a proc-macro crate; Solace doesn't hold proc-macro crates yet." and "This is a workspace of N crates; add each member as its own project."
   A Rust project's session start, and no other, carries the brief lines: "Rust in Solace: every item has its own name, once per module; put platform differences inside the body (`#[cfg(unix)] { … }` or `if cfg!(unix)`). At module level a macro only defines (`macro_rules!`) or is `thread_local!`: no `lazy_static!`, `bitflags!` or other item-making calls (use `LazyLock`, `OnceLock`, consts). No `include!`; use `include_str!` or `include_bytes!` for data. `#[path]` names a file beside its parent. Every `unsafe` has a `// SAFETY:` comment. `build.rs`, `.cargo/config.toml`, `rust-toolchain.toml` and Cargo.toml's dependencies are settings: `write` them with `reason`."
3. Every change is checked with `cargo check --all-targets --message-format=json`; a change with errors is not saved. Errors are located by their primary span, and an error reported once per target is shown once. A warm check after a one-function edit took 0.71 s and a cold one 10.4 s, so every submit is checked, in the project's own target folder. `cargo test` (26–28 s, mostly running the suite) stays at `test` and the existing gates, never per submit.
4. `build.rs`, `.cargo/config.toml`, `rust-toolchain.toml` and Cargo.toml's dependency and build keys (the dependency tables, `[target.*]` dependencies, `[patch]`, `[replace]`, `package.build`, `package.links`, `lib.proc-macro`) are settings the human approves. On Linux, checks run under bubblewrap without network, with only `target/` writable; so do Solace's `test` and `build`, which run `build.rs` too. Checks pass `--offline`; `~/.cargo/registry`, `~/.cargo/git` and the toolchain are mounted read-only; `cargo fetch` runs outside the sandbox only after an import and after an approved change to Cargo.toml's dependencies.
5. Items are parsed with `syn`, and files are written from stored text exactly. rustfmt formats a submitted item only: the code isn't rustfmt-clean (about 295 hunks across 21 files with its `rustfmt.toml`), so items not submitted keep their text.
6. Deferred: workspaces, proc-macro crates, type-accurate refs, extract and merge, probe.
Why: Rust code is kept as plain files today, so agents edit it by file with no type check per change. A warm check is under a second, so checking every change is affordable; formatting whole files would bury a change in unrelated reformatting.

## D-159 The frontend bundles build in the plan; `build.rs` only embeds
Amends D-131 point 6 (Aaron, board task "Self-build: bundling out of build.rs", 2026-10-17). Part of the Rust symbol store and lockdown feature.
1. `solace-cli/frontend` declares `[build]`: one step `release`, its `build-release` command (timeout 600 s), with `outputs = ["priv/app.js"]`. The board's `priv/app.js` serves its standalone page and is built by its `build-release` command, not by the plan: the crate embeds the frontend's bundle, which compiles the board's Gleam as a path dependency.
2. The root declares Solace's crate: one step `cargo`, `cargo build --release --locked --manifest-path solace-cli/Cargo.toml` (timeout 900 s), after `solace-cli/frontend`.
3. `solace-cli/build.rs` runs nothing: it embeds `frontend/priv/app.js`, watches the frontend and the projects `frontend/gleam.toml` lists as path dependencies (their `src`, `gleam.toml`, `package.json` and `bun.lock`; a dependency path is read from the frontend, `..` going up one folder, and a part a project doesn't have yet is watched through its folder, which cargo scans recursively), stops with "solace-cli/frontend/priv/app.js is missing: run the [build] step `release` in solace-cli/frontend (`build plan` lists it)" when the bundle is missing, and warns with the same step when it is older than its sources: a Rust check must not depend on another project's state, and file times misfire after a `git checkout` or `pull`. cargo shows that warning only on a build where it reruns the script, and `condense` keeps it whole (D-111), so it can't be relied on; point 4 is where staleness is read.
4. `build plan` marks a declared output out of date, naming the steps to run: "out of date: solace-cli/board/src changed after priv/app.js was built: run step `release`", or "priv/app.js is missing". A project's outputs are built from its own `src`, `gleam.toml`, `package.json` and `bun.lock` and the same files of each path dependency its `gleam.toml` lists, worked out by Solace, not declared. A source newer than the oldest output by more than 5 s marks it, the same tolerance as `build.rs`; stored hashes of the sources can replace file times once Solace runs the plan. Aaron chose the plan over `-vv` in `run` (build-script noise on every build) and over leaving it to the Build tab.
Why: a Rust check ran bun and gleam in other projects, so it failed on their Gleam errors and needed write access to them; the plan already says how each project builds.

## D-160 A task whose home is no project is re-homed
Amends D-129 point 2, and Spec 15 §4 (Aaron, board task "Homeless tasks fall back to the root", 2026-10-17).
1. When a task's home is no project of the workspace, it is re-homed: its home becomes the first project of its scope, and its scope keeps only the projects of it.
2. When none of its scope folders is a project, its home becomes the root and its scope becomes `.`, when the root is a project. A task stays without a project only when the root itself isn't a project.
3. The load (`GET /tasks` and `GET /tasks/{id}`) applies this and writes the change: the task's folder moves as a scope change moves it, and the task gets a `solace` note naming its old and new home and the folders it dropped. The `noProject` mark comes from the home on each load and is never saved to the file.
4. A task with a run still live is not re-homed until the run ends: the agent works from the task's folder, and the load leaves it marked.
5. A project list that can't be read counts every home as a project, so no task is marked or moved while the list is unreadable.
Why: task:59d0b7ae77d161d9 showed as having no project although its scope named projects: a change on such a task saved the mark into its file, where it stayed whatever the home. A task with no project is a dead end the human can't fix, and the root is where its runs wrote, so it falls back to the root. The scope drops its non-project folders (Aaron's choice) so the Scope tab of a re-homed task shows only projects and saves.

## D-161 Every read-only run may run marked commands
Amends D-67 (Aaron, board task "Address failing tests", 2026-10-17). Investigations and discussions, like reviews, may `run` a `[commands]` entry marked `allowed_in_reviews = true` in that project's `solace.toml`; unmarked commands stay refused, with "this run is read-only". The key keeps its name. Why: an investigation asked about failing tests couldn't run the suites in question, and D-67 gave no reason to leave investigations out. A command a human vetted for reviews is just as safe there; commands that change project files, such as a bundle step, stay unmarked.

## D-162 `view` reads Rust files with the `syn` splitter
Amends D-121 points 1, 2 and 4 (Aaron, board task "Rust: syn-based splitter", 2026-10-17). Part of the Rust symbol store and lockdown feature.
1. `view` of a `.rs` file uses the splitter of D-158 point 5 (`syn` 3, the version already in `Cargo.lock`), not the line scanner, so Rust has one parser. Python keeps its scanner.
2. Names follow D-158 point 1: a trait impl is `<Type as Trait>` and its functions `<Type as Trait>::f`; an inherent impl keeps its generics (`Store<T>::open`).
3. A file `syn` can't parse gets no outline: the reply names the line and the error, and `read <file>`.
Why: the store and `view` must agree on what an item is and what it is called.

## D-163 `build plan` alone reports a stale bundle; `run` replies show elapsed time
Amends D-159 points 3 and 4, and Spec 14 §4 (Aaron, board task "Rust symbol store and lockdown: Tech Debt", 2026-10-17). Part of the Rust symbol store and lockdown feature.
1. `solace-cli/build.rs` stops when the bundle is missing and watches its sources; it no longer warns when the bundle is older than them. `build plan` is the one place staleness is reported, and `GET /build/plan` carries the same mark for each project (`stale`). A part a project doesn't have yet is watched through the files beside it, not its folder, so a change in a project's `build/` folder doesn't rerun the script.
2. Every `run` reply starts with the exit code and the elapsed time: `exit code: 0 (4.2s)`. The root's measurement wrappers (`cargo-check-time`, `cargo-check-time-cold`, `cargo-test-time`, `cargo-test-time-no-run`, `cargo-fmt-files`) are removed.
Why: staleness is a planning concern, not a compilation one, and two copies of the same check drift apart. With timing part of `run`, any command measures itself without a shell wrapper.

## D-164 Models and their keys are set up from Settings
Builds on the machine-wide settings in `config.toml` (Aaron, board task "Set up models and their keys from Settings", 2026-10-17).
1. The Settings page lists the configured `[[models]]` with why each can't start, and adds, edits and deletes endpoint models (label, agent, model id, base URL, API key) through `POST /models` and `DELETE /models/<id>`. Solace writes `config.toml` whole after checking it, so comments in it are not kept.
2. A pasted key is kept by model id in `keys.json` beside `config.toml`, mode 0600, for now on every system; never in `config.toml`, never sent back (`GET /models` says only `has_key`). A keychain (and, on Windows, the credential manager or a user-only ACL) can come later.
3. An entry saved from Settings without a `key_env` gets `stored_key = true`. At launch the stored key wins: Codex gets it as `SOLACE_ENDPOINT_KEY` (its `env_key`), Claude as `ANTHROPIC_AUTH_TOKEN`. `key_env` still works. An endpoint that names neither needs no key.
4. A model whose key is missing stays in the list: "No key yet; add one in Settings".
Why: the server starts from the desktop app and never sees shell variables, so an endpoint model could only be fixed by editing system environment files.

## D-165 A package of more than one crate is refused at import
Amends D-158 point 2 (Aaron, board task "Rust: import a one-crate package", 2026-10-17). Part of the Rust symbol store and lockdown feature.
1. A package of more than one crate (`src/lib.rs`, `src/main.rs` and each program in `src/bin/` are one each) is refused at import, naming its crates: "This package has 3 crates (src/lib.rs, src/main.rs, src/bin/tool.rs); Solace holds a package of one crate." A package whose one crate is a program in `src/bin/` is imported.
Why: D-158 holds one crate but gave no text for this case; naming the crates shows the user what to split.

## D-166 A recorded build compares source contents, not times
Amends D-159 points 3 and 4, and D-163 point 1 (Aaron, board task "Rust symbol store and lockdown: Tech Debt", 2026-10-18). Part of the Rust symbol store and lockdown feature.
1. `solace-cli/build.rs` watches only the bundle (`frontend/priv/app.js`) it embeds, so a source a project doesn't have yet, a new `package.json`, can't go unwatched. `build plan` is the one place staleness is reported (D-163).
2. When a passing `run <command>` is a recipe's last step and the recipe names outputs, Solace writes `.solace/built.json` in the project: the hash of each output, and of each source part (its own and its path dependencies' `src`, `gleam.toml`, `package.json` and `bun.lock`). `build plan` marks the parts whose hash changed, "`a/src` changed since `priv/app.js` was built"; a source touched without changing marks nothing.
3. File times are used only when there is no record, or when an output has changed since it was recorded (it was rebuilt some other way), the same 5-second rule as D-159 point 4.
Why: watching a missing part through the files beside it left a file added later unwatched, and file times misfire after a checkout or a pull; recorded hashes keep the plan honest while `build.rs` watches one file.

## D-167 Rust `check` and `test`; `submit` waits for its own task
Implements D-158 point 3, and splits its per-submit check out (Aaron, board task "Rust: check, test and missing cargo", 2026-10-18). Part of the Rust symbol store and lockdown feature.
1. On a Cargo package, `check` runs `cargo check --all-targets --message-format=json --offline` in the project's own `target/`, each message located by its primary span (out of a macro to its call site) and shown once; `test` runs `cargo test` and reads failures by libtest name. `rustfmt` formats one item at a time, a method through an `impl _ { … }` wrapper, never a whole file. An import starts a background check to warm `target/`, without making the import wait.
2. `submit` to a Rust project is task:ba1542d1a967b9b7, which also connects it to the check; until it lands, `submit` replies "Rust projects can't take `submit` yet".
3. A machine without `cargo`: `check`, `test` and `submit` are refused with "Needs the Rust toolchain (`cargo`)" (E6013); reads and every other command keep their own reply, and the project's lane on the board shows that message, carried by `GET /projects` as `needs` and drawn by `solace_board`. An undo in a Rust project is checked too, and stays unchecked, not refused, when the toolchain is missing.
Why: every Rust submit needs a working check first, and saving Rust items (`::` names, `impl` members, new items) is as much work again, so it is its own task. A check that found errors also says more may follow: borrows are checked only once types check.

## D-168 One `submit` for Rust and Gleam; Rust items save by name
Amends D-167 point 2 and D-158 point 6 (Aaron, board task "Rust: submit saves items", 2026-10-18). Part of the Rust symbol store and lockdown feature.
1. Agents work with a Rust store exactly as with a Gleam one: the same commands, forms and replies. A Cargo package takes `submit` and `drafts` (D-46 draft-keeping is language-neutral, so a refused submit keeps its draft); `submit` saves items by name, and every submit is type-checked with cargo before it is saved, so it needs `cargo` (E6013) like `check` and `test`. D-158 point 6's deferral of `submit` is lifted here.
2. An item inside a container is saved by wrapping it in that container's own opening text and `}`: `cargo::format_item`'s `nested: bool` becomes its `opening: Option<&str>`, the caller splits the wrapper and takes the one child out. It works alike for `impl`, `trait`, inline `mod` and `extern` parents. The stored name is the parent's name, `::`, the item's own, and a rename is allowed, as in Gleam.
3. The work is split across tasks: this one saves items (task:ba1542d1a967b9b7); reading Rust code by name and the session are task:ac5ae78fb2e94ab2; new containers are point 4; new modules (task:e6a34c9df7823031) and `delete` (task:87caaab35442ab65) were split out of task:5ce78f3fa5e3d808; a whole-module `submit <module>` is task:4f544f48f7b2f1c0; `refs`, `impact` and test reach are task:d7aa4d5e523a065c. Every command a later task owns is refused with its task's name.
4. A whole container is saved by `submit --new <module>` with its `impl`, `trait`, inline `mod` or `extern` text: it is stored nested (parent, ord, tail) the way an import stores one, so a trait impl or a type's first method lands in one submit, and adding an item to an empty `impl X {}` re-splits it inside its own opening text so its tail becomes the closing `}`. A block's own text is not an item: a named block (`submit <name>`, `*** Update: <container>`) is refused with "a new `impl` block goes through `submit --new <module>`; an existing one changes item by item", naming no task.
Why: one API for both languages is what makes a Rust store worth having, and saving an item is what makes its check (D-167) useful; the wrapper taken from the item's own parent is the one shape that fits every container Rust has.

## D-169 Rust integration tests are `integration::<file>`
Amends D-158 point 1 (Aaron, board task "Rust symbol store and lockdown: Tech Debt", 2026-10-18). Part of the Rust symbol store and lockdown feature.
1. A crate's `tests/*.rs` files are `integration::<file>`: `tests/acceptance.rs` is `integration::acceptance`, and a `mod x;` of one is `integration::x`, so the helper `tests/common/mod.rs` is `integration::common`. The crate's own unit-test module keeps its name `tests`, so the two never share a prefix. A library module that is itself called `integration` still clashes, and the existing "Two modules are named" refusal covers that.
Why: `tests::acceptance` read as a child of the crate's `tests` module, and a library module named `tests` with a child of the same name was refused at import although the two modules differ.

## D-170 Rust refs, impact and test reach are name-based
Amends D-158 point 6 and D-168 point 3 (Aaron, board task "Rust: refs, impact and test reach", 2026-10-18). Part of the Rust symbol store and lockdown feature.
1. A Rust store gets the same reference graph Gleam's has, built when the index is built from the names each item's text mentions (paths, bare names, method calls and macro calls, read from tokens so a call inside `assert!(…)` or `format!(…)` counts), resolved against the module's `use` lines (aliases, `{…}` groups, `self::`/`super::`/`crate::`, globs followed through the parent's imports) and the crate's item names. `refs`, `impact` and `inspect --context` answer Rust names in Gleam's reply format, and their refusal is removed. `refs` lists a function's callers.
2. Resolution overestimates on purpose, so test reach never misses a test: a method call links to every crate item with that name, a qualified path whose start doesn't resolve to every item with its last name, and a mention of a type to every item of its trait impls, so `Display::fmt`, `Drop` and `From` behind `?` are seen. Type-accurate refs stay deferred (D-158 point 6).
3. Test reach is one selection for both languages: `#[test]` functions are the tests; a symbol is test code under `#[cfg(test)]` or in `tests/`; results are recorded by libtest name (the module path with `crate` and a `tests/` crate's `integration::<file>` prefix dropped). `test` runs `cargo test … -- --exact <names>`, the whole suite when every test is due and a check when none is, and the reach hash covers `Cargo.toml`, `Cargo.lock` and `build.rs`.
Why: with the graph and test selection in place, an agent changes Rust code by symbol exactly as Gleam code, with no command that works in one language and not the other.

## D-171 Rust new modules come from `submit --new`
Amends D-168 point 3 (Aaron, board task "Rust: new modules via submit --new", 2026-10-18). Part of the Rust symbol store and lockdown feature.
1. `submit --new a::b::c` creates the module when the store doesn't have it: every `::` part an identifier and the parent module there (`cache` belongs to `crate`, `board::agents::cache` to `board::agents`), and the text's docs and `use` lines become the module's header. A patch's `*** Add: <module>` creates one the same way. `integration::x` creates the test crate root `tests/x.rs` (D-169), which no `mod x;` line names; any other new module's file follows its parent's file (`src/board/agents.rs` gives `src/board/agents/cache.rs`) and its parent's header takes `mod c;` after its last `mod` line. A name that is none of these, and a new module whose file is already there outside the store, are refused as in Gleam.
2. `mod x;` in submitted text is refused, naming `submit --new <module>` for adding a module and `delete` of its items for removing one. Creating a module is one transaction with the check that follows it, and undo removes the module's file, its `mod` line and its items.
Why: D-168 split new modules out of task:5ce78f3fa5e3d808, and agents add a Rust module as they add a Gleam one; the file and `mod` line follow Cargo's layout, so the package Solace projects builds with the normal compiler.

## D-172 Rust whole-module `submit` keeps ids by name
Amends D-168 point 3 (Aaron, board task "Rust: whole-module submit", 2026-10-18). Part of the Rust symbol store and lockdown feature.
1. `submit <module>` replaces a Rust module whole, as it does a Gleam one: the text is the module's file, every stored item must be there, and the header's `mod x;` list can't change (a module is added with `submit --new`). Each submitted item is matched to its stored row by name and namespace within the parent, so its id stays; two `impl Store` blocks pair by order; a stored row the text leaves out is refused with E0102, and an item the text adds is saved as `--new` saves it. Only the items whose text changed are formatted, with `cargo::format_item` inside the parent's opening when nested, so unchanged items keep their bytes (D-158 point 5); `ord`, `sep` and a container's `tail` follow the text. The module is checked with cargo before it is saved, and a module the store doesn't have is created by the `--new` path. A patch's `*** Update: <module>` applies its hunks to the module's text and saves each changed or added leaf item as a named block; the header goes through `submit --imports <module>`, a new container through `submit --new <module>`, and an existing container's own text through `submit <module>`.
Why: D-168 gives Rust Gleam's commands; without a whole-module submit the two languages would differ, and resubmitting a module a symbol at a time would lose its ids.

## D-173 Rust `delete` checks with cargo
Amends D-168 point 3 (Aaron, board task "Rust: delete checks with cargo", 2026-10-18). Part of the Rust symbol store and lockdown feature.
1. `delete <name>...` works in a Rust store as it does in Gleam: every name is resolved (`resolve`, with `rust_container` behind it for a container the item's name hides), each named item is retired with every item nested in it, and a module this empties loses its row, its file and its declaration — the `mod x;` line in its parent's header, or the parent's own `mod x;` item. `crate` is never removed, and a test crate root (`integration::x`) has no line. The row left last in an emptied module takes the deleted row's separator, so the file keeps its ending, and `materialize` prunes the module's file (D-136). cargo checks the result, so the reply is Gleam's `Deleted X; module M retired.` lines and `broken_report`.
2. It doesn't use the reference graph (D-170): the graph overestimates on purpose, so it would refuse a safe delete. A cargo error whose symbol was not broken before the delete and is not being deleted — or that names no symbol — rolls the whole list back, and the transaction writes the files again. One name gives Gleam's reply, `E5001 crate.two is still used by:\n  crate.three`, each erroring symbol listed once (an error in a header is listed as `<module> imports`); cargo can't say which of several names an error comes from, so several names are refused together: `E5001 nothing deleted.\ncrate.a, crate.b are still used by:\n  …`. That is the one reply that differs from Gleam's.
3. `reference.txt`'s Rust projects section says so; its `delete` row stays language-neutral (D-168 point 1).
Why: `delete` is what lets an agent remove Rust items once a task locks `.rs` files, and only cargo knows who really uses a name; the name-based graph would overestimate a use and block a safe delete.

## D-174 Cargo's and rustup's older setting names are settings too
Amends D-158 point 4 and Spec 14 §1 (Aaron, board task "Rust: lockdown tiers and sandbox", 2026-10-19). Part of the Rust symbol store and lockdown feature.
1. `.cargo/config` and `rust-toolchain`, the older names Cargo and rustup read beside `.cargo/config.toml` and `rust-toolchain.toml`, are settings the human approves too (D-158 point 4): each can name a program or a toolchain Cargo and rustup run at check time. Spec 14 §1 rule 2, the session-start brief and `reference.txt` name both pairs.
Why: the older name does the same thing, so protecting only the newer one would let an agent `write .cargo/config` with a `rustc-wrapper`, or a `rust-toolchain` naming its own program folder, and run it at the next check without approval.

## D-175 A workspace's keys and its members' files are settings too
Amends D-158 point 4 and Spec 14 §1 (Aaron, board task "Rust: lockdown tiers and sandbox", 2026-10-19). Part of the Rust symbol store and lockdown feature.
1. Cargo.toml's `[workspace]` keys `members`, `default-members`, `exclude` and `dependencies` are settings the human approves, like its dependency tables (D-158 point 4): members add crates whose build scripts run, and a workspace dependency can point an inherited dependency at a local path.
2. In a Rust project, a `Cargo.toml` or `build.rs` below the root is a protected setting (D-158 point 4): a workspace member or path dependency runs its build script at the next check, and its manifest can add dependencies.
Why: the review of this task showed that an ordinary `[workspace] members` edit plus a member's `build.rs` ran a build script with no approval, and `[workspace.dependencies]` could point an inherited dependency at a local crate.

## D-176 Rust dependencies are fetched where they change
Amends D-158 point 4 and Spec 14 §4 and its A11 (Aaron, board task "Rust: fetch dependencies outside the sandbox", 2026-10-19). Part of the Rust symbol store and lockdown feature.
1. `cargo fetch` runs in the project root, outside the sandbox, after a Rust import and after an approved change to Cargo.toml's dependency or build keys. Every check still runs `--offline`. Opening a Rust root that already has a store checks its cache offline and fetches only what is missing; a failed reopen fetch is shown on the root's lane and does not stop `serve`; the fetch runs in the background, so `serve` starts listening without waiting for it. A Rust project listed below a root is not fetched on reopen: its checks report the missing dependency.
2. A failed fetch is reported where it was asked for, with cargo's message, and undoes that step: an import creates no store, `solace.toml`, or `.solace/`, and an approval leaves Cargo.toml and Cargo.lock as they were (a new lock is removed).
3. Without `cargo`, an import goes ahead as a Gleam import does without `gleam`, and says "Dependencies not fetched: Needs the Rust toolchain (`cargo`)". An approval is refused with `E6013` and writes nothing.
4. An approved fetch runs while the board's projects are locked, so the board waits for it: first for a running reopen fetch (an offline try of up to 2 minutes, then an online fetch of up to 10 minutes), then up to 10 minutes for its own — about 22 minutes in all.
Why: `--offline` leaves an agent no command that can fill a missing cache, so the fetch must run where dependencies change; and a step that cannot fill the cache must not leave the store or the files changed.

## D-177 `[features]` is a setting too
Amends D-158 point 4 and Spec 14 §1 (Aaron, board task "Rust symbol store and lockdown: Tech Debt", 2026-10-19). Part of the Rust symbol store and lockdown feature.
1. A Cargo.toml's `[features]` table is a setting the human approves, like its dependency tables (D-158 point 4): a feature can turn on an optional dependency whose build script runs.
Why: the review of task:7db8dd126a22c863 showed that a feature edit could turn on an optional dependency that was already approved, along with its build script.

## D-178 A one-crate Cargo package below a root is added in place
Amends D-57 point 3 and D-59 (Aaron, board task "Workspace: add a Rust package in place", 2026-10-19). Part of the Rust symbol store and lockdown feature.
1. A one-crate Cargo package below a managed root is added in place, not migrated: `found` lists it with `language: "rust"`, and the Workspace page's Add runs `setup`. The import writes only `.solace/` and a new `solace.toml`; every other file is left byte for byte as it was.
2. `language::rust::package` reads and refuses the whole package before anything is written, naming each line outside the subset, so a refusal leaves the folder and the project list unchanged. Without `cargo`, the add goes ahead and says the dependencies weren't fetched (D-176).
3. Gleam still comes in by migration. Rust migration (copy, check, swap) and multi-crate workspaces stay deferred (D-158 point 6).
Why: a Cargo package already has the files Solace would project, so copying and swapping it would change nothing while risking the original.

## D-179 The symbol store is committed, but its local files are not
Amends D-58 point 6 (Aaron, board task "Adopt solace-cli into the store", 2026-10-19). Part of the Rust symbol store and lockdown feature.
1. One marked block in the Solace root's `.gitignore` shares every project's symbol database (`store.db`) and approved list (`approved`). Everything else in each `.solace/` stays ignored: locks, command logs, per-session caches, undo records, migration scratch, and `board/` run data, conversations and images.
2. Setup creates the root file when missing and replaces only its own marked block, leaving human text byte-for-byte; running setup again changes nothing. This is Solace's exception to D-68. A project's `.solace/.gitignore` is removed only when it is a file an earlier Solace version generated; one a human edited is left alone.
Why: the symbols belong to the project, while run logs and conversations can hold private data and the other files change constantly.

## D-180 A Rust module can be formatted whole
Amends D-158 point 5 (Aaron, board task "format: one Rust module", 2026-10-20). Part of the Rust symbol store and lockdown feature.
1. In a Rust project, `format <module>` runs rustfmt on that module's written file with the project's `rustfmt.toml` and edition, then reads the formatted file back into the store. Every item in the module is rustfmt's afterwards; ids stay by name and the change is checked like a submit. A file the store doesn't hold is out of scope.
2. `format` with no module is refused, because the crate isn't rustfmt-clean and one call would bury the task's real change; the refusal says to name a module. `submit` still formats only the items whose text changed (D-172 point 1).
3. rustfmt keeps `reorder_modules = false` in `rustfmt.toml` as well as on `format`'s command line, so `cargo fmt --check` and `format` agree. `format` compares the formatted file's `mod` list with the whole old file's, not just its header's, so a child module after the items isn't mistaken for an added one; a real add or drop is still refused.
Why: reshaping a module should take one call, not hand-written indentation for every item.

## D-181 Source files are read by symbol, not by path
Amends D-121 points 1–3 and D-162 (Aaron, board task "sources: block direct source reads", 2026-10-20). Part of the Rust symbol store and lockdown feature.
1. In a symbol-held project, source files are projections written by Solace. `view <file>` and `view <file>::<name>` are refused for graph source (E6001), as `read` already was; Python and a plain-file project's `.rs` keep the file forms. `files` may list graph paths as `[graph]` without contents, and `grep` doesn't search them.
2. `read`, `write`, `move`, `remove` and `view` give one graph-path hint for both languages: when the store holds the module, it names `view <module>` and `inspect <module>.<symbol>`; when the path isn't in the store, it names `submit --new <module>`. A refusal covers an exact graph path, or a new `.rs` or `.gleam` path under a graph root; a bare `app.rs` stays a symbol name.
3. A new Rust path names a module only when it is certain: `src/lib.rs` and a lone `src/main.rs` are `crate`; `src/a.rs` and `src/a/mod.rs` are `a`; `src/a/b.rs` and `src/a/b/mod.rs` are `a::b`; `tests/x.rs` is `integration::x` (D-169). Everything else under a graph root, including a second crate's `src/main.rs`, `src/bin/**`, and nested test paths, gets the can't-tell hint. `examples/`, `benches/`, and root `build.rs` aren't graph paths, so they get no graph hint.
4. Code is cited in the form the tools print, `module.item` (`lockdown.dispatch`, `checkout.pay`) or a method's `crate.Store::view`; `inspect` also accepts `::` in place of the last `.`.
Why: an investigation cited file paths because `view <file>` still read Rust graph source and the brief and reference taught it; source files are projections, so reading and citing them by name is the one way to reach the symbols the store holds.

## D-182 A command can say it is a test
Part of the Agent Tooling Efficiency feature (Aaron, board task "briefs: name the test commands to run", 2026-10-20).
1. A `[commands]` entry may set `test = true`; the key is optional, defaults to false, and a non-boolean is invalid (`E6020`).
2. A work brief names each scope project's marked commands once, after its last change: `test --all` still runs the toolchain's tests, then `run <name>` for the marked suites. A suite names its project when the task spans more than one, even if only one project marks it; a project with none adds nothing.
3. `test --all` remains the whole toolchain suite; the marker only keeps the brief from making the agent guess which declared commands are tests.
Why: overlapping declared suites made an agent run the same tests several times, before and after a one-line fix.

## D-183 A review's final assessment is its note
Part of the Agent Tooling Efficiency feature (Aaron, board task "review: recover a lost note", 2026-10-21). Amends D-76 and Spec 15 §6.
1. A review that exits cleanly, with no transcript error and no feedback recorded, records its last message as its feedback and ends `reviewed` only when the message's first line is exactly `**Assessment: Looks good**` or `**Assessment: Needs work**`.
2. A refused or failed `task note` is recovered this way; a call that replied `Recorded.` is already on its way and is not repeated.
3. The recovered message takes the `task note` path: the review checkpoint, any changed-during-review note and the `reviewed` transition. It adds no attention and leaves an existing mark in place.
4. A final message is not a note in general. Work and investigate runs still fail when they exit without finishing.
Why: a clean review with a complete assessment was marked failed only because its final message was not a `task note` call; the assessment was already the feedback the human needed.

## D-184 A passing test in a work run names the next step
Amends D-152 and Spec 3 §1 (Aaron, board task "test: a Next line on a pass", 2026-10-21). Part of the Agent Tooling Efficiency feature.
1. In a work run, a passing `test` reply ends `Next: if every part of the task is changed, finish with task done.` A failing reply, or a `test` outside a work run, has no such line.
2. When the pass names a change the task made that no test reaches, the line says `Next: add a test for <names>; then, if every part of the task is changed, finish with task done.`
3. The reference says "When `test` passes and every part of the task is changed, the code is done: finish with `task done`. `test` type-checks first, so don't run `check` or `probe` after it." Spec 3 §1 records the line.
Why: runs 8 and 12 spent extra `probe`/`check` calls after a passing test instead of finishing.

## D-185 Rust `extract` works within one project
Part of the Rust symbol store and lockdown feature (Aaron, board task "extract: Rust command, same project", 2026-10-22). Amends D-158 point 6 and Spec 1 §5.
1. `extract <symbols…> --to <module>` moves named Rust items and the private helpers only they use to another module of the same crate, keeping their identity and revision unless their text changes. A new target module is created with its `mod` declaration; an emptied source module is retired. `--to-project` stays refused (`E6013`).
2. References are rewritten only through the scoped resolver's `rewrite_sites` and `use_sites`: a bare name an import binds stays bare while the import is retargeted, and a name no import binds is qualified. Moved items and retained private helpers used across the new module boundary become `pub(crate)`.
3. A test that would be split from what it tests is refused (`E5002`); impls and nested test modules are not movable yet (`E6013`, named with task:cecf5788c7f06aa3), and import carrying or namespaces wait on task:f9b3a60e7dd8547f. A name the target already has is `E1004`.
4. One transaction gated on cargo: a check failure restores every row, file and history entry and shows the errors.
Why: Rust extract was refused as "Gleam only" even though the Rust resolver and store already had the pieces; same-project moves are the smallest useful step and cross-project work stays deferred.

## D-186 An investigation reports requirements, not edits
(Aaron, board task "briefs: outcomes, not edits", 2026-10-22). Part of the Agent Tooling Efficiency feature. Amends D-147 point 2, D-156 point 2 and Spec 15 §6.
1. An investigation's report states the required behavior, constraints, acceptance criteria and risks, plus anything it didn't check. It may name the symbols, modules and non-source files that need attention.
2. It does not prescribe implementation, edits, submission shape or checks: no plan assigning file changes, no count or kind of `submit`/`write` calls, and no test commands. A smaller option is still offered with `task ask` when it changes what the human gets.
3. A report never assigns a change to the human, who never edits files or runs commands; any needed change, a settings change included, is the worker's. Probing and tests are only for defining the issue more thoroughly.
4. A work run keeps ownership of edits and checks, including `submit` then `test`, `write` then `run <name>`, and `test --all`.
Why: an investigation answers what must be true, not what to do; prescribing edits and submission calls made it a plan the worker only followed, and cost calls.

## D-187 Rust `extract` moves impls and tests with their type
(Aaron, board task "extract: Rust impls and test modules", 2026-10-22). Part of the Rust symbol store and lockdown feature. Amends D-185 point 3.
1. A moved type takes its inherent and trait `impl` blocks, matched by the path the impl header resolves to, not by last name: `impl vendor::Task` stays when local `Task` moves. An ambiguous owner is refused (`E6013`) naming it.
2. A test that reaches only moved code moves, including one inside `#[cfg(test)] mod tests`: a whole container moves as it is; a partial move creates a container with `#[cfg(test)]` and `use super::*` in the target (`E1004` if the target has that name). A test that also reaches staying code is `E5002`. Private helpers a test uses move with it.
3. Nested methods, `use` items and `impl` rows cannot be selected directly (`E6013`); select the type or container.
4. `pub(crate)` promotion applies to top-level items and inherent methods only, never trait-impl methods (`E0449`).
Why: a type without its impls, or a test cut off from its subject, would not build; import carrying and namespaces stay with task:f9b3a60e7dd8547f.

## D-188 A task's milestone follows its feature
Amends D-87 point 2 and Spec 15 §1 (Aaron, board task "tasks inherit their feature's milestone", 2026-10-22). Part of the Roadmap: milestone usability feature.
1. A task with a feature always has that feature's milestone; only a task with no feature has a milestone of its own. Giving a task a feature sets its milestone to the feature's, even over one it had. Removing the feature keeps the milestone it had.
2. Changing a feature's milestone moves every task linked to it to that milestone, including tasks in a final column and archived tasks. A `PATCH` that sets a task's `milestone` while it has a feature is refused (`E7001`); the feature's milestone is the one place to change it.
3. A task saved before this change takes its feature's milestone when loaded; a task with no feature keeps its own milestone.
4. On the Roadmap, a feature appears only under the milestone it targets, never under a milestone that holds one of its tasks. A feature with no target milestone, or one not loaded, is under "No milestone".
Why: tasks could drift from their feature and the Roadmap showed a feature under every milestone with one of its tasks, so the Rust store tasks showed no milestone even though their feature was in Native Rust (Subset) Support.

## D-189 Back and Forward retrace Strategy tabs and record links
Amends D-90 point 3 (Aaron, board task "nav: Back follows tabs and links", 2026-10-22).
1. A Strategy tab change adds a history entry for `/strategy?tab=<name>`, rather than replacing the current one; Back and Forward restore the named tab in the mounted page. A missing or unknown value still shows Vision.
2. Following a task, feature or milestone link adds an entry for the resulting page and focus, including when the link's page is already shown; Back and Forward restore the page, tab and focused record. A direct record address is resolved in place, replacing its entry.
3. Modified clicks keep the browser's new-tab behavior.
Why: Back and Forward should retrace every navigation the human made, not only page changes.

## D-190 A stopped agent's raw exit text stays in its run log
(Aaron, board task "agents: plain messages for sign-in failures", 2026-10-23). Part of the Works out of the box feature.
1. When a run stops because its agent's sign-in expired, its program needs an update or the program is missing, the task shows what happened and what to do. The exit status and the agent's own words do not appear on the task, in a notice or in the run's outcome.
2. The run log is the run's technical details. It holds both the agent's output and the exit status, because the launch sends stdout and stderr there. The Runs tab opens it with Watch, and the run names its log file.
Why: a task has no separate technical-details area, and the run log already keeps the raw text the agent wrote.

## D-191 Setup's shared store block is pinned
Amends D-58 by adding point 7, D-179 point 1 and Spec 15 §4 (Aaron, board task "setup: keep store.db in git", 2026-10-23). Part of the Works out of the box feature.
1. The root `.gitignore` block shares every project's `.solace/store.db` and `.solace/approved`. Every other file below `.solace/`, including SQLite sidecars (`store.db-wal`, `store.db-shm`), locks, logs, caches, undo records and run data, stays ignored.
2. Setup's exact block is pinned by a test, so changing what is shared or ignored takes a deliberate test change.
Why: an ignore that hid `store.db` left the symbol store out of the project, while committing sidecars or local run state would add churn and private data.

## D-192 A new project is a Gleam project or a Cargo package
Amends D-158 and Spec 15 §4 (Aaron, board task "setup: new Rust project", 2026-10-23). Part of the Works out of the box feature.
1. The Workspace page's New project form creates a Gleam project or a one-crate Cargo package. `POST /projects/new` takes `language: "gleam" | "rust"`; missing means `gleam`, and any other value is refused with `E7001`, naming both. `target` applies only to Gleam.
2. For Rust, `cargo new <dir> --name <name>` runs in the root, without a nested git repository inside one, then `setup`; the result is a binary package with `src/main.rs` and Cargo's chosen edition, added in place as D-178 does. `cargo new` checks the name and its refusal is shown as written. A failed run leaves no folder, so a refusal changes no project list. Without `cargo`, a new Rust project is refused before anything is created; unlike Add, a new Rust package can't be made without it.
3. `solace create` stays Gleam-only. Rust libraries, multi-crate workspaces and migration of existing Rust code stay out of scope (D-178 adds Rust code in place).
Why: the form could make only Gleam projects, so starting a Rust project needed a shell and `cargo new`; the same form should do it, with Cargo's own name checks and no half-made folder.

## D-193 A Rust method name may leave out generics
(Aaron, board task "names: Rust methods without generics", 2026-10-24). Part of the Agent Tooling Efficiency feature. Amends D-168 and Spec 3.
1. A name-taking command matches exact names first. Only when none match does it compare a Rust symbol's name with each balanced `<…>` group after an identifier left out: `language::rust_refs.Refs::new` names `Refs<'a>::new` when one impl matches. A name starting with `<`, such as `<Failure as From<…>>::from`, matches only exactly.
2. This covers `view`, `inspect`, `refs`, `impact` and `delete` through `Store::resolve`, and patch headers and `*** Add: <module>.<Type>::<fn>` containers through the Rust symbol and container lookup. When more than one generic-free name matches, `E1002` lists the full names; an exact match always wins.
Why: an agent read a method as `Refs::new` and had to retype the lifetime the store showed, which cost a failed call.

## D-194 A patch Add may carry a type with its impl
(Aaron, board task "submit: a new type and its impl together", 2026-10-24). Part of the Agent Tooling Efficiency feature. Amends D-168 point 4 and Spec 3.
1. An `*** Add: <module>` may hold a new item with its inherent `impl` block, and any new `impl` or `trait` block. A block's own text still goes through `submit --new <module>`, not `submit <name>` or `*** Update: <container>`; this is the patch form D-168 point 4 left out.
2. An `*** Add: <module>.<Type>::<fn>` adds the method to the `impl` added earlier in the same patch, or, when only the type was added, opens one with the type's generics (defaults dropped, lifetimes and const parameters kept). The patch yields one `impl` per type and its arguments: impls with different arguments stay separate, an Add that names a method for an ambiguous type is refused with E1002, and impls from Add sections are held and rendered at the end.
3. A patch's `*** Add: <module>` may also add an inherent impl for a type already stored; the module, not the type's name, is the block's target.
4. `reference.txt`'s patch paragraph and Spec 3's `submit` row say so.
Why: an agent adds a type and its methods in one patch, and a second call that names the type as a container was the friction this removes.

## D-195 A task link opens on the board only while the board shows it
Amends D-98 points 2 and 3, D-129 point 2 and Spec 15 §4 (Aaron, board task "links: open on the board only if shown", 2026-10-24).
1. `GET /tasks/find/{id}` adds `onBoard`: true only while the board shows the task. It is false when the task is archived, is in Backlog, has no project (D-129), or has a feature that is `proposed`, `upcoming` or `canceled` (D-128). A feature that is `active` or `complete`, or that cannot be found, keeps the task on the board.
2. A task link opens on the board when `onBoard` is true, and otherwise on the Strategy page's Tasks tab with that task focused, as an archived task's link does today. A reply without `onBoard`, from an older server, keeps D-98's rule: the board unless the task is archived or has no project.
3. Feature and milestone links are unchanged.
Why: a link could open the board where a hidden task had no card, leaving the human with no way to reach it.

## D-196 The board takes task visibility from the server
Amends D-195 and Spec 15 §1 and §4 (Aaron, board task "links: open on the board only if shown", 2026-10-24).
1. Every task list and task event carries `onBoard`, computed by the same rule as `GET /tasks/find/{id}`. It is not stored on the task.
2. `solace_board.cards` shows a task when `onBoard` is true and the task's scope covers the lane. It no longer decides visibility from Backlog, archived state, or feature state; features still name and color their tasks.
3. When a feature's state change can change its tasks' visibility, the server announces those tasks, so the board updates them. `active` and `complete` are both visible.
Why: the board and a task link could disagree, and keeping the same rule in two languages invited drift.

## D-197 A proposal's body diff matches unchanged lines anywhere
Amends D-116 point 1 and Spec 15 §6 (Aaron, board task "proposals: body diff shows only changes", 2026-10-25).
1. A changed body or vision text shows as a line diff in its order: the lines both versions share anywhere, found as a longest-common-subsequence match, stay in place; a line only in the old text takes the deletion tint and one only in the new text the addition tint; no line is struck through. Each run of unchanged lines keeps one line of context on each side of a change and collapses the rest to one muted line, "3 lines unchanged" (or "1 line unchanged"). Lines are compared as D-116 says: space around a line is ignored and blank lines are left out, so a body differing only in those is unchanged.
2. The new body stays below the diff as an open "New body" section, as Markdown; a create, and a proposal saved before `before`, show every field as before.
Why: changing one line of a long body showed the whole old and new bodies, and then a diff of only the shared start and end still showed everything between the first and last change.

## D-198 A completed task with no feature files its review's tech debt too
Amends D-112 points 1, 3 and 4, and Spec 15 §6 and A20 (Aaron, board task "tech debt: file for tasks with no feature", 2026-10-25).
1. Every review is asked to put minor, non-blocking findings that should be fixed later under a `### Tech debt` heading, one finding per bullet with its location. A task with no feature is asked too, and the wording names no feature.
2. When a completed task has no feature, its last review's findings go to a new Tech Debt task, never an open one: `techDebt: true`, in `planning` or the first column of a workflow without one (D-132), with no feature or milestone, the completed task's `scope` and `home`, and titled `<task title>: Tech Debt`.
3. Each finding is one `solace` note naming its source as D-112 point 4 says, and the completed task gets a note naming the Tech Debt task. A task with a feature still uses its feature's open Tech Debt task as D-112 point 3 says. A cancel, a revert, or a review without the section files nothing.
Why: the feature path left tech debt from a task with no feature in that task's own notes, where later work never saw it. The human chose the same review section and filing for every completed task, with a Tech Debt task of its own when it has no feature.

## D-199 Review findings say what to change; tech debt has one exact shape
Amends D-112 point 1, D-198 point 1, and Spec 15 §6 and A20 (Aaron, board task "reviews: detailed findings, strict Tech debt format", 2026-10-25).
1. Every review is asked to write each finding so a worker can act on it without investigating again: it names the symbol (`module.name`) or file, what is wrong, what to change it to and why, and how to check it is done (a test to add or run, or what a command or page should show).
2. A review's minor, non-blocking findings that should be fixed later go in a Tech debt section at the end of the feedback, each written like a finding. The heading is exactly `### Tech debt`; each item is one `- ` bullet at the start of a line; its details are lines indented under it; no line after the heading starts with `#`; no other heading is inside the section, and nothing follows it except a new heading. A section in any other shape is dropped when the task is completed.
Why: reviewers ran stronger models than the workers following them, so vague findings sent workers back to the same code, and a Tech debt section in the wrong shape was silently dropped by `board.tech_debt_items`.

## D-200 Strategy runs read steps cheaply, list failures and see worker help
Amends D-99 points 2 and 3, D-105 point 2, D-144 and Spec 15 §6 (Aaron, board task "strategy runs: cheaper run reads, worker docs", 2026-10-25).
1. When `strategy show run` asks for steps (`<steps>`, `all` or `--failed`), `run` is one line: `<id> <kind> <model> <outcome>`, or its `state` when it has no outcome. With no step list it stays the whole run record without `log`.
2. `--failed` gives one line per failed `tool` step: `<step> <command head>: <first error line>`. The command is the first one the batch says failed, else the title's first line, without the tool name or failure marker; the error is the first E-code line in that command's batch section, else its first non-header line. No failed steps, or no steps, is an empty list. `--failed` takes no steps or `--from`.
3. A strategy run's `help <command>` shows a command it may run; for a worker command it can't run, it gives the command's reference paragraph marked as a worker command. Host commands stay hidden. A row's command names split on `|` only outside `<…>` and `[…]`, so placeholder alternatives add no command.
4. The strategy brief names `grep`'s `--in <path>` and `--context n`, and `strategy show run --failed`.
Why: a step read repeated a run's full record, failed steps could not be found without reading the whole transcript, and strategy help both omitted worker commands and listed placeholder fragments as commands.

## D-201 A new task with no feature can be approved into Planning
Amends D-93 points 4 and 7 and Spec 15 §6 (Aaron, board task "proposals: approve a task into Planning", 2026-10-25).
1. A strategy proposal that creates a task with no `feature` offers `Approve`, `Approve to Planning` and `Reject`. Every other proposal, including an edit, offers `Approve` and `Reject`.
2. `Approve to Planning` applies the proposal as `Approve` does, with the same refusals. The new task is created in `planning` of its home project, or the workflow's first column when it has none (D-132). Its answer is `Approved and applied: created task:<id> (Planning)`.
3. `Approve` still creates the task in Backlog.
Why: a human planning new work wants it on the board at once, without approving it into Backlog and moving it by hand.

## D-202 Planning runs are not tied to tasks
Amends D-93 and Spec 15 §4 and §6 (Aaron, 2026-10-25).
1. A strategy run is never started on a task: opening a task on the Tasks tab does not change the context of the run started next.
2. Runs saved on a task stay as stored, keeping their ids, notes and timestamps, and are listed with the page's runs.
Why: a run started while a task was open on the Tasks tab was filed under that task and disappeared from the planning panel, so a human believed it lost.

## D-203 A brief shows the plan around its task, and `task peek` reads it
Amends D-110 point 3, D-138 point 3 and Spec 15 §6 (Aaron, board task "briefs: surrounding plan and task peek", 2026-10-25).
1. A task run may read its own feature, the tasks it depends on, the tasks that depend on it, and tasks named as `task:<id>` in its body or notes. It may not read any other task or feature, or any task's runs. The notes are the task's stored notes, including `solace` notes and notes a resumed run has not seen; a deleted note no longer counts.
2. Every run kind's brief, `session start --task` in a board run, and `session remember` show the plan around the task. A **Feature** section gives the feature's title, state and whole body, then the latest direction note as D-138 says. A **Depends on** section appears whenever the task has dependencies, finished ones included: each as `task:<id> <title> (<column>)`, with `, in <project>` when it is not the task's home and `— unfinished` when it is not in a final column, then D-110's advice line only when one is unfinished. A **Needed by** section lists every task whose `dependsOn` names this one in the same one-line form. A deleted dependency or dependent is left out. One line names `task peek <id>` when a dependency or dependent is listed and `task peek feature` when the Feature section is present. No size cap applies; the brief stays as short as it can.
3. `task peek <task>` is read-only, so every run kind may use it. It gives the task's title, column, feature, dependencies and body. `--notes [<notes>]` adds the notes named, or every one, in full, numbered, and a reply too long for its budget stops at a whole note or comes in parts, with a `Next:` line ending in `--from <n>`, as `strategy show note` does. A task outside point 1's set is refused, and the refusal lists the tasks the run can read.
4. `task peek feature` gives the task's feature: its title, state, whole body and its tasks with their columns. A task with no feature, or whose feature is gone, is refused.
5. The usage line, `help task` and the reference name `task peek`.
Why: a run saw only the feature's title and latest note, so agents asked the human to paste the plan around their task.

## D-204 A code hunk matches through indentation; a file hunk doesn't
Amends D-142 point 1 and Spec 3 (Aaron, board task "submit: code hunks match through indentation", 2026-10-25). Part of the Work run friction fixes feature.
1. For code (`submit`'s hunks, `at_end` false), when exact and trailing-space matching find nothing, a third level that ignores space at both ends applies if it matches at exactly one start position. Quoted context and `-` lines keep the target's own text, so no line is re-indented by a quoted one. `+` lines shift by the difference between the target's indentation and the patch's at the first matched line, and a line shifted past the start stays at the start.
2. A loose match in two or more places is refused with D-142's `E0108` and its leading-space hint. The save reply says when a hunk matched through indentation.
3. A file patch (`write`) keeps D-142's strict rule: leading space is part of the line.
Why: run 00acbe40d2cf4d4d on task:3517183e01f63016 wrote one line with 12 leading spaces instead of 16, and about 30 failed draft repairs followed. Code is formatted after the check, and quoted lines keep the target's text.
