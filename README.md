# Solace

Solace is an experimental system in which agents build and maintain a software project and humans direct them. It wraps a project: the project's code lives in a semantic store as individual symbols (functions, types, constants), and agents read and change it only through Solace, by symbol, with every change type-checked before it is saved. Humans hand work to agents through a local task board. Today Gleam is the only supported project language.

See [PROPOSAL.md](PROPOSAL.md) for the design, [DECISIONS.md](DECISIONS.md) for settled questions, and [WORK.md](WORK.md) for progress.

## What ships

One executable, `solace`, which contains:

- the store and the command-line interface;
- the MCP server that agents use (`solace mcp`);
- the task board server and its web client (`solace serve`);
- the generator for locked-down agent configurations (`solace agent-config`).

The benchmark harness (`bench/`), the pilots (`pilot/`, `pilot-large/`) and the benchmark visualizer are development tools for this repository and are not part of Solace.

## Requirements

- **To build:** Rust and Cargo (1.98.1 is used here).
- **To use on a project:** Gleam and Erlang/OTP (Gleam 1.18.1 and OTP 29 are used here). Solace runs `gleam` to format, check, test and build the project.
- **To run agents:** the Codex CLI or Claude Code, signed in. Agent runs use those services and may cost money.

Nothing else is needed at run time: the board's web client is built into the binary.

## Build

From the repository root:

```sh
cargo build --release --manifest-path solace-cli/Cargo.toml
```

The deliverable is the single file `solace-cli/target/release/solace`. The store (SQLite), the board's web client and the guide are all built into it, and it links only to the system C library. Copy it anywhere on the target machine's `PATH`. It runs on the platform it was built for, so build on (or for) each platform you ship to. Building needs only Cargo: the board's web client is committed prebuilt, so Gleam and Bun aren't needed unless you change the board.

## Install

To build and install in one step:

```sh
cargo install --path solace-cli
```

This puts `solace` in `~/.cargo/bin`. Use `cargo install --path solace-cli --root <dir>` to install elsewhere.

## Use it on a project

Solace wraps an existing Gleam project, and doesn't create one. It needs the project's `gleam.toml` because it runs `gleam` to check, test and build the code, and that file names the project and its dependencies. To start a new project, create it with Gleam first:

```sh
gleam new my_app
cd my_app
```

Then, in the Gleam project's directory:

```sh
solace import        # loads Gleam code and creates solace.toml if absent
solace check         # type-check the project from the store
solace serve         # start the task board on http://127.0.0.1:3030 (--port to change it)
```

Open the board, create a task and move it to planning. There you can have an agent *investigate* it first, which changes nothing and reports findings or a plan (complete the task there if it only needed an answer), then assign it to a model to do the work. Solace starts the agent, puts the agent's notes and questions on the task, and moves the task to *blocked* when the agent needs an answer or to *needs review* when it has finished. A task that is waiting on you is marked *Needs attention*, with the reason, and one whose agent has finished is marked as such. Effort is chosen separately from the model. Notes are written in a markdown editor and can include images, which agents also see. Cards show which tasks have an agent working, and each run on a task can be followed live: the agent's messages, and each command it runs with its result. The board's **Guide** link (`/guide`) explains the whole workflow for humans. The guide is built into the binary.

From Needs review, choose **reassign** to return a task to Assigned for fixes while preserving its changes. The next work run receives the review feedback and any new instructions; approval must be given again after the fixes. **Start over** instead reverts the task's changes.

### Models

Solace runs models through agent programs you already have installed and signed in to: the Codex CLI for OpenAI models, and Claude Code for Claude models. It stores no credentials. The current major models are built in, so the board has a list with no configuration. A model that can't start (its program is missing, isn't signed in, or its key variable isn't set) is greyed out with the reason, and assigning it is refused.

To add models or change the built-in ones, edit `~/.config/solace/config.toml` (or the file named by `SOLACE_CONFIG`):

