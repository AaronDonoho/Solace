# Work

Roadmap: PROPOSAL.md §18 table. Phases are sequential; a phase ships only when its gate is met. Per D-1, Phase 1 is split so the central experiment runs before the language exists.

## Done: Phase 0 — Specify (Gate 0 met 2026-09-27)

**Gate 0:** specs 1–4 accepted; benchmark runs end to end on conventional tooling.

- [x] Install toolchain: Gleam 1.18.1, OTP 29, Rust 1.98.1.
- [x] Pilot baseline (`pilot/`, 12 tests passing): the order-management service (§18.4) in plain Gleam. Stored data, checkout state machine, database and clock passed in explicitly. Small, with tests.
- [x] Benchmark tasks: 10 in `bench/tasks/` (features, seeded defects, cross-caller refactor, state-machine change, rule changes that force test updates, resume from a work record). `bench.py validate` confirms each: hidden tests fail before, pass with the reference solution.
- [x] Benchmark harness: `bench/bench.py run` drives headless Claude Code in an isolated copy (six tools, no user settings/plugins/MCP), grades with hidden tests, records tokens, tool calls, tokens and calls before first edit, cost. Smoke-tested on task 02 with Haiku (pass, $0.06).
- [x] Baseline run `bench/results/20260927-*-claude-sonnet-5/`: Sonnet 5, 3 repeats, **30/30 pass**, median 173k tokens, 9 tool calls, 4 calls before first edit, $3.70 total. A 100% pass rate means the tasks are too easy to separate the two setups on correctness; comparisons will be on efficiency until harder tasks are added.
- [x] Spec 1 accepted: `specs/01-identity.md` (answers both §6.5 questions).
- [x] Spec 2 accepted: `specs/02-projection.md`.
- [x] Spec 3 accepted: `specs/03-protocol.md`.
- [x] Spec 4 accepted: `specs/04-language-core.md` (Phase 1 subset; everything else listed as deferred).

## Current: Phase 1a — Store and protocol over Gleam (the central experiment)

- [x] Large pilot (D-7): `pilot-large/`, 22 modules, 29.5k tokens of source, 98 tests. Plan and as-built notes: `bench/pilot-large-plan.md`.
- [x] 10 large tasks `bench/tasks/L01-*`…`L10-*`, all validated.
- [x] Large baseline `bench/results/20260927-221107-claude-sonnet-5/`: Sonnet 5, 5 tasks (L01, L03, L05, L06, L07), 1 run each, **5/5 pass**, median 1.6M tokens per task (1.13M work, 71%), 113k before first edit, $3.63 total. L02, L04, L08, L09 and L10 stay available for later.
- [x] Comparison design (D-8): the Solace arm uses the protocol only, with no file tools.
- [x] `solace-cli/` (Rust): import, session, view, inspect --context, refs, find, submit (symbol, --new, --imports, whole module), delete, check, test, work records, and `solace mcp` (D-9). Byte-exact round trip on pilot-large; inspect about 6 ms, submit plus type check about 0.5 s.
- [x] Solace arm run `bench/results/20260927-230127-claude-sonnet-5-solace/`: same 5 tasks, 1 run each, **5/5 pass**. Total tokens −42% (8.79M → 5.07M), work tokens −32% (6.20M → 4.21M), cost $3.63 → $2.92. Part of the total is the smaller fixed prompt (7.4k vs 15k). No orientation win: median 9 turns to the first edit vs 6 for files. Agents viewed whole modules instead of inspecting (L07: 13 modules before the first edit). The L07 work record was only in the prompt, so work records went untested.
- [x] D-10 built: module views are outlines (`--bodies` for full text); `session start` has a symbol index (about 7.3k tokens on pilot-large) and `--scope`; `inspect` takes several names; open work records are expanded with the context of the symbols they name; `work add` reads the record from the tool's `text`. Bench: `turns→edit` metric; a task's `work.md` becomes HANDOFF.md (files) or a store work record (solace); L07 now uses it.
- [x] Codex harness (D-11): `bench.py run` defaults to Codex, GPT-5.6-Terra, medium effort. Smoke-tested: the solace MCP tool works, `apply_patch` is blocked in the solace arm, `gleam test` passes in the files arm's sandbox, and the metrics match Codex's own totals.
- [ ] Rerun after D-10 on Codex (D-11): both arms on the same 5 tasks (`python3 bench/bench.py run --arm files L01* L03* L05* L06* L07*`, then `--arm solace`). **Ask Aaron first.** Main number: turns to the first edit (Gate 1 target: median ≤ 3), then tokens.
- **Phase 1a gaps against the specs** (add if the checkpoint passes): no changes/commit/base revisions (Spec 3 §4); no rename, copy, merge or extract (Spec 1 §5); no decision requests; doc comments stay in symbol text (Spec 2 §1); revisions hash comment-free text rather than identity-resolved content (Spec 1 §3); no export/import format (Spec 2 §4); unused imports are never removed.
- SQLite store: symbol identities, revisions, references, work records, decisions.
- Import Gleam modules into symbols; canonical projection via `gleam format`; export per §7.5.
- Protocol CLI: `session start`, `inspect --context`, symbol-scoped submit, transactions (§14.5), work records, decision requests.
- **Checkpoint:** a benchmark run with the protocol vs. without. It has to show a measurable win on orientation cost and edit reliability before Phase 1b starts.

## Then: Phase 1b — Solace language core (Rust)
- Parser → canonical formatter → type and effect checker (holes, refinements checked at runtime, state machines, examples run as tests) → Erlang source (D-2).
- Golden-file tests throughout. Plug into the Phase 1a store in place of Gleam.
- **Gate 1:** agents build and maintain the pilot through the protocol alone; typical tasks reach their first edit within 3 model turns (median); export round-trips losslessly.

## Open work records
- **Benchmark metric:** tokens (see `bench.py report`: total, work = total minus fixed per-call prompt, tokens before first edit, uncached). Orientation is measured in model turns to the first edit (D-10). Tool calls only explain token counts.
- **Pilot has no HTTP layer.** Add one only if a benchmark task needs it.
