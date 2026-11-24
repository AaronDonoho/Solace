#!/usr/bin/env bash
# Installs everything Solace needs on Ubuntu 24.04 (also under WSL), then builds
# and installs `solace`. Safe to run again: each step skips what is already there.
#
#   scripts/setup-ubuntu.sh          # tools, web client deps, solace
#   scripts/setup-ubuntu.sh --app    # also the Tauri app's libraries and CLI
#
# Only the apt step uses sudo, and only when a package is missing. Gleam and
# Erlang/OTP are not installed here: Solace uses only its own pinned copies,
# installed with Install on the first-run checklist (`solace serve`).
set -euo pipefail

repo=$(cd "$(dirname "$0")/.." && pwd)
app=false
[ "${1:-}" = "--app" ] && app=true
step() { printf '\n==> %s\n' "$*"; }
has() { command -v "$1" >/dev/null 2>&1; }
# Appends a line to ~/.profile (login shells, the Tauri app) and ~/.bashrc (terminals) once.
profile_line() {
    for f in ~/.profile ~/.bashrc; do
        grep -qxF "$1" "$f" 2>/dev/null || printf '\n%s\n' "$1" >>"$f"
    done
}

step "System packages"
pkgs=(build-essential curl git bubblewrap python3 ca-certificates)
# The Bun installer unpacks a zip.
has bun || [ -x ~/.bun/bin/bun ] || pkgs+=(unzip)
$app && pkgs+=(libwebkit2gtk-4.1-dev libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev file)
missing=()
for p in "${pkgs[@]}"; do
    dpkg-query -W -f='${Status}' "$p" 2>/dev/null | grep -q "install ok installed" || missing+=("$p")
done
if [ ${#missing[@]} -gt 0 ]; then
    sudo apt-get update
    sudo apt-get install -y "${missing[@]}"
else
    echo "all present"
fi

step "Rust"
if ! has cargo && [ ! -x ~/.cargo/bin/cargo ]; then
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
fi
. ~/.cargo/env
rustc --version

step "Bun"
if ! has bun && [ ! -x ~/.bun/bin/bun ]; then
    curl -fsSL https://bun.sh/install | bash
fi
profile_line 'export PATH="$HOME/.bun/bin:$PATH"'
export PATH="$HOME/.bun/bin:$PATH"
bun --version

step "Web client dependencies"
(cd "$repo/solace-cli/board" && bun install)
(cd "$repo/solace-cli/frontend" && bun install)

step "Solace"
cargo install --locked --path "$repo/solace-cli"

step "Pilot build (the Rust tests copy bench/pilot-large/build)"
# Solace's own tools folder, as the first-run checklist installs it.
tools="${XDG_DATA_HOME:-$HOME/.local/share}/solace/tools"
gleam=$(ls -d "$tools"/gleam-*/gleam 2>/dev/null | sort -V | tail -n 1)
erl_bin=$(ls -d "$tools"/erlang-*/bin 2>/dev/null | sort -V | tail -n 1)
if [ -n "$gleam" ] && [ -n "$erl_bin" ]; then
    (cd "$repo/bench/pilot-large" && PATH="$erl_bin:$PATH" "$gleam" test >/dev/null 2>&1 && echo "built")
else
    echo "skipped: install Gleam and Erlang with Install on Solace's first-run checklist, then run this script again"
fi

if $app; then
    step "Tauri CLI"
    cargo tauri --version 2>/dev/null || cargo install tauri-cli --version '^2' --locked
    sh "$repo/solace-app/scripts/sidecar.sh"
    echo "Run the app: cd solace-app && cargo tauri dev"
fi

step "Done"
echo "Open a new terminal (or run: source ~/.bashrc), then run: solace serve"
echo "On its first-run checklist, install Gleam and Erlang with Install; Solace uses only those copies."
echo "Agents: install and sign in to Claude Code (curl -fsSL https://claude.ai/install.sh | bash; claude) or the Codex CLI."
