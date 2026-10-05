# Solace

A semantic programming system: programs are a store of identified, typed, verifiable symbols; text is one projection. Full design: `PROPOSAL.md` (long; read only the section you need).

## Working on a board task
- Your task, its notes and its answers are in your brief. Start from them.
- `DECISIONS.md` contains past decision: `grep D-57` (or a word) finds the line, and `read DECISIONS.md --lines a..b` reads that record. Don't re-open a settled decision.
- Before finishing, add one entry to the top of `WORK.md`: `read WORK.md --lines 1..3`, then `write WORK.md` replacing the first entry's opening words with your entry followed by them. The entry says what changed, the checks that passed, and what's next.
- A new decision made by a human goes at the end of `DECISIONS.md`, in the same short form as the others. If you need a human's judgment, ask with `task ask`; don't guess.

## Rules
- Specs (`specs/`) change only through a decision.
- Prefer tests as the spec: golden files (`input -> expected output/diagnostics`) for the parser, checker and code generation.
- Build the smallest thing that passes the current gate (PROPOSAL.md §14.1). Defer anything a later phase owns.

## Layout
- `solace-cli/`: the Rust CLI and server; `board/`, `migration/` and `frontend/` below it are Gleam projects of their own.
- `specs/`: specifications, numbered.
- `bench/`: benchmark pilots, tasks and harness (benchmark runs need Aaron's go-ahead).
