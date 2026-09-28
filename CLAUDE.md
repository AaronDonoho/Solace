# Solace

A semantic programming system: programs are a store of identified, typed, verifiable symbols; text is one projection. Full design: `PROPOSAL.md` (long, so read only the section you need; section numbers are cited below).

## Session protocol
1. Read `WORK.md` (current phase, open tasks) and `DECISIONS.md` (settled questions; do not re-open them).
2. Take one task. Finish it with a passing check (test, golden file, or benchmark run).
3. Before ending: update `WORK.md` (done / next / blockers). Record any new human decision in `DECISIONS.md`.
4. Need human judgment? Add a `pending` entry to `DECISIONS.md` and stop rather than guess.

## Rules
- Specs are written just before the phase that needs them (`specs/`), kept short, with testable claims. A spec changes only through a decision.
- Every phase ends at its gate (PROPOSAL.md §18 roadmap table). Gates are judged by the benchmark (§19), not by feel.
- Prefer tests-as-spec: golden files (`input -> expected output/diagnostics`) for the parser, checker and codegen.
- Build the smallest thing that can pass the current gate. Defer anything later phases own.

## Layout (grows as needed)
- `PROPOSAL.md`: design proposal
- `DECISIONS.md`: decision records (§9.4 format)
- `WORK.md`: roadmap and work records (§9.3 format)
- `pilot/`: small order service in plain Gleam (~2.5k tokens), the small benchmark size point
- `pilot-large/`: large order service in plain Gleam (~30k tokens), the main benchmark baseline
- `bench/`: benchmark tasks and harness (`python3 bench/bench.py validate|run|report`; tasks `0*` small, `L*` large)
