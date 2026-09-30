# Decisions

Format (PROPOSAL.md §9.4): question, options, answer, rationale, who/when, affects. Status is `decided` or `pending`. Settled decisions are not re-asked.

## D-1 Test the central hypothesis before building the language — decided
- **Question:** Build the Solace language first, or first test whether a semantic store plus symbol-level protocol helps agents at all?
- **Options:** (a) language first, as the proposal's Phase 1 is written; (b) store and agent protocol over an existing language first, then the language.
- **Answer:** (b). The store and agent protocol (§6, §7, §14) are prototyped over **Gleam** code and benchmarked against plain Gleam files. The Solace language is built only after that shows a measurable win.
- **Rationale:** The language, checker and backend are the most expensive and least novel part of the bet. Gleam already has most of §8 (ADTs, exhaustive match, no null, Result/Option, labeled arguments, no macros) and targets both BEAM and JS.
- **Who/when:** Aaron, 2026-09-27
- **Affects:** Phase 0, Phase 1 ordering (see WORK.md)

## D-2 BEAM compilation target — decided
- **Question:** What does the Solace compiler emit for the BEAM?
- **Options:** Gleam source; Elixir source; Erlang source; BEAM bytecode.
- **Answer:** **Erlang source**, compiled by `erlc`. Core Erlang is the upgrade path if we need finer control.
- **Rationale:** The same argument §12.2 makes against compiling to Elm: emitting Gleam would put a second type checker and its error messages between Solace and the runtime. Elixir adds macros and a larger surface. Erlang source is the stable, direct BEAM input and needs no extra toolchain beyond OTP. Bytecode is unnecessary work.
- **Who/when:** Aaron (delegated the choice), 2026-09-27
- **Affects:** Phase 1 compiler backend; §18.1 toolchain (Rust compiler, Erlang runtime support)

## D-3 Implementation language for the toolchain — decided (by default)
- **Question:** What are the compiler, store and checker written in?
- **Options:** Rust (proposal §18.1); Gleam; OCaml; Go.
- **Answer:** **Rust**, binding from Phase 1b. The Phase 1a store and protocol prototype may use Rust or Gleam, since it's thin and cheap to redo.
- **Rationale:** Sub-second checks; a single binary (note: builds still need OTP because of D-2); Rust bindings for the Phase 2 solver; the Gleam compiler, written in Rust, is a working reference for an ML-style checker that emits Erlang. Against it: slower iteration, and graph-shaped data needs arenas or IDs.
- **Who/when:** Proposed by Claude, not objected to by Aaron, 2026-09-27. Can be reopened before Phase 1b starts.
- **Affects:** Phase 1b onward

## D-4 Submits that break dependents — decided
- **Question:** A submit that breaks its dependents (for example, a signature change): refuse it, or allow it?
- **Options:** (a) refuse, so the symbol and all its callers must be submitted together as one multi-symbol view; (b) accept if the symbol itself checks, and track broken dependents in the open change.
- **Answer:** (b). `commit` is refused while any broken dependents remain.
- **Rationale:** Otherwise a signature change could never be made one step at a time; benchmark task 04 is this case.
- **Who/when:** Aaron, 2026-09-27
- **Affects:** Spec 2 §2, Spec 3 §4

## D-5 Holes, error propagation and fallbacks — decided
- **Question:** Syntax for typed holes, returning early on an error, and replacing an error with a default.
- **Options considered:** holes as `?name` or `_name`; propagation as the `expr?` suffix or a `try` keyword; a fallback as `||` (with truthiness), `??`, or `else`.
- **Answer:** Holes stay `?name "question"` (as in PROPOSAL.md §8.6). Propagation is `try expr`. Fallback is `expr else value`, allowed on `Option` and `Result` only.
- **Rationale:** With `?` reserved for holes, a keyword for propagation avoids two meanings for one character. `||` would overload boolean OR and need truthiness, and §8.13 excludes both overloading and implicit conversions. `else` reads as English and can't be confused with holes. Swallowing an error with `else` is acceptable because it's explicit in the code and shows up as a branch in the decision tree and in test generation. If agents overuse it, a policy rule can limit it per namespace.
- **Who/when:** Aaron, 2026-09-27
- **Affects:** Spec 4 §3

## D-6 Spec review choices: contracts, effects, local names, machines — decided
- **Contracts:** checked at compile time where the checker can decide them, otherwise at run time, from Phase 1 on. (Spec 4 §3)
- **Unused effects:** a warning, not an error. Unused effects are compiled out, so callers aren't charged with them. (Spec 4 §4)
- **Local variable names:** excluded from the revision hash, since renaming a local changes nothing semantically. Bindings are hashed by position. (Spec 1 §3, Spec 2 §1)
- **State machines:** stay narrowed in Phase 1 (no data on states, no guards). Whether to widen them is revisited during Phase 1b. (Spec 4 §5)
- **Who/when:** Aaron, 2026-09-27, in the spec walkthrough. Specs 1–4 accepted with these changes and D-4, D-5.