```toml
task_timeout = 3600          # seconds an agent may work on a task; waiting for an answer doesn't count
# presets = false            # hide the built-in models

[[models]]                   # an OpenAI-compatible endpoint, through Codex
id = "local-qwen"
label = "Qwen3 Coder (local)"
agent = "codex"
model = "qwen3-coder"
endpoint = { kind = "openai", base_url = "http://localhost:11434/v1", key_env = "LOCAL_LLM_KEY" }

[[models]]                   # a Claude-compatible endpoint, through Claude Code
id = "proxy-sonnet"
label = "Sonnet via our proxy"
agent = "claude"
model = "claude-sonnet-5-5"
endpoint = { kind = "anthropic", base_url = "https://llm-proxy.example.com", key_env = "PROXY_KEY" }
```

An entry with the same `id` as a built-in model replaces it. `key_env` names the environment variable that holds the endpoint's key, which Solace reads when it starts the agent. Codex reaches OpenAI-compatible endpoints through the Responses API, so the endpoint has to support it. The built-in list is in [solace-cli/src/default-models.toml](solace-cli/src/default-models.toml).

### Project settings

A `solace.toml` in the project root holds the project's Solace settings. `solace import` creates a starter file if absent and preserves existing settings. Rerunning it on an imported project creates missing settings without reimporting or replacing stored symbols. Only humans edit it; agents can read it but not change it. Everything in it is optional.

An `AGENTS.md` in the project root holds your instructions for agents: conventions, domain rules, things to avoid. Every agent gets it first when its session starts, through Solace, whichever agent you run. Like `solace.toml`, only humans change it; an agent can propose a change, which you approve or reject on the task.

```toml
resource_limit = 262144                   # bytes; larger files can't be resources
resources = ["priv/**/*.html", "docs/**/*.md"]
ignore    = ["priv/generated/"]

[validators]                              # checked before a resource change is kept
"priv/**/*.html" = ["tidy", "-q", "-e", "{file}"]

[commands]                                # commands agents may run
smoke = { run = ["gleam", "run", "-m", "smoke"], timeout = 60 }
e2e   = { run = ["npx", "playwright", "test"], timeout = 600, checks = ["api.handle"] }
```

