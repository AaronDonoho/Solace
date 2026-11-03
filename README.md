# Solace

Solace is an experimental system in which agents build and maintain a software project and humans direct them. It wraps a project: the project's code lives in a semantic store as individual symbols (functions, types, constants), and agents read and change it only through Solace, by symbol, with every change type-checked before it is saved. Humans hand work to agents through a local task board. Today Gleam is the complete project language; a Rust Cargo package is held as symbols too: its items are read, checked and saved by name (D-168).

See [PROPOSAL.md](PROPOSAL.md) for the design, [DECISIONS.md](DECISIONS.md) for settled questions, and [WORK.md](WORK.md) for progress.

## What ships

One executable, `solace`, which contains:

- the store and the command-line interface;
- the MCP server that agents use (`solace mcp`);
- the task board server and its web client (`solace serve`);
- the generator for locked-down agent configurations (`solace agent-config`).

The benchmark harness (`bench/`), the pilots (`bench/pilot/`, `bench/pilot-large/`) and the benchmark visualizer are development tools for this repository and are not part of Solace.

## Install

Start from a machine with nothing but a terminal. Linux and macOS are supported (Solace is developed on Linux; on Windows use WSL). Steps 1 and 2 install what Solace uses, step 3 installs what builds it, steps 4 to 6 build and install it, and step 7 is optional.

### 1. What Solace uses

The app checks for these tools when it opens and shows this same list, with the install step for your system. None of them blocks the board from opening, but each is highly recommended: without Gleam or Erlang, projects can't build or run; without an agent, no task can run; without Git, there is no history to switch branches with; and on Linux, bubblewrap makes the folder a migration protects read-only to Solace's commands and runs Rust commands without network access, writing only `target/` and `Cargo.lock`.

| Tool | What it's for | Version |
| --- | --- | --- |
| Gleam | formats, checks, tests and builds your projects | 1.18.1 or newer |
| Erlang/OTP | runs Gleam projects | 29 or newer |
| An agent | Codex CLI or Claude Code; does the work you assign | current |
| bubblewrap | makes a migration's original folder read-only to Solace's commands, and runs Rust commands without network access, writing only `target/` and `Cargo.lock` (Linux) | current |
| Git | keeps your project's history and lets you switch branches | current |

**macOS** (if `brew` is missing, install Homebrew first):

```sh
/bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)"
brew install gleam erlang git
brew install codex   # or: curl -fsSL https://claude.ai/install.sh | bash
```

**Ubuntu or Debian:**

```sh
sudo apt update
sudo apt install -y build-essential curl libssl-dev libncurses-dev m4 bubblewrap git
# Gleam isn't in apt. Download the x86_64-unknown-linux-musl (or aarch64-unknown-linux-musl)
# archive from https://github.com/gleam-lang/gleam/releases, then:
tar xf gleam-*-unknown-linux-musl.tar.gz && sudo install -m 755 gleam /usr/local/bin/gleam
# Ubuntu's and Debian's erlang package can be older than OTP 29. Build OTP 29:
cd /tmp
curl -LO https://github.com/erlang/otp/releases/download/OTP-29.0/otp_src_29.0.tar.gz
tar xf otp_src_29.0.tar.gz && cd otp_src_29.0
./configure --prefix=/usr/local && make && sudo make install
cd -
# Then install one agent:
curl -fsSL https://claude.ai/install.sh | bash
```

