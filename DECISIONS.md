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
