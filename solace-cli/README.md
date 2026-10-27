# solace-cli

The source of the `solace` executable. Installing and using it are described in the [repository README](../README.md).

- `src/main.rs`: the store, the symbol commands and the MCP server.
- `src/drafts.rs`: rejected submissions kept as drafts, `submit --replace`, and patches (`apply_patch` form) turned into submit blocks.
- `src/reach.rs`: test reach and impact analysis (D-44).
- `src/reference/`: the agent reference, one file per section; the workflow before "Commands in full" is what `session start` shows.
- `tests/reference_layout.py`: checks the column layout of each reference section; the root's validator for `src/reference/` (D-143).
- `src/language/gleam.rs`, `src/language/rust.rs`, `src/language/cargo.rs`: splitting Gleam and Rust source into symbols, and running cargo's check and test.
- `src/store.rs`: the store's projection: a Rust file's items, kept nested and written back exactly, and what a Rust module's body, outline and session names are (D-158).
- `src/lockdown.rs`: file tiers, `solace.toml`, file and run commands, `agent-config` (Spec 14).
- `src/board.rs`, `src/default-board.toml`: the task board server and the default workflow (Spec 15).
- `src/outside.rs`: changes made outside Solace (D-58): what Solace last wrote (`.solace/written`), reading changed files in, Unexpected Changes, and the approved list (`.solace/approved`). `materialize` in `main.rs` asks it before writing a file.
- `src/workspace.rs`: the Solace root and its projects on one board: finding the one root, the project list, finding, adding and creating projects (D-57). Cross-project reads (`--project`) are `read_project` in `main.rs`.
- `src/migrate.rs`: copies existing projects, checks them and adopts the copy at the original path (D-59). `solace create <name>` only makes a new project, after confirmation.
- `src/view.rs`: the contract graph, task review and scratch test runner (Spec 16, P1–P3).
- `board/`, `contracts/`, `frontend/`, `migration/`: the web clients. The frontend's `[build]` step `release` bundles it before the crate builds (`build plan`); `build.rs` only embeds `frontend/priv/app.js`, which also holds the board, contract view and migration clients, stops with the step to run when it is missing, and warns when it is older than the sources of the projects `frontend/gleam.toml` lists, whose sources and manifests it watches, a project's folder standing in for a manifest it doesn't have yet. The board's own bundle serves its standalone page and is built by its `build-release` command.
- `tests/`: the board acceptance scenarios (`board_acceptance.py`, `view_acceptance.py` and `workspace_acceptance.py`, run by `cargo test`), a fake agent, and golden files for `agent-config`.
- Test-only crash hook: `SOLACE_FAIL_AFTER=command` exits a board run's Solace process after its command and before the undo rows commit; `SOLACE_FAIL_AFTER=commit` exits after the commit; `SOLACE_FAIL_AFTER=others` exits after the other projects of an `extract --to-project` or a `merge` (its destination, the dependents it rewrites) commit, before the source's. Recovery in `lockdown::recover` then adds the undo rows, or drops the pending record.

A model in the system configuration can give `run = ["program", "argument", ...]` in place of `agent` and `model`, to start a custom adapter or a fake agent. The program gets the task prompt on stdin and the variables `SOLACE_TASK`, `SOLACE_RUN`, `SOLACE_SESSION` and `SOLACE_BIN`. The tests use only this path. CLI file commands use the session `SOLACE_SESSION` (default `cli`); each MCP process is its own session.

The server provides `GET /graph`, `GET /graph?task=<id>`, `GET /nodes/<id>?source=1`, and `POST /tests` with `{"task":"<id>"}`. The board links changed tasks to `/contracts?task=<id>`. The contract page at `/contracts` (D-85) draws a searchable, expandable SVG contract sheet with node details, source on request, files and task review. It keeps node positions in browser storage, supports keyboard navigation and pan/zoom, and runs review tests with live updates. Runs and replay follow in later packages.

`GET /graph` also returns `project: {name, fingerprint}` for the drawing title block. The name comes from `gleam.toml`, falling back to the directory name; the fingerprint is the full SHA-256 of the store snapshot. Overview arranges isolated entries in a grid and fits the drawing on load and when Overview is selected.
