# Solace

A semantic programming system: programs are a store of identified, typed, verifiable symbols; text is one projection. Full design: `PROPOSAL.md` (long; read only the section you need).

## Working on a board task
- Your task, its notes and its answers are in your brief. Start from them.
- `DECISIONS.md` contains past decisions: `read DECISIONS.md --lines a..b` reads a record. Don't re-open a settled decision.
- A decision is added only when the human approves it. In a run, propose one with `task decision` (or `strategy decision` on the Strategy page), in the same short form as the others, when the human settles something later work must follow. If you need a human's judgment, ask with `task ask`; don't guess.

## Rules
- Specs (`specs/`) change only through a decision.
- Prefer tests as the spec: golden files (`input -> expected output/diagnostics`) for the parser, checker and code generation.
- Build the smallest thing that passes the current gate (PROPOSAL.md §14.1). Defer anything a later phase owns.
- Tests should be written to cover every logical branch.
- If implementing, clean up tech debt in your task.
- Write American English ("color", "organize"). Change British spellings you come across when they are in your task's scope.
- Where applicable, tests should be kept in separate files from runtime code.
- Practice domain-driven development.
- If any file or any solace command/response appears out-of-date, unclear, misleading, or causes mistakes: say so. Seek constant improvement of solace.
- Writings need to be understood by both humans and agents. Do not expect humans to comprehend implementation details.
- Be straightforward, but kind; please build a welcoming environment where everyone can do their best work.


## Layout
- `solace-cli/`: the Rust CLI and server. The Gleam projects below it are its pages and tools:
  - `board/`: the task board.
  - `migration/`: the migration tool.
  - `frontend/`: the app's shared page bundle.
  - `components/`: the shared web elements.
  - `contracts/`: the Contract view page, a project's code graph.
  - `strategy/`: the Strategy page: vision, personas, features, roadmap, tasks and the planning panel.
  - `settings/`: the Settings page: models, languages, agent behavior, skills and appearance.
  - `design/`: the Design page: architecture and UI/UX designs, their versions and proposals.
  - `delivery/`: the Build page: recipes, builds and releases.
- `solace-app/`: the Tauri desktop app (D-80).
- `specs/`: specifications, numbered.
- `bench/`: benchmark pilots, tasks and harness (benchmark runs need Aaron's go-ahead); `visualizer/` shows their results, with its page in `visualizer/frontend/`.
