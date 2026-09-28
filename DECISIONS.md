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
