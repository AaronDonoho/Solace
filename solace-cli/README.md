# solace-cli

The source of the `solace` executable. Installing and using it are described in the [repository README](../README.md).

- `src/main.rs`: the store, the symbol commands and the MCP server.
- `src/gleam.rs`: splitting Gleam source into symbols.
- `src/lockdown.rs`: file tiers, `solace.toml`, file and run commands, `agent-config` (Spec 14).
- `src/board.rs`, `src/default-board.toml`: the task board server and the default workflow (Spec 15).
- `board/`: the board's web client, which is built into the binary.
- `tests/`: the board acceptance scenarios (`board_acceptance.py`, run by `cargo test`), a fake agent, and golden files for `agent-config`.

A model in the system configuration can give `run = ["program", "argument", ...]` in place of `agent` and `model`, to start a custom adapter or a fake agent. The program gets the task prompt on stdin and the variables `SOLACE_TASK`, `SOLACE_RUN`, `SOLACE_SESSION` and `SOLACE_BIN`. The tests use only this path. CLI file commands use the session `SOLACE_SESSION` (default `cli`); each MCP process is its own session.