For Codex instead, download a release binary from [the Codex releases page](https://github.com/openai/codex/releases).

**Arch Linux:**

```sh
sudo pacman -S --needed gleam erlang bubblewrap git
# Then install one agent:
curl -fsSL https://claude.ai/install.sh | bash
```

For Codex instead, download a release binary from [the Codex releases page](https://github.com/openai/codex/releases).

Other systems: install [Gleam](https://gleam.run/install/), [Erlang](https://www.erlang.org/downloads), [Git](https://git-scm.com/downloads), [bubblewrap](https://github.com/containers/bubblewrap) and one agent from step 2.

Open a new terminal so the tools are on your `PATH`, then check them:

```sh
gleam --version && erl -noshell -eval 'io:format("OTP ~s~n",[erlang:system_info(otp_release)]),halt().' && git --version
```

On Linux, `bwrap --version` should print a version too. Each should print a version. If one says "command not found", its installer printed a line to add to your shell profile; follow it and open a new terminal.

### 2. Get an AI agent

Solace doesn't contain a model. The board hands tasks to an agent program that you've installed and signed in to, and Solace stores no credentials. Install one of these, run it once, and sign in:

- **Claude Code:** `curl -fsSL https://claude.ai/install.sh | bash`, then run `claude`.
- **Codex CLI:** `brew install codex` on macOS, or a release binary from the [Codex releases page](https://github.com/openai/codex/releases), then run `codex`.

Agent runs use those services and may cost money. You can install and look around the board without an agent, but no task will run until one is signed in. The board offers only the models that can start; a line under the list says why the others can't (not installed, not signed in, or a usage limit reached), and **Try again** brings back a model held by a usage limit.

### 3. What builds Solace

Solace itself is built with Rust, its web clients are compiled by Gleam and bundled by Bun, and the bundled SQLite needs a C compiler. You only need these to build it; a release install does not.

| Tool | What it's for | Tested version |
| --- | --- | --- |
| Rust and Cargo | builds Solace | 1.98.1 |
| Bun | bundles the board's and the frontend's web clients | 1.2.23 |
| A C compiler | builds the bundled SQLite | system |

**macOS** (Homebrew's compiler tools cover the C compiler):

```sh
brew install rustup bun
rustup default stable
```

**Ubuntu or Debian:**

```sh
sudo apt install -y build-essential curl unzip
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
curl -fsSL https://bun.sh/install | bash
```

**Arch Linux:**

```sh
sudo pacman -S --needed base-devel rustup bun
rustup default stable
```

Other systems: install [Rust](https://rustup.rs), [Bun](https://bun.sh) and a C compiler.

### 4. Get the code

```sh
git clone https://github.com/AaronDonoho/Solace.git
cd Solace
```

### 5. Build and install the `solace` CLI

From the repository root:

```sh
cargo build --release --locked --manifest-path solace-cli/Cargo.toml
sudo install -m 755 solace-cli/target/release/solace /usr/local/bin/solace
solace --help
```

The first build takes a minute or two. The binary is self-contained (web clients and guide included); running it still needs Gleam, Erlang and an agent.

### 6. Build and install the desktop app

Only the Linux packages have been tested; macOS is untested and Windows isn't supported. Run everything from the repository root.

**1. Install Tauri's system libraries** (one line for your system):

| System | Command |
| --- | --- |
| Ubuntu or Debian | `sudo apt install -y libwebkit2gtk-4.1-dev libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev file` |
| Arch Linux | `sudo pacman -S --needed webkit2gtk-4.1 xdotool openssl libappindicator-gtk3 librsvg` |
| macOS | `xcode-select --install` |

**2. Build the app** (the same on every system):

```sh
cargo install tauri-cli --version '^2' --locked
cargo tauri build
```

This also rebuilds the CLI from step 5 and packs it into the app.

**3. Install the app** (one line for your system):

| System | Command |
| --- | --- |
| Ubuntu or Debian | `sudo apt install ./solace-app/target/release/bundle/deb/*.deb` |
| Arch Linux (or any Linux) | `./solace-app/target/release/bundle/appimage/*.AppImage` (it runs without installing) |
| macOS | `open solace-app/target/release/bundle/dmg/*.dmg`, then drag Solace into Applications |

Then open Solace from your launcher and choose your project's folder.

### 7. For working on Solace itself: test tools

Using Solace doesn't need these. Developing it does: the acceptance tests and the layout audit (`layout_audit`, which measures the pages and takes screenshots of them) are Python scripts, and the audit drives a headless Chromium through Playwright.

| Tool | What it's for |
| --- | --- |
| Python 3 and pip | runs the acceptance tests and the layout audit |
| Playwright for Python | lets the layout audit drive a browser |
| Chromium for Playwright | the browser the layout audit loads the pages in |

```sh
python3 -m pip install playwright
python3 -m playwright install chromium
```

Install them for the same `python3` that Solace finds on your `PATH`. If pip refuses with "externally-managed-environment", create a virtual environment (`python3 -m venv ~/.venvs/solace`), install into it, and activate it before starting Solace. When either is missing, `layout_audit` stops with one line saying which.

### Troubleshooting

- **`linker 'cc' not found`:** install the C compiler (`build-essential` on Ubuntu or Debian, `base-devel` on Arch; on macOS run `xcode-select --install`).
- **The build stops in a web client step (`gleam` or `bun` not found):** install them as in step 1. These steps run on every build, because the board's and the frontend's web clients are compiled then.
- **`solace: command not found`:** check that `/usr/local/bin` is on your `PATH`.

## Use it on a project

Solace wraps a Gleam project. It needs the project's `gleam.toml` because it runs `gleam` to check, test and build the code, and that file names the project and its dependencies. To start from nothing, create a new project after confirming its name and folder:

```sh
solace create my_app # runs gleam new and sets up Solace; --yes skips confirmation
cd my_app
solace check         # type-check the project from the store
solace serve         # start the task board on http://127.0.0.1:3030 (--port to change it)
```

Only `solace serve` writes tasks. With no server running, what a CLI run sends to its task (notes, questions, verification, settings proposals) waits in `.solace/tasks/inbox/` and reaches the board at the next `solace serve`.

For a project you already have, run `solace serve` in its folder, open **Workspace** in the menu bar and choose **Migrate this project**. Solace makes a copy next to it, `solace-migration`, sets up the copy to use Solace, and shows what it checked. Your folder isn't changed until you choose **Switch to the copy**, which moves it to `<name>.pre-solace` and puts the copy in its place; **Switch back** undoes that. The same from the shell:

```sh
solace migrate --yes           # make the copy and check it
solace migrate --confirm-paths # continue, after looking at any files it lists
solace migrate --report        # see what Solace checked
solace migrate --adopt --yes   # switch to the copy
```

Install bubblewrap (`bwrap`) so Solace's commands see your original folder as read-only during a migration and Rust commands run without network access. The copy has the same code as your original, errors included: a build or test that fails before migrating fails in the copy too. Solace won't switch if a check found a problem or your folder changed after the copy was made. `solace create` refuses existing names and unmanaged Gleam folders.

Open the board, create a task and move it to planning. There you can have an agent *investigate* it first, which changes nothing and reports findings or a plan (complete the task there if it only needed an answer), then assign it to a model to do the work. Solace starts the agent, puts the agent's notes and questions on the task, and moves the task to *Review* when the agent has finished. A task that is waiting on you stays where it is and is marked *Needs attention*, with the reason (an agent's question, or a run that stopped), and one whose agent has finished is marked as such. Effort is chosen separately from the model. Notes are written in a markdown editor and can include images, which agents also see. Cards show which tasks have an agent working, and each run on a task can be followed live: the agent's messages, and each command it runs with its result. The **Knowledgebase** page (`/knowledgebase`, in the menu bar) explains the whole workflow for humans; old `/guide` links lead there. The guide is built into the binary.

From Review, choose **reassign** to return a task to Assigned for fixes while preserving its changes. The next work run receives the review feedback and any new instructions; approval must be given again after the fixes. **Start over** instead reverts the task's changes.

### Several projects on one board

One Solace root can hold several Gleam projects in folders below it. The root folder is a project too. If it isn't a Gleam project, it's a project of plain files: its agents read, write and run declared commands on every file outside the other projects, such as docs, scripts and code in other languages. Solace sets the root up this way by itself, with its own lane, when `solace serve` starts there or a project is added below it. In the root folder:

```sh
solace create my_app # create a new project below this folder after confirmation
solace serve         # one board, with a lane per project
```

The **Projects** section of the **Workspace** page removes projects, re-lists removed projects that still have a store, and creates new ones: choose **Language** (**Gleam** or **Rust**), give a package name, and optionally a folder; for a Gleam project, also choose the target (Erlang or JavaScript), which a Rust package doesn't have. Solace runs `gleam new` or `cargo new` (without a nested git repository inside one) and sets the project up; a new Rust project is a one-crate binary package. `solace serve` in an empty folder starts an empty root, so a new project can be made entirely from the Workspace page. `solace create <name>` adds a new Gleam project to the enclosing root. To migrate several existing projects, run `solace migrate --yes --project api --project web` in their common folder, review the report, then run `solace migrate --adopt --yes`. The whole folder is copied and keeps its original path after adoption. The root's `solace.toml` keeps the list beside the root project's own settings:

```toml
projects = ["bench/pilot", "bench/pilot-large", "solace-cli/board"]
```

There is one root: `solace serve` and `solace create` find it from anywhere below, and every project stays at it or below it. Folders outside it, reached through `..` or a symlink, are refused, and so is a root inside another root or inside a project. Each project keeps its own store, `solace.toml`, `AGENTS.md`, tasks and agents. Agents in different projects work at the same time. An agent reads any project in the root by adding `--project <folder>` to `view`, `inspect`, `refs`, `find`, `files` or `read`. Reads take no locks, so they never wait on another project's agent, and humans can use the same option from a shell.

A task can change several projects. Pick them under **Projects this task can change** when you create it; the task's card then shows in each of their lanes. In the task's **Scope** tab you can switch projects on or off whenever its agent isn't working (paused or waiting on your answer counts), except a project the task has already changed. Its agent works in the task's own project and changes the others with the same `--project` option; asking for a project outside the task's list is refused, so the agent asks you. While it works, no other task's agent starts in any of those projects. Undo, review and completion cover every project the task changed.

Every project on the board uses the root's workflow (the `[board]` section of the root's `solace.toml`). Lanes are named by folder, fold away, and share one row of column headers. Removing a project keeps its setup. One board serves a project at a time. A Solace project nested inside another is off limits to the outer project's agents, apart from these reads.

### Models

Solace runs models through agent programs you already have installed and signed in to: the Codex CLI for OpenAI models, and Claude Code for Claude models. It stores no credentials. The current major models are built in, so the board has a list with no configuration. A model that can't start (its program is missing, isn't signed in, or its key variable isn't set) is greyed out with the reason, and assigning it is refused.

Solace also asks each agent program which models it offers, so a model the agent added shows in the list with no rebuild and no config edit. Codex answers `codex app-server`'s `model/list`, which names the efforts each model supports and its default; models Codex marks hidden are left out. A model whose agent names no default effort runs at `medium` when it offers it, else at the middle of the efforts it offers; a `[[models]]` entry's own `effort` wins. Claude Code has no command that lists models without starting one, so Solace offers its aliases — `opus`, `sonnet`, `haiku` and `fable`, each naming the latest of its family — and once a run's log names the model the agent used, the run shows it and the model's label names it. With `ANTHROPIC_API_KEY` set, Solace also reads the provider's models endpoint for the exact ids, active ones only; it stores no key and uses only the variable that is already set. The answers are asked at startup, about every 15 minutes, and when a run says its model is no longer offered, all in the background, so the board always shows the last list. A model that leaves an agent's offer keeps working for the tasks already using it, with the reason `no longer offered by …`, and isn't offered for a new run. A listed model isn't proof your account may run it: usage limits are shown separately.

To add models or change the built-in ones, edit `~/.config/solace/config.toml` (or the file named by `SOLACE_CONFIG`):

```toml
task_timeout = 3600          # seconds an agent may work on a task; waiting for an answer doesn't count
involvement = "collaborator" # how agents talk with you: director, collaborator or engineer
detail = "standard"          # how much they write: concise, standard or thorough
asking = "balanced"          # how readily they ask: often, balanced or rarely
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

An entry with the same `id` as a built-in model — or as a model Solace discovered — replaces it: your config wins. `key_env` names the environment variable that holds the endpoint's key, which Solace reads when it starts the agent. Codex reaches OpenAI-compatible endpoints through the Responses API, so the endpoint has to support it. The built-in list is in [solace-cli/src/default-models.toml](solace-cli/src/default-models.toml), and each agent's last answer is kept in `agent-models.json` beside your config file.

### Project settings

A `solace.toml` in the project root holds the project's Solace settings. `solace create <name>` writes a starter file in the new project. Migration preserves existing settings and adds a starter file if absent. Only humans edit it; agents can read it but not change it. Everything in it is optional.

An `AGENTS.md` in the project root holds your instructions for agents: conventions, domain rules, things to avoid. Every agent gets it first when its session starts, through Solace, whichever agent you run. Like `solace.toml`, only humans change it; an agent can propose a change, which you approve or reject on the task.

```toml
watch_limit = 262144                      # bytes; larger files are unwatched
unwatched   = ["priv/generated/"]          # files Solace doesn't watch or search

[validators]                              # checked before a file change is kept
"priv/**/*.html" = ["tidy", "-q", "-e", "{file}"]

[commands]                                # commands agents may run
smoke = { run = ["gleam", "run", "-m", "smoke"], timeout = 60 }
e2e   = { run = ["npx", "playwright", "test"], timeout = 600, checks = ["api.handle"] }
```

A command with `checks` is a test suite outside `gleam test`; `checks` names the entry points it drives (`"*"` for all; a `gleam run -m <module>` command adds that module's `main`). Solace tells agents when a change reaches a suite, and never runs it unasked.

The `[board]` section changes the task workflow: its columns, the actions allowed in each, and approval gates. The default is in [solace-cli/src/default-board.toml](solace-cli/src/default-board.toml), and [specs/15-board-api.md](specs/15-board-api.md) describes the format.

### What agents can touch

Solace puts every file in the project into one category, named for what Solace does with it (details in [specs/14-lockdown.md](specs/14-lockdown.md)):

| Category | Files | Agents |
| --- | --- | --- |
| Graph | Gleam files under `src/` and `test/`, and a Rust package's `.rs` files, held as symbols | read and change symbols through Solace, never as files |
| Watched | every other text file within `watch_limit`, including `gleam.toml` | read and write through Solace, checked by any validator; a task's changes are undone with it |
| Protected | `solace.toml`, `AGENTS.md` | read; in a board work run, propose a change that you approve or reject on the task |
| Unwatched | `.git/`, build output, `.gitignore`d files, paths listed in `unwatched`, binary or oversized files | read and write through Solace |
| Internal | `.solace/` (store, board data), and any other Solace project nested inside this one | no access |

File writes enforce `watch_limit`, stale-write checks and any matching validators. `gleam.toml` must still parse after a write and can't be removed. Watched and unwatched file contents stay out of symbol searches. Running a launcher still requires a declared command; writing a script does not set its executable bit.

Agents get no shell and no file tools, only the Solace MCP tool and web access. Code is meant to change through tasks, but Solace never writes over a source file it didn't write last. An edit made outside Solace (by hand, by another tool, by `git pull` or a branch switch) is read into the store. If an agent is working, the edit joins its task. Otherwise it waits on an **Unexpected Changes** task, marked with a hazard sign, until you complete it (keep the changes) or cancel it (restore Solace's code); no agent starts work meanwhile. Code that was approved in Solace, such as a teammate's pulled work, is read in quietly through `.solace/approved`, which you commit; Solace keeps one marked block in the root `.gitignore` that commits every project's `store.db` and `approved` and ignores the rest of each `.solace/`. Migration reports source differences before you adopt the copy.

### Running an agent yourself

The task board resumes the author's compatible conversation through investigation, implementation and review fixes. Reviews start fresh. If resumption is unavailable, Solace supplies a handoff automatically. Conversations are cleaned up after graceful shutdown: reviewers after review, authors when the task is completed or cancelled. Explicit new-agent/start-over actions start fresh.

An agent's session starts with a short workflow and one line per command; `help` gives every option. Agents edit code by sending a patch in the `apply_patch` form they already know, naming symbols or modules instead of files, so they send only the lines that change; Solace applies it and checks it like any other submit. Each submit reply reports what changed in the set of broken symbols: what it fixed, one line for each symbol still broken with an error already shown, and new errors in full, once per message, with the other places that have the same error one line each. `check` always shows every error. `inspect --context` on a type lists the places that construct it with every field (the ones a new field breaks); on a function, each caller's call line. Several commands in one tool call print each signature once.

Agents' `test` runs only the tests that can see a change since they last passed, and any that failed or never ran; `test --all` runs every test. After each submit, Solace tells the agent what else the change reaches and which of those no test checks (`impact` lists them all), and the review lists the same for you.

Rejected submissions are retained as editable drafts. Agents use `drafts`, `view --draft <id>` and `submit --draft <id> --replace` to repair them without resending the whole submission. `drafts` marks each one current or stale. A draft goes stale only when a symbol it replaces, or its module's imports, changes, and a stale draft refuses to overwrite newer source. Reviewers start with `task diff`, which shows each change the task made against its state before the task. Submission warnings are concise; `check --warnings` shows full details.

`solace agent-config codex|claude <dir>` writes a locked-down configuration for an agent into `<dir>`, plus a `launch.json` with the arguments and environment to start it with. The board uses the same configuration. To connect some other MCP client, have it start `solace mcp` in the project directory.

## Current limitations

- Gleam is the complete project language. A Rust Cargo package is held as symbols too: `check`, `test`, `submit`, `delete` and the reads work on it (D-168–D-173).
- One agent works on a project at a time: assigning a task is refused while another task's agent works in any project it covers. A task changes only the projects it lists, even when it needs a path dependency of its own. The work of open tasks isn't isolated either, so a later task sees the unapproved changes of earlier ones.
- The board is local only, for one person, with no authentication.
- The Contracts page at `/contracts` opens on an outline of what the project does: its main ways in and the requests they handle, then each area (source module) with every public function described in words, what it reads or changes, and whether tests check it. One button asks an agent to describe functions that have no description. The drawing sheet beside it shows areas, entry points, logic, effects, tests and files. A task link opens review with changes and test results. Running functions and replaying prior runs are later packages.
- Starting real agents from the board is not yet tested end to end. The tests use a fake agent.
- There are no prebuilt releases. Solace is built from source with Cargo, and needs several tools installed besides it: Gleam and Erlang/OTP for the project, and Codex or Claude Code for agents. Making setup easier is planned.

## Development

Tests (from the repository root unless noted):

```sh
cargo test --manifest-path solace-cli/Cargo.toml       # unit tests plus the board acceptance scenarios (needs python3)
python3 -m unittest bench.test_bench                   # benchmark harness
cd solace-cli/board && gleam test --target javascript && bun test test/api.test.mjs
```

The board's web client is in `solace-cli/board/` and the frontend shell that serves it, the contract view and the migration is in `solace-cli/frontend/` (both Gleam and Lustre). Building Solace rebuilds both bundles (see Install); each project's README describes its own build.

Desktop app (`solace-app/`, Tauri v2, D-80): after a release build of `solace-cli`, run `sh solace-app/scripts/sidecar.sh` once, then `cargo tauri dev`. To replace the placeholder icon, run `(cd solace-app && cargo tauri icon path/to/logo.png)` with a square PNG of at least 512×512.

### Pilots

The small and large pilots are Gleam order-management services used as benchmark projects:

```sh
cd bench/pilot && gleam test
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

Each run uses the next numbered directory under `bench/results/`. `--run-number 2` saves both setups under `bench/results/2/`. `--agent claude` uses Claude Code. For a local model, `--base-url http://127.0.0.1:1234/v1` points Codex at an OpenAI-compatible server, but Codex gives MCP tools only to models it knows, so the Solace setup fails there; use `--agent claude` with `ANTHROPIC_BASE_URL` and `ANTHROPIC_API_KEY` set in the environment, and `CLAUDE_CODE_MAX_CONTEXT_TOKENS` set to the context the model is loaded with. `--agent opencode` runs opencode against an OpenAI-compatible server (`--base-url`, default `http://127.0.0.1:8080/v1`) in both setups, with its own config, plugins and sessions kept out. `--agent trueforge` runs TrueForge (`npx @truefoundry/trueforge`, Node 22 or later) the same way: the bench starts a private TrueForge server for the run, the files setup works in TrueForge's local sandbox (it needs `bwrap`, `socat` and `rg`), and the Solace setup reaches `solace mcp` through a small HTTP bridge, since TrueForge connects to MCP servers only over HTTP. `--agent pi` runs pi (the `pi` command) against the same kind of server, in the Solace setup only: pi cannot limit its shell to the commands the files setup allows. Its settings, extensions and sessions are kept out, as with opencode. To compare harnesses on the same model, give each its own run and pass them all to the report: `python3 bench/bench.py report bench/results/21 bench/results/22` prints one line per harness and setup before the per-task tables. `python3 bench/bench.py --help` lists every option.

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