A command with `checks` is a test suite outside `gleam test`; `checks` names the entry points it drives (`"*"` for all; a `gleam run -m <module>` command adds that module's `main`). Solace tells agents when a change reaches a suite, and never runs it unasked.

The `[board]` section changes the task workflow: its columns, the actions allowed in each, and approval gates. The default is in [solace-cli/src/default-board.toml](solace-cli/src/default-board.toml), and [specs/15-board-api.md](specs/15-board-api.md) describes the format.

### What agents can touch

Every file in the project falls into one tier (details in [specs/14-lockdown.md](specs/14-lockdown.md)):

| Tier | Files | Agents |
| --- | --- | --- |
| 1 Source | Gleam files under `src/` and `test/` | read and change symbols through Solace, never as files |
| 2 Manifest | `gleam.toml` | read and write through Solace |
| 3 Resources | files matching `resources` in `solace.toml` | read and write through Solace, checked by any validator |
| 4 Ignored | everything else, including build output, logs and `.gitignore`d files | read and write through Solace; excluded from symbol searches |
| Internal | `.solace/` (store, board data) | no access |
| Settings | `solace.toml`, `AGENTS.md` | read; in a board work run, propose a change that you approve or reject on the task |

Files such as `README.md` and `bin/coolcalc` need no `resources` entry to be writable. File writes still enforce `resource_limit`, stale-write checks and any matching validators. Both resource and ignored file contents stay out of symbol searches. Running a launcher still requires a declared command; writing a script does not set its executable bit.

Agents get no shell and no file tools, only the Solace MCP tool and web access. Humans shouldn't edit source files directly either. Solace rewrites them from the store and doesn't keep edits made on disk.

### Running an agent yourself

The task board resumes the author's compatible conversation through investigation, implementation and review fixes. Reviews start fresh. If resumption is unavailable, Solace supplies a handoff automatically. Conversations are cleaned up after graceful shutdown: reviewers after review, authors when the task is completed or cancelled. Explicit new-agent/start-over actions start fresh.

An agent's session starts with a short workflow and one line per command; `help` gives every option. Agents edit code by sending a patch in the `apply_patch` form they already know, naming symbols or modules instead of files, so they send only the lines that change; Solace applies it and checks it like any other submit. Each submit reply reports what changed in the set of broken symbols: what it fixed, one line for each symbol still broken with an error already shown, and new errors in full, once per message, with the other places that have the same error one line each. `check` always shows every error. `inspect --context` on a type lists the places that construct it with every field (the ones a new field breaks); on a function, each caller's call line. Several commands in one tool call print each signature once.

Agents' `test` runs only the tests that can see a change since they last passed (and any that failed, or that reach other processes, the network or foreign code); `test --all` runs every test. After each submit, Solace tells the agent what else the change reaches and which of those no test checks (`impact` lists them all), and the review lists the same for you.

Rejected submissions are retained as editable drafts. Agents use `drafts`, `view --draft <id>` and `submit --draft <id> --replace` to repair them without resending the whole submission. `drafts` marks each one current or stale. A draft goes stale only when a symbol it replaces, or its module's imports, changes, and a stale draft refuses to overwrite newer source. Reviewers start with `task diff`, which shows each change the task made against its state before the task. Submission warnings are concise; `check --warnings` shows full details.

`solace agent-config codex|claude <dir>` writes a locked-down configuration for an agent into `<dir>`, plus a `launch.json` with the arguments and environment to start it with. The board uses the same configuration. To connect some other MCP client, have it start `solace mcp` in the project directory.

## Current limitations

- Gleam is the only project language.
- One agent works on a project at a time: assigning a second task is refused while one is running. The work of open tasks isn't isolated either, so a later task sees the unapproved changes of earlier ones.
- The board is local only, for one person, with no authentication.
- The contract view at `/view` shows entry points, logic, effects, tests and files on an interactive drawing sheet. The overview fits the project drawing, and its title block names the project and store version. A task link opens review with changes and test results. Running functions and replaying prior runs are later packages.
- Starting real agents from the board is not yet tested end to end. The tests use a fake agent.
- There are no prebuilt releases. Solace is built from source with Cargo, and needs several tools installed besides it: Gleam and Erlang/OTP for the project, and Codex or Claude Code for agents. Making setup easier is planned.

## Development

Tests (from the repository root unless noted):

```sh
cargo test --manifest-path solace-cli/Cargo.toml       # unit tests plus the board acceptance scenarios (needs python3)
python3 -m unittest bench.test_bench                   # benchmark harness
cd solace-cli/board && gleam test --target javascript && bun test test/api.test.mjs
```

The board's web client is in `solace-cli/board/` (Gleam and Lustre). After changing it, rebuild the bundle that is built into the binary, as described in [solace-cli/board/README.md](solace-cli/board/README.md). Bun is needed only for that.

### Pilots

The small and large pilots are Gleam order-management services used as benchmark projects:

```sh
cd pilot && gleam test
cd ../pilot-large && gleam test
```

### Benchmarks

Benchmark runs start real agents and may cost money. The harness runs tasks in temporary copies of the pilots. To check that a task's hidden test fails before its reference solution and passes after it:

```sh
python3 bench/bench.py validate L01-loyalty-coupon-order
```

To run one task through Solace and then report the result:

```sh
python3 bench/bench.py run --arm solace L01-loyalty-coupon-order
python3 bench/bench.py report bench/results/RUN_DIRECTORY
```

Each run uses the next numbered directory under `bench/results/`. `--run-number 2` saves both setups under `bench/results/2/`. `--agent claude` uses Claude Code. For a local model, `--base-url http://127.0.0.1:1234/v1` points Codex at an OpenAI-compatible server, but Codex gives MCP tools only to models it knows, so the Solace setup fails there; use `--agent claude` with `ANTHROPIC_BASE_URL` and `ANTHROPIC_API_KEY` set in the environment, and `CLAUDE_CODE_MAX_CONTEXT_TOKENS` set to the context the model is loaded with. `python3 bench/bench.py --help` lists every option.

### Benchmark visualizer

```sh
cd bench/visualizer && gleam run
```

Open the printed URL (normally `http://127.0.0.1:8765`) and load a directory such as `bench/results`. To rebuild its frontend after a change:

```sh
cd bench/visualizer/frontend
gleam build --target javascript
bun build build/dev/javascript/bench_visualizer_frontend/bench_visualizer_frontend.mjs --outfile ../priv/app.js --minify-syntax --minify-whitespace --target browser
```
