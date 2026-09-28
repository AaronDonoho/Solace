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
- SQLite store: symbol identities, revisions, references, work records, decisions.
- Import Gleam modules into symbols; canonical projection via `gleam format`; export per §7.5.
- Protocol CLI: `session start`, `inspect --context`, symbol-scoped submit, transactions (§14.5), work records, decision requests.
- **Checkpoint:** a benchmark run with the protocol vs. without. It has to show a measurable win on orientation cost and edit reliability before Phase 1b starts.

## Then: Phase 1b — Solace language core (Rust)
- Parser → canonical formatter → type and effect checker (holes, refinements checked at runtime, state machines, examples run as tests) → Erlang source (D-2).
- Golden-file tests throughout. Plug into the Phase 1a store in place of Gleam.
- **Gate 1:** agents build and maintain the pilot through the protocol alone; export round-trips losslessly.

## Open work records
- **Before the Phase 1a checkpoint:** the pilot is ~300 lines, too small for the protocol to show an orientation win. Grow it, or add tasks spread across more code, before comparing.
- **Pilot has no HTTP layer.** Add one only if a benchmark task needs it.
