---
name: setup
description: Install and verify this repo's toolchain (Rust, Gleam, Erlang/OTP 29, Bun, bubblewrap, optionally the Tauri app) and build `solace` on Ubuntu or WSL. Use when asked to set up, install, or check the tools for this repo on a new machine.
---

# Set up Solace on Ubuntu / WSL

1. Run `scripts/setup-ubuntu.sh` from the repo root (`--app` also installs the Tauri app's libraries and CLI). It is idempotent and prints each tool's version.
   - Its apt step needs sudo, and only runs when a package is missing. If sudo can't prompt in your shell, tell the user the exact `sudo apt-get install -y ...` line from the error and ask them to run it with `! <command>`, then run the script again.
   - Pinned versions are `OTP_VERSION` and `GLEAM_VERSION` at the top of the script. Erlang comes from builds.hex.pm (apt stops at OTP 27) into `~/.local/otp`; the script adds it to `~/.profile` and `~/.bashrc`.
2. Verify: `cargo test --manifest-path solace-cli/Cargo.toml`. Unit tests need `bench/pilot-large/build`, which the script builds.
3. Remind the user that agents need Claude Code or the Codex CLI installed and signed in, and to open a new terminal so the new `PATH` applies.

Notes:
- Building `solace-cli` regenerates `solace-cli/board/priv/app.js` and `solace-cli/frontend/priv/app.js` when the web clients' sources change. Check `git status` first and mention any uncommitted changes there.
- Tauri app: `sh solace-app/scripts/sidecar.sh` after a release build, then `cd solace-app && cargo tauri dev`. Under WSL the window opens through WSLg.
- Machine-specific (not done by the script): to make Windows Terminal open Ubuntu in the repo, set the Ubuntu profile's `startingDirectory` to `\\wsl.localhost\Ubuntu\home\<user>\dev\solace`.