## D-7 Benchmark sizes — decided
- **Question:** The pilot is about 2.5k tokens, too small for the protocol to show a token saving. How big should the benchmark codebase be?
- **Answer:** Build a second, larger pilot (`pilot-large/`) at about **30k tokens of source**, measured as source bytes ÷ 4. Keep the current pilot and its 10 tasks frozen as the small size point. The two sizes give a scaling curve.
- **Rationale:** Efficiency is measured in tokens. In the small pilot, 75% of a run's tokens are fixed per-call overhead and reading the code costs almost nothing, so there's nothing for the protocol to save. Tasks are written before the code, so the code isn't tuned to them.
- **Who/when:** Aaron (size), 2026-09-27
- **Affects:** bench/, Phase 1a checkpoint

## D-8 What the agent may use in the Solace arm — decided
- **Question:** In the Phase 1a comparison, does the agent work through the Solace protocol only, or get the protocol on top of its normal file tools?
- **Options:** (A) protocol only: no Read, Edit, Write, Glob or Grep on source files; reading and editing go through `solace` commands, and the agent can still run `gleam test`; (B) protocol plus the normal file tools.
- **Answer:** (A).
- **Rationale:** The proposal claims agents should work through the semantic model *instead of* files, and Gate 1 requires building and maintaining the pilot "through the protocol alone". (B) would largely measure the file workflow again if the agent falls back on familiar tools. Under (A), a run where the agent gets stuck shows a gap in the protocol.
- **Who/when:** Aaron, 2026-09-27
- **Affects:** bench harness (a second arm), Spec 3 (the protocol must cover everything the tasks need, including running tests)

