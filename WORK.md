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
- [x] First Codex comparison (D-10 tools, Terra medium, 1 run per task; L01 had 3 runs per arm): **10/10 pass**. Solace reached the first edit in fewer turns (median 4 vs 6) but used **30% more tokens** in total (2.32M vs 1.78M); it won only L07 (−28%). Causes: (1) the session index (~8k tokens) is re-sent on every turn: about 9k × 67 Solace turns, more than the whole gap; (2) submit round trips: imports in submitted text weren't merged, unqualified names (`None`, `Option`) weren't auto-imported, and a type change and its callers needed one submit each (L06: 8 turns).
- [x] Fixed (2): a bare `submit` takes several `// ── name ──` blocks checked in one transaction; imports in submitted text merge into existing ones; unknown unqualified types and constructors are imported automatically (stdlib and unique project types); `submit --new module.name` works; `inspect <module>` gives the outline. Also fixed the bench's edit detection (batched submits were missed; L05's first edit was turn 5, not 12).
- [x] D-12: session index is names only; session start went from about 9k to 2.3k tokens.
- [x] Solace arm rerun with D-12 and the submit fixes (Terra medium, 1 run per task; files baseline from the 2026-09-28 batch): **5/5 pass**. Total tokens **−23%** (1.36M vs 1.78M): L01 −52%, L03 −11%, L05 +28%, L06 −14%, L07 −54%. Model turns −32% (48 vs 71). Turns to first edit: median 5 vs 6 (Gate 1 target ≤ 3 not met). Tokens before first edit mixed (19k–82k vs 34k–79k). L05 lost by reading whole modules with `--bodies` (~10k in one turn).
- [x] Codex truncated tool results over 10k tokens (4 of 5 Solace runs lost part of their first read and re-read it; the files arm never hit it). Harness now sets `tool_output_token_limit=30000` for both arms; the reference tells agents to read symbols, not modules, and to use `--bodies` sparingly. L05 Solace rerun: pass, 362k tokens (was 425k; files 331k), 11 turns (was 14), first edit at turn 4 (was 5), 40k tokens before it (was 65k). The rest of the gap is after the first edit: guessed constructors, a syntax slip, and a wrong assumption about audit order (7 turns of fixes).
- [x] Full L01–L10 comparison, both arms, Terra medium, 1 run each (files `20260928-010519`, Solace `20260928-012219`; wall time now recorded from the Codex log): **20/20 pass**. Tokens **−10%** (2.69M vs 3.00M; L02 −46%, L07 −35%, L05 −10%, L01 +27%, L03 +31%, L06 +37%). Turns −17% (104 vs 125). Uncached −6%. Wall time **+18%** (1172s vs 997s). Turns to first edit: median 5.5 vs 5, and tokens before the first edit higher in 9 of 10 tasks. The new "list the tests, then view one or two" advice cost a turn (outline in one turn, bodies in the next: L01, L06, L10), and L05 lost a turn to two commands sent newline-separated in one call. The files arm itself moved a lot since the last batch (L01 141k→83k, L05 331k→492k), so single runs vary ±40% per task.
- [ ] Next: decide whether this meets the Phase 1a checkpoint, or first repeat runs (single runs vary about ±15%) and work on the turns-to-first-edit target.
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
