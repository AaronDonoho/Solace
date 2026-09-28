# Work

Roadmap: PROPOSAL.md §18 table. Phases are sequential; a phase ships only when its gate is met. Per D-1, Phase 1 is split so the central experiment runs before the language exists.

## Current: Phase 0 — Specify

**Gate 0:** specs 1–4 accepted; benchmark runs end to end on conventional tooling.

- [x] Install toolchain: Gleam 1.18.1, OTP 29, Rust 1.98.1.
- [x] Pilot baseline (`pilot/`, 12 tests passing): the order-management service (§18.4) in plain Gleam. Stored data, checkout state machine, database and clock passed in explicitly. Small, with tests.
- [ ] Benchmark: about 10 tasks against the pilot (new feature, defect fix, cross-caller refactor, resume half-finished work from a work record, maintain unfamiliar code). Each task has hidden tests.
- [ ] Benchmark harness: runs an agent on a task, records success, tool calls, tokens, and tokens before the first edit (§19.1). Runs end to end on the baseline.
- [ ] Spec 1: identity and revision model (§6), including answers to §6.5.
- [ ] Spec 2: text projection and export format (§7).
- [ ] Spec 3: agent protocol and diagnostics (§14).
- [ ] Spec 4: language core (§8, Phase 1 subset only: §18.3 list). Can wait until Phase 1b starts, as long as it's accepted before the gate.

## Next: Phase 1a — Store and protocol over Gleam (the central experiment)
- SQLite store: symbol identities, revisions, references, work records, decisions.
- Import Gleam modules into symbols; canonical projection via `gleam format`; export per §7.5.
- Protocol CLI: `session start`, `inspect --context`, symbol-scoped submit, transactions (§14.5), work records, decision requests.
- **Checkpoint:** a benchmark run with the protocol vs. without. It has to show a measurable win on orientation cost and edit reliability before Phase 1b starts.

## Then: Phase 1b — Solace language core (Rust)
- Parser → canonical formatter → type and effect checker (holes, refinements checked at runtime, state machines, examples run as tests) → Erlang source (D-2).
- Golden-file tests throughout. Plug into the Phase 1a store in place of Gleam.
- **Gate 1:** agents build and maintain the pilot through the protocol alone; export round-trips losslessly.

## Open work records
- **Benchmark tasks (next):** design ~10 tasks against `pilot/`, each with hidden tests kept outside the pilot tree (e.g. `bench/tasks/<n>/`). Decide the harness driver (probably headless Claude Code, which reports tokens and tool calls) when writing the harness.
- **Pilot has no HTTP layer.** Add one only if a benchmark task needs it.