## D-9 Agents reach the protocol through MCP, not the shell — decided
- **Question:** Spec 3 §1 has agents run `solace` through their shell, with an MCP wrapper only "if the benchmark shows it matters". Does it?
- **Answer:** Yes. The Solace arm gives the agent one MCP tool, `solace` (`command` plus optional `text` for new code), served by `solace mcp`, and no built-in tools at all. The CLI stays for humans and scripts.
- **Rationale:** In the first smoke run through the shell, (1) Claude Code auto-approved read-only commands such as `git status` and `git diff`, so the agent could see files despite D-8, and (2) Claude Code's shell safety check rejected heredocs containing ordinary Gleam (braces next to string quotes), forcing awkward `printf` workarounds. An MCP tool removes shell quoting and file access in one step.
- **Who/when:** Claude, 2026-09-27, under D-8 (Aaron's choice of protocol-only). Reopen if it causes problems.
- **Affects:** Spec 3 §1, bench harness

## D-10 Orientation: outline views, symbol index, batched inspect, turns as the metric — decided
- **Question:** The first Phase 1a comparison saved 32% of work tokens, but not on orientation. Agents called `session start` and some `inspect --context`, then viewed whole modules anyway (L07: 13 modules before the first edit). Tool results before the first edit came to only 3k–23k tokens, while 124k–410k was spent before it, so turns, not answer size, drive orientation cost. What changes?
- **Answer:**
  1. A module view returns an outline by default (signature, effects, one-line description, verification status). Bodies need `--bodies`.
  2. `session start` includes a one-line-per-symbol index (name, signature, description), narrowed with `--scope`, with "N more" beyond a per-module cap.
  3. `inspect a b c --context` returns several bundles in one call.
  4. `session start` expands open work records with the context bundles of the symbols they name.
  5. L07 is made fair: the work record goes in the store, the prompt says only to resume the work, and the files arm gets the same record as a handoff note in the repository.
  6. Orientation is measured in model turns before the first edit (primary), with tokens before the first edit secondary. Gate 1 gets a turn target: typical tasks reach their first edit within 3 model turns (median). Tokens remain the overall efficiency metric.
- **Rationale:** Change what the command agents already use returns, rather than asking them to use a different one. Every turn re-sends the whole context. L07's −41% came from symbol-level editing and diagnostics, not from work records, because the record was never in the store.
- **Who/when:** Aaron, 2026-09-27. The 3-turn figure is Claude's pick for "a turn target"; adjust if needed.
- **Affects:** PROPOSAL.md §7.4, §9.3, §14.1–14.3, §18 (Gate 1), §19.1, §19.3; Spec 3 (session start, view, inspect); bench (L07, turn metric)

## D-11 Benchmark agent: Codex with GPT-5.6-Terra at medium effort — decided
- **Question:** Which agent runs benchmark tasks from now on?
- **Answer:** The Codex CLI (`codex exec`), model `gpt-5.6-terra`, reasoning effort medium, is `bench.py run`'s default. Earlier runs stay as Sonnet 5 results; that mix is accepted. Every run needs Aaron's go-ahead first.
- **How the arms map:** each run gets a fresh `CODEX_HOME` holding only the sign-in, so user config, plugins, hooks and memories stay out. Both arms turn off Codex features that aren't part of the task (sub-agents, apps, plugins, goals, images, browser, web search). Files arm: the shell and `apply_patch` in a workspace-write sandbox. Solace arm (D-8): no shell, a read-only sandbox so `apply_patch` can't write, and the `solace` MCP server, which runs outside the sandbox. Codex runs this model in "code mode": one `exec` tool that runs JavaScript calling the other tools, so edits are detected by scanning that code.
- **Metrics:** taken from Codex's session log, which has one usage record per model call. Sign-in is through ChatGPT, so there's no dollar cost; tokens are the metric anyway.
- **Consequence:** arms compared with each other must use the same agent, so the D-10 rerun needs a fresh files baseline on Codex as well.
- **Who/when:** Aaron (agent and model), 2026-09-27; harness details by Claude.
- **Affects:** bench/bench.py

## D-12 The session index lists names only — decided
- **Question:** The D-10 index (one line per symbol with its signature, about 8k tokens on pilot-large) is re-sent on every turn. In the first Codex comparison that came to about 600k tokens over 67 Solace turns, more than Solace's whole deficit. Keep it?
- **Answer:** No. `session start` lists symbol names per module (about 1k tokens for pilot-large), and signatures come from `inspect` or `view <module>`. No separate command for the full index: agents re-send their context every turn, so none pays for it only once, and `view <module>` already gives signatures per module.
- **Who/when:** Aaron, 2026-09-28
- **Affects:** D-10 point 2, PROPOSAL.md §14.2, Spec 3 §2

## D-13 Task-directed session start; exact-replacement submit — decided
- **Question:** Orientation still takes 5–6 turns, as the agent follows code → tests → fixtures one step per turn. Solace turns are slower than files turns because submit re-sends whole symbols (about 59% of Solace output tokens vs 39% for apply_patch diffs). What changes?
- **Answer:**
  1. `session start --task` (the task in `text`) replaces the full symbol index with a task packet: name, signature, doc line and the reason each symbol was chosen, never bodies. Ranking is deterministic: task words (identifiers split at `_` and case changes, lowercased, common English words dropped) are weighted by rarity and by field (name 3, doc 2, code 1, which includes string literals such as fixture data); symbols the task names as `module.name` come first; one hop from the best matches adds callers, callees and covering tests, plus the non-test symbols (fixtures) that the top covering tests use. Entries are never truncated. The module list is reserved first, then entries are added in rank order until about 2k tokens are used. The Solace arm's prompt starts with it. Plain `session start` stays.
  2. `submit <name> --replace`: one exact old → new replacement (`old` and `text`) within one symbol; zero or several matches are refused; the whole resulting symbol is then formatted and checked in one transaction as usual. No line numbers, fuzzy matching or patch language. Whole-symbol submit stays the default.
- **Rationale:** (1) targets the general orientation problem rather than one missing fixture. (2) was made conditional on measurement, and the measurement showed submitted source is a large share of output. There are no revision or conflict checks to preserve yet (Spec 3 §4 is not built).
- **Who/when:** Aaron, 2026-09-28; weights (squared rarity, neighbours ¼ of the seed's score, fixtures ⅛) tuned by Claude against which symbols the ten L solutions change.
- **Affects:** Spec 3 §2 (session start), PROPOSAL.md §14.2; bench (Solace preamble)

## D-14 One signature per reply; a size budget instead of truncation — decided
- **Question:** In L07 (Luna Max) the two broad `inspect --context` replies were about 14k and 11k tokens. Codex cut both, and the agent re-requested part of the first one. 20–30% of each reply was the same signatures repeated across bundles (`Shipment`, `Store`). Paginate?
- **Answer:** No pagination: each page costs a model turn, and turns re-send the whole context (D-10). Instead:
  1. A reply with several names is one document: each signature is printed once, and later bundles refer to it as `(above)`. A symbol that was asked for is printed in full once, and other bundles refer to it as `(in full in this reply)`. Work-record context in `session start` works the same way.
  2. A reply has a size budget set per agent, below that agent's tool-output cut (about 3.9 bytes per token): 32,000 bytes for Codex, whose code-mode `exec` cuts at about 10k tokens, and 80,000 bytes for Claude Code, whose MCP cap is 25k tokens. `solace mcp --budget <bytes>` sets it; the default is 32,000. Commands in one tool call share it. Over budget, `inspect --context` trims in this order: callers become a count (`solace refs`), then tests are listed by name only, then each Uses list gives names only. The full text of requested symbols is never trimmed. Names that still don't fit are left out and listed at the end with the exact command that shows them. `view` falls back from module bodies to outlines, then lists what doesn't fit. A single item larger than the budget is still printed.
- **Rationale:** An agent pays for truncated output and then asks for it again. Trimming by priority, with a note saying what was left out, gives the agent a complete document it can act on. Raising agents' output limits scales only so far.
- **Replay on the Luna Max Solace arm (reads before the first edit, same commands):** L07 −33%, other tasks −9% to −34%. Only L07 and L06 hit the budget.
- **Low-budget check:** L01 at 6,000 bytes cost 211k tokens against 133k at 32,000, with 3 more turns before the first edit. The agent fetched what was left out. Trimming guards against truncation and does not save tokens, so the budget sits close to each agent's cut (Aaron: set it per agent).
- **Who/when:** Aaron, 2026-09-28; budget figures and trim order by Claude.
- **Affects:** Spec 3 §3; solace-cli (`inspect`, `view`, work-record context, `mcp --budget`); bench (`MCP_BUDGET`)

## D-15 Priorities from the visualizer feedback: build through the protocol first — decided
- **Question:** Three feedback sessions (`feedback/`) on building and changing the bench visualizer with Solace led to three proposals: (1) safe submissions (validate in a temporary projection, `submit --check-only`, a draft that survives a rejected submit); (2) the whole Gleam workflow through one MCP server (routing across projects, creating packages, dependencies, build/run/smoke commands); (3) missing context through the protocol (read-only dependency signatures; compiler and browser errors mapped to symbols). Which of these does the project take on, and in what order?
- **Answer:** The goal stays MCP-only (D-8, D-9): agents *build and maintain* code through the protocol alone (Gate 1). Each proposal is judged by whether it keeps an agent limited to MCP on the rails in Phase 1. In order:
  1. **A benchmark task that builds from scratch:** a new module with several interdependent types and functions. Every task so far is maintenance, so the "build" half of Gate 1 has not been measured.
  2. **Creating a large module safely through the protocol:** a draft held by Solace, which survives a rejected submit and can be fixed with `submit --replace` instead of sending the whole module again; `submit --check-only`; validation against a temporary projection; diagnostics that give lines within the submitted text.
  3. **Read-only dependency signatures:** `inspect` and `find` reach dependency packages (signatures from `build/packages/`, never added to the store), so an agent does not read package source or guess APIs.
  4. **Small fix:** `check` and a rejected `submit` no longer overwrite managed files that differ from the store (`materialize` currently does).
- **Deferred:** managing Gleam packages and dependencies, bundling, and routing across several projects (Phase 1b replaces Gleam; packages are Phase 6; the browser is Phase 4); run and smoke commands (`test` covers the pilot); runtime and browser errors mapped to symbols (Phase 2 replay, Phase 5 observability); `sync` / `import --update` (with MCP only, only humans edit managed files, so it is a convenience for them, not part of the agent path).
- **Rationale:** With MCP only, an agent has no scratch file and cannot read package source, so a server-held draft and dependency signatures are the gaps that push it off the protocol. The feedback sessions were not MCP-only (the agent used the CLI and file tools), so their friction over filesystem divergence largely disappears under D-8. Gleam-specific project management would mostly be thrown away in Phase 1b.
- **Who/when:** Aaron, 2026-09-28; the ranking was proposed by Claude.
- **Affects:** bench (new build task); solace-cli (`submit` drafts and `--check-only`, dependency index for `inspect`/`find`, `materialize`); Spec 3 §2 and §6

## D-16 Pause measurement; build lockdown, agent tooling and the human UX — decided
- **Question:** What does the project work on next?
- **Answer (decided):**
  1. Benchmark measurement is paused for a while. Gates are not judged until it resumes. The D-15 build task and before/after runs wait.
  2. Three priorities: (a) **lockdown**: agents do all code-related work through Solace and have no other way to change source; (b) **agent tooling** that makes that work effective (the D-15 drafts, `--check-only` and dependency signatures come under this); (c) **human UX**: the contract view and the channel to agents.
  3. Gleam is a target, not necessarily the final language. What gets built must not depend on Gleam where a language-neutral form is possible.
  4. **Contract** means the behavior and footprint of the system: side effects, APIs, infrastructure as code, and the logic that drives each. The human sees a high-level view, descends into each behavior, submits test data and watches it flow through each decision point to its output or side effect.
  5. **Channel to agents is asynchronous, through a task board.** Columns: backlog (prioritized) → planning (the human can ask agents to review and give feedback) → assigned (the human picks a model; Solace starts the agent and updates the status; the agent's notes go on the task) → blocked (can't finish or needs help) or needs review (finished). From blocked: unblock, or ask for more review. From needs review: approve to done, ask for more agent review, start over, or cancel. Any task can be paused at any time, and then any of these actions taken. A question an agent raises goes to Solace, which puts it on the task and blocks the task. A task may refer to existing contracts, ask for changes to them, or suggest new behavior.
  6. **The board and Solace are decoupled** and talk only through an API, so the board can be swapped for another. Solace ships with its own board as the default. The API is minimal and defined by Solace: only what a Solace-compatible board needs, not modeled on Jira or any other tracker. Adapting another board to it is that adapter's job.
  7. **The UX is a web app,** not native.
  8. **Files come in four tiers.** (1) Source: symbols, as now. (2) Project manifest: a typed record changed through commands (`solace deps add …`); the manifest file is generated from it. (3) Resources (HTML, YAML, SQL, IaC, fixtures…): edited through Solace, checked only by a validator declared for the file kind, shown in the contract view as unverified footprint; **no revisions stored**, since history is source control's job. (4) Ignored (generated code, logs, build output…): not tracked or reported; Solace only lists them in the file tree.
  9. **Sorting and access.** Tier 3 is the files matching patterns declared in the Solace project settings. Files and folders are made tier 4 by registering them in the project settings; `build/`, `.git/`, dependency caches and whatever `.gitignore` ignores are tier 4 anyway. A tier 3 file must stay under a size limit, which is a per-project setting that can be edited. Solace keeps one content hash per tier 3 file, to refuse edits made on top of a change it didn't see, but no history. A file in no tier is tier 4; it becomes tier 3 only by matching a declared pattern. The UI shows which tier every file is in.
  10. **No shell for agents (D-8 kept).** Agents get the Solace MCP tool (plus their own web access) and no shell or file tools. Through Solace: tier 1 only through the symbol protocol, never as files; tiers 2 and 3 read and written directly as files; tier 4 read only; a limited set of commands (such as check, test, build and run) that Solace runs for the agent.
  11. **Humans don't edit tier 1 files on disk.** Solace rewrites them from the store whenever it needs to and doesn't guard against human edits. This replaces D-15 point 4.
- **Who/when (8–11):** Aaron, 2026-09-29; tier layout proposed by Claude.
- **Rationale:** Aaron wants Solace usable end to end by a human directing agents before more measurement.
- **Who/when:** Aaron, 2026-09-29
- **Affects:** WORK.md (roadmap order; §18 puts the human view in Phase 3); D-15 (deferred packages and run commands are reopened by lockdown); PROPOSAL.md §9.4, §14.7, §15

## D-17 The board's workflow and where a task's work lives — decided
- **Question:** How fixed is the board workflow, where is task data kept, and where does a task's work live until it's accepted, so that `start_over` and `cancel` can undo it?
- **Answer:**
  1. **The workflow comes from configuration.** Columns, the actions allowed in each, and the stamps a column requires are set in `solace.toml`, and Solace ships a default. It is expected to change.
  2. **Task data is local files in the project** (`.solace/board/`), with no agent access. Boards still go through the API.
  3. **Solace is not tied to git.** Version control is the humans' job. The work stays in the project as it is made. To undo it, Solace records what each task changed and the state before the task first changed it: symbol text and revisions in the store, and the earlier bytes of tier 2 and 3 files. `revert` restores these. It is refused, with nothing changed, if a later task has changed any of the same symbols or files since. The records are invisible to agents: no search, view, index or count includes them. These records are dropped when the task reaches a final column, so they aren't history.
  4. **`approve` is a stamp.** A column can require a stamp before a task may enter it. In the default workflow, `done` requires `approved`.
- **Options considered:** a git branch and worktree per task (Claude's first recommendation); Spec 3 §4's staged changes in the store.
- **Rationale:** Keep Solace independent of the version control humans choose. The store already sees every symbol edit, so tying edits to a task costs a run id per submit. The store kept only a hash of each revision, not its text, so the earlier text has to be recorded for this.
- **Who/when:** Aaron, 2026-09-29; the building blocks for configuration, the rule for later tasks and the before-state records are by Claude.
- **Affects:** Spec 15; Spec 14 §1 (`.solace/` is internal); solace-cli (runs tied to submits, before-state records)

## D-18 Parallel work on tasks — decided for now
- **Question:** How will several agents work on tasks at once? D-17 lands a task's work straight into the project, so Spec 15 allows one work run per project at a time. The decision here is which direction to keep open.
- **Options:**
  (a) **Serial only:** one work run at a time (Spec 15 today).
  (b) **Shared tree with claims:** tasks claim the symbols and files they write. Each task's checks still see the others' unfinished work, so one task's compile errors break another's checks.
  (c) **Isolation in the store:** each task has its own open change (Spec 3 §4) and its own copy of any resource files it writes, and is checked in its own compile directory. `approve`, or an action step such as `merge`, merges the change symbol by symbol. The conflict checks Spec 3 already has (E5101, E5102) catch cases such as one task changing a signature while another adds a caller. It doesn't need git. D-17's before-state records are this design's base revisions.
  (d) **Git worktrees per task:** isolation comes from git. The main store has to be rebuilt from merged files while keeping symbol identities, and Solace becomes tied to git.
  (e) **(c) plus git as an output:** when a task is approved, Solace can also commit it to a branch or open a pull request, as an optional integration.
- **Claude's recommendation:** aim for (c), with (e) as an optional integration later; stay with (a) until parallel runs are needed. To keep the way open, Spec 15 describes what `revert` and `approve` mean, not how they are done. Moving from (a) to (c) then changes no API and no workflow configuration.
- **Answer:** (a) for now: one work run per project, with the revertible before-state records of D-17. To be revisited. Spec 15 §5 states what `revert` and `approve` guarantee separately from how it's done, so a later move to (c) or (e) changes no API or workflow configuration.
- **Who/when:** Aaron, 2026-09-29
- **Affects:** Spec 15 §5; Spec 3 §4; PROPOSAL.md §14.5, Phase 6

## D-19 Board refinements: agents wait, gates, task types; defaults accepted — decided
- **Question:** Aaron's review of the choices Claude made in Specs 14 and 15.
- **Answer:**
  1. **An agent that asks a question waits** for the answer instead of ending, so its context isn't lost. A human can choose to start a new agent on the task instead (`new_agent`). `pause` also holds the agent without ending it.
  2. **Later, a view of the agent:** from a task, a human can watch its agent work or read its context. The view is read-only, not a terminal. Asking the agent side questions without disturbing its work is a possible future decision. For now, runs only record where their session log is.
  3. **Approval gates:** any column can require a human's stamp (such as approval) before a task moves on to the next column. The default puts the gate where an agent's result shows up on the task (`needs_review`). `approve` only stamps; moving on is a separate action.
  4. **Task types:** tasks come in types. By default these are implementation, which changes the project, and investigation, which changes nothing and whose result is the agent's report.
  5. **Leaving a gated column any way but forward removes the stamp:** a task moved anywhere else after approval loses its approval. If an agent can't keep waiting, Solace starts a new one when the task continues.
  6. **Claude's other defaults in Specs 14 and 15 are accepted** (listed in the session of 2026-09-29): sizes, timeouts, `.solace/` internal, `.gitignore`d files in tier 4, command rules, JSON task files, local HTTP with no authentication, the model list in the system configuration, read-only review runs, dropping before-states in final columns, and spec numbers 14 and 15.
- **Who/when:** Aaron, 2026-09-29
- **Affects:** Spec 15 §1, §2, §3, §6; Spec 14 §5 (tool-call timeout in `agent-config`)

## D-20 Guardrails and the human interface come first; Solace ships as a product — decided
- **Answer:**
  1. **Priorities:** the primary goals are guardrails that keep agents working through Solace, and the human interface. Efficiency (tokens, speed) is a secondary goal. PROPOSAL.md says so in §1 and §19.
  2. **Order of the D-16 work:** lockdown → task board → contract view.
  3. **Contract view, basic version** until it has its own spec: an interactive graph of the system's behavior and footprint. Nodes are colored by type (API, system logic, infrastructure, side effects, …). The human can search, follow connections, submit test data and watch it flow through the decision points (D-16.4).
  4. **Solace is a deliverable:** the Solace system is built and shipped as a product that wraps a user's project. The benchmark (`bench/`) and the pilots are development tools and don't ship with it.
- **Who/when:** Aaron, 2026-09-29
- **Affects:** PROPOSAL.md §1, §19; WORK.md; packaging of solace-cli, the UX and the default board

## D-21 Lockdown session and launch configuration details — pending
- **Question:** Spec 14 does not define CLI session identity, the `agent-config` directory layout, or how a CLI caller supplies `old`.
- **Options:** explicit session handles everywhere; MCP process sessions and an optional CLI environment handle. Launch scripts or configuration plus a machine-readable launch descriptor.
- **Implementation reading:** MCP process identity; CLI `SOLACE_SESSION` (default `cli`), with one hash per file per session. `agent-config` writes isolated agent settings plus `launch.json` containing arguments/environment. CLI replacement accepts `--old <text>`. Bench keeps its existing web-disabled override.
- **Status:** pending human confirmation; accepted specs unchanged.

## D-22 Board system settings and pause representation — pending
- **Question:** Spec 15 leaves the system configuration schema, listen port, paused column, and default type ordering unspecified. A4 mentions `new_agent` from `needs_review`, though the default column does not allow it; review runs end with `task note`, whereas other notes do not end work.
- **Options:** a dedicated paused column versus retaining the configured column and setting `paused_from`; allow extra default actions versus follow the literal default table.
- **Implementation reading:** retain column while paused, expose `paused_from`; follow the literal default action table. Default type is implementation, custom types use TOML declaration order. System settings live at `$SOLACE_CONFIG` or `~/.config/solace/config.toml`, with `task_timeout` and `[[models]]` (`id`, `label`, `agent`, `model`, optional `effort`, or a test/custom `run` argv). Serve on loopback port 8080, overridable with `--port`. (Aaron, 2026-09-29: the default port is 3030.) Review `task note` completes that review. Missing models are an actionable error, never guessed.
- **Status:** pending human confirmation; accepted specs unchanged.

## D-23 Models, endpoints and tasks that need attention — decided
- **Answer:**
  1. **Solace runs models through external agent programs** (the Codex CLI, Claude Code) and stores no credentials. Agents use their own sign-in.
  2. **Built-in presets** for the current major models ship in the binary. The system configuration can add models, replace presets by `id`, or hide them.
  3. **Custom endpoints:** an OpenAI-compatible endpoint through Codex, or a Claude-compatible endpoint through Claude Code. An endpoint's key is read from an environment variable when the agent starts. An operating-system keychain is the accepted alternative, if environment variables prove insufficient.
  4. **The model is chosen when a task is assigned** (or a new agent started).
  5. **Unavailable agents:** a model whose program, sign-in or key is missing is shown as unavailable with the reason, and assigning it is refused. A task whose agent fails, or which needs an answer, is marked as needing attention, with the reason, until a human next acts on it.
- **Who/when:** Aaron, 2026-09-29; the attention field and the local-only availability check are by Claude.
- **Affects:** Spec 15 §1, §4, §6; solace-cli (`board.rs`, `default-models.toml`), the board client

## D-24 Following an agent's work on the board — decided
- **Answer:**
  1. **The board shows when an agent is working** on a task, on its card and in its detail.
  2. **A human can follow a run** as it happens, or read it afterwards: the agent's messages, thinking, and each tool call with its input, output and status, the calls collapsed until opened. It is a view only (D-19).
  3. **Agent text is formatted:** notes, messages and the attention reason are shown as markdown. A failed run's attention note gives the agent's last message rather than raw log lines.
- **Who/when:** Aaron, 2026-09-30; the transcript route and its entry format are by Claude.
- **Affects:** Spec 15 §4, §6, §7 and A13; solace-cli (`board.rs`), the board client and the guide

## D-25 Phases replace task types — decided
- **Question:** With task types (D-19.4), an investigation task could be "reviewed" in planning and then assigned for more investigation. The phases of a task already separate looking from changing, so is a type needed?
- **Answer:**
  1. **Tasks have no type.** What a run may do comes from its kind: an **investigate** run (planning) explores the task, asks questions and reports findings or a plan, and changes nothing; a **work** run (assigned) makes the change; a **review** run (needs review, blocked) assesses the task's changes. Only work runs write.
  2. **A task that only needs an answer ends after planning:** planning allows `complete`, and the investigation's report is the result.
  3. **An investigation's report goes to the work run**, in its brief with the other notes.
  4. **An investigate run that asks stays in planning** and waits, marked as needing attention. Starting `investigate` again continues it with the answers, as `unblock` does for a work run.
  5. **Later (not decided):** reviews that give a verdict and structured findings a human can turn into subtasks.
- **Who/when:** Aaron, 2026-09-30; points 3 and 4 are by Claude.
- **Affects:** D-19.4; Spec 15 §1, §2, §3, §4, §6, A9; solace-cli (`board.rs`, `default-board.toml`, `reference.txt`), the board client and the guide

## D-26 Tier 4 is writable; import creates project settings — decided
- **Answer:** Tier 4 files are readable and writable through Solace; being ignored excludes them from symbol search, not file editing. `README.md` and new files such as `bin/coolcalc` require no `resources` entry. This supersedes the tier-4 read-only rule in D-16.10 and Spec 14. Source still uses the symbol protocol; `.solace/` remains inaccessible and `solace.toml` remains human-owned. The existing text-write size limit, stale-write checks, matching validators and task undo apply to direct tier-4 edits as well. Text replacement refuses non-UTF-8 input rather than silently changing its bytes.
- **Import:** Create a starter `solace.toml` when absent, preserving any existing configuration. Rerunning import on an existing store succeeds and creates missing settings without reimporting source or changing stored symbols. Its examples explain classification and optional validators/commands; they grant no additional command execution by default.
- **Who/when:** Aaron, 2026-09-30; implementation details by Codex.
- **Affects:** Spec 14 §1–3 and A4; Spec 15 §5; import, file commands, reference, README and guide. Running a script remains subject to declared-command rules; writing it does not set executable permissions.

## D-27 Agent memory and skills — decided
- **Question:** Agent runs are isolated (D-16, D-21): no user settings, skills, plugins, extra MCP servers or memory carry over. How does an agent get skills and memories that last beyond one task?
- **Answer:**
  1. **Memories are tier 4 files** in a project directory named in `solace.toml`. Agents read and write them with the ordinary file commands (D-26); no new command is needed.
  2. **The brief lists a memory index, not the contents:** one line per file (its name and first line), plus an instruction to read what is relevant. An agent writes a memory when a human tells it to remember something, not on its own at the end of each run.
  3. **Skills use the same mechanism** with a human-owned directory: the brief lists them, and an agent reads one when it needs it.
  4. **Any run may write memory,** including read-only investigate and review runs; the memory directory is the only exception to their write refusal.
  5. **Memory edits are not task changes:** they do not appear in a task's review and are not undone when a task is reverted.
  6. **Review and investigation results are files,** written through Solace, and the board shows the file on the task. They are not memory and do not go in the memory directory. Solace writes the report to `.solace/board/`, beside the run's log, from the text of the run's finishing command; agents keep no access to `.solace/`. Reports, like the rest of the board, are not versioned for now; versioning the board may come later.
  7. **Later (not decided):** extra MCP servers, possibly as a Solace passthrough command (not named `mcp`, which starts Solace's own server) that logs each call and refuses it in read-only runs unless the human marks the server safe.
- **Who/when:** Aaron, 2026-09-30; points 3–5 are by Claude.
- **Affects:** Spec 14 (settings key), Spec 15 (brief, read-only runs, task changes); `board.rs`, `lockdown.rs`, `reference.txt`, README and guide

## D-28 Unlisted files are tier 3 — decided
- **Question:** Spec 14 §1 rule 8 puts every file no rule names in tier 4, so ordinary project files such as `README.md`, `bin/` and `docs/` are labelled ignored, `files` collapses their directories, and a `run`'s changes to them cannot be undone. Should unlisted files start in tier 3 instead?
- **Answer:** Yes. A file no earlier rule matches is tier 3 when it is text within `resource_limit`, and tier 4 when binary or over the limit. Tier 4 then means only noise: `.git`, build and dependency output, anything `.gitignore` ignores, `ignore` patterns, and binary or oversized files.
- **Humans must see the tiers:** the human view of the project (D-16's human UX) shows each file's tier and what that tier lets agents do.
- **Open (pending):** whether `resources` is still needed, since it no longer changes a text file's tier; validators keep their own patterns.
- **Who/when:** Aaron, 2026-09-30.
- **Affects:** Spec 14 §1–3 and A1; `lockdown.rs` (`tier`, `files`), the starter `solace.toml`, reference, README and guide

## D-29 Humans can delete notes — decided
- **Question:** Notes are append-only, so a wrong or stale note stays on the task and goes into every later agent's brief. Can a human remove one?
- **Answer:** Yes. The board deletes any note, whoever wrote it, after the human confirms. Deletion is permanent and isn't recorded. Notes are addressed by position, because they have no ids and are only ever appended.
- **Who/when:** Aaron, 2026-09-30.
- **Affects:** Spec 15 §4 (`DELETE /tasks/{id}/notes/{i}`), `board.rs`, the default board

## D-30 Answer submission continues the waiting agent — decided
- **Problem:** Aaron selected an option in Questions, but the answer was only saved; the investigation continued to wait for a separate Investigate action.
- **Behavior:** submitting the last pending answer continues the same waiting agent through its allowed workflow continuation. Work and investigation both use this behavior. Paused runs retain the answer until Resume. No answer submission starts a new agent, replaces a run, gives approval or reverts changes. Custom workflows without a continuation retain the answer and show an actionable attention reason.
- **Who/when:** Aaron reported the unmet interaction expectation, 2026-09-30; continuation behavior and pause handling implemented by Codex. Supersedes the separate answer-then-continue interaction in D-19/D-25.
- **Affects:** Spec 15 §4/§6/A5; question and resume endpoints; guide.

## D-31 Reassign from review without reverting — decided
- **Request:** Aaron wants to move a story from review back to assigned to address failed review, without starting over.
- **Answer:** Add `reassign` to Needs review in the default workflow. Preserve code, resources and original before-state records; clear approval; start a work run with the task notes, review feedback and optional human instructions. Use the existing work model unless another is selected. An active review must finish first. `start_over` keeps its existing revert behavior.
- **Who/when:** Aaron, 2026-09-30; action name and workflow configuration by Codex.
- **Affects:** default workflow, Spec 15, guide, README and board acceptance tests. Custom workflows can declare the same action explicitly.

## D-32 Confirm cancellation and completion — decided
- **Request:** Aaron wants confirmation dialogs for Cancel and Complete.
- **Answer:** The bundled board asks before sending either action. Cancel explains that agents stop and task changes are reverted; Complete explains that changes stay, the task closes, and undo records are removed. Both identify the task. Dismissing the dialog sends no request and preserves entered instructions. Use the existing browser confirmation, as for note deletion. API gates remain enforced after confirmation.
- **Who/when:** Aaron, 2026-09-30; dialog wording and implementation by Codex.
- **Affects:** bundled board, Spec 15, guide and client API test.
