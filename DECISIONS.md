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
