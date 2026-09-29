# Solace

Solace is an experimental symbol-based programming tool. The current prototype stores Gleam functions, types, and constants in a local semantic store, exposes them through a CLI and MCP server, and checks edits with Gleam. The repository also contains two order-management pilots, a benchmark harness, and a browser-based benchmark visualizer. See [PROPOSAL.md](PROPOSAL.md) for the design and [WORK.md](WORK.md) for progress.

## Prerequisites

- Gleam and Erlang/OTP for the pilots and visualizer (Gleam 1.18.1 and OTP 29 are the versions used here).
- Rust and Cargo for the Solace CLI (Rust 1.98.1 is the version used here).
- Python 3 for the benchmark harness.
- Git for the benchmark harness's temporary project copies.
- Bun to rebuild the visualizer's browser bundle.
- Codex or Claude Code, configured with credentials, only if you want to run agent benchmarks. Benchmark runs use those services and may incur charges.

Start each section's commands from the repository root unless noted otherwise.

## Solace CLI and MCP server

```sh
cargo build --release --manifest-path solace-cli/Cargo.toml
export PATH="$(pwd)/solace-cli/target/release:$PATH"
```

Solace works from a Gleam project directory, not the repository root. Import a project once if it does not already have a `.solace/store.db`:

```sh
cd pilot-large
solace import
solace session start --task "change order pricing"
solace inspect pilot/pricing.quote --context
solace check
solace test
```

`solace help` lists the symbol-level read and edit commands. Submissions are type-checked before they are saved. The store is project-specific; run `solace import` separately in another Gleam project that has no store. To connect an MCP client, have it launch `solace mcp` with its working directory set to that project; this is a stdio server, not a web server. The benchmark harness sets up this MCP connection automatically for its Solace arm.

## Pilots

The small and large pilots are Gleam order-management models, exercised through their tests rather than standalone servers:

```sh
cd pilot && gleam test
cd ../pilot-large && gleam test
```

## Benchmark visualizer

Build the Lustre frontend after changing it:

```sh
cd bench/visualizer/frontend
gleam build --target javascript
bun build build/dev/javascript/bench_visualizer_frontend/bench_visualizer_frontend.mjs --outfile ../priv/app.js --minify-syntax --minify-whitespace --target browser
```

Then start the Ewe backend:

```sh
cd ..
gleam run
```

Open the URL printed by `gleam run`. It normally uses `http://127.0.0.1:8765` and picks a free port if 8765 is occupied. Use **Select files / folders** in the page to choose directories under this repository, such as `bench/results`, then refresh to load results. The frontend and backend each have their own `gleam.toml` and tests (`gleam test` from each directory).

## Benchmarks

The harness runs tasks in temporary copies of the pilots. Validate that a task's hidden test fails before its reference solution and passes afterward:

```sh
python3 bench/bench.py validate L01-loyalty-coupon-order
```

Run one task with the Solace MCP-only agent setup, then report the result using the directory printed by the run:

```sh
python3 bench/bench.py run --arm solace L01-loyalty-coupon-order
python3 bench/bench.py report bench/results/RUN_DIRECTORY
```

Replace `RUN_DIRECTORY` with the number printed by the run. Each run automatically uses the next numbered directory under `bench/results/`. Use `--run-number 2` to save both arms under `bench/results/2/`; repeating an arm there appends the next repetition number. Use `--agent claude` for Claude Code, or omit task names to run all tasks. `python3 bench/bench.py --help` lists all options. The visualizer can load a run directory or an individual scenario.
