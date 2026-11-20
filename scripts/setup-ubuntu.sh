#!/usr/bin/env bash
# Installs everything Solace needs on Ubuntu 24.04 (also under WSL), then builds
# and installs `solace`. Safe to run again: each step skips what is already there.
#
#   scripts/setup-ubuntu.sh          # tools, web client deps, solace
#   scripts/setup-ubuntu.sh --app    # also the Tauri app's libraries and CLI
#
# Only the apt step uses sudo, and only when a package is missing. Erlang/OTP
# comes from builds.hex.pm (Ubuntu's apt sources stop at OTP 27) and lives in
# ~/.local/otp; Gleam goes in ~/.local/bin.
set -euo pipefail

OTP_VERSION=29.1.1
GLEAM_VERSION=1.19.1

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

step "Gleam $GLEAM_VERSION"
export PATH="$HOME/.local/bin:$PATH"
if ! gleam --version 2>/dev/null | grep -qx "gleam $GLEAM_VERSION"; then
    tmp=$(mktemp -d)
    name="gleam-v$GLEAM_VERSION-$(uname -m)-unknown-linux-musl.tar.gz"
    url="https://github.com/gleam-lang/gleam/releases/download/v$GLEAM_VERSION/$name"
    curl -fsSL -o "$tmp/$name" "$url"
    (cd "$tmp" && echo "$(curl -fsSL "$url.sha256" | cut -d' ' -f1)  $name" | sha256sum -c -)
    tar xzf "$tmp/$name" -C "$tmp"
    install -D -m 755 "$tmp/gleam" ~/.local/bin/gleam
    rm -rf "$tmp"
fi
profile_line 'export PATH="$HOME/.local/bin:$PATH"'
gleam --version

step "Erlang/OTP $OTP_VERSION"
otp=~/.local/otp/OTP-$OTP_VERSION
if [ ! -x "$otp/bin/erl" ]; then
    . /etc/os-release
    base="https://builds.hex.pm/builds/otp/$(uname -m | sed 's/x86_64/amd64/;s/aarch64/arm64/')/ubuntu-$VERSION_ID"
    tmp=$(mktemp -d)
    curl -fsSL -o "$tmp/otp.tar.gz" "$base/OTP-$OTP_VERSION.tar.gz"
    sum=$(curl -fsSL "$base/builds.txt" | awk -v v="OTP-$OTP_VERSION" '$1 == v { print $4 }')
    [ -n "$sum" ] || { echo "no checksum for OTP-$OTP_VERSION in $base/builds.txt" >&2; exit 1; }
    echo "$sum  $tmp/otp.tar.gz" | sha256sum -c -
    mkdir -p ~/.local/otp
    tar xzf "$tmp/otp.tar.gz" -C ~/.local/otp
    (cd "$otp" && ./Install -minimal "$otp" >/dev/null)
    rm -rf "$tmp"
fi
profile_line "export PATH=\"\$HOME/.local/otp/OTP-$OTP_VERSION/bin:\$PATH\""
export PATH="$otp/bin:$PATH"
erl -noshell -eval 'io:format("OTP ~s~n", [erlang:system_info(otp_release)]), halt().'

step "Web client dependencies"
(cd "$repo/solace-cli/board" && bun install)
(cd "$repo/solace-cli/frontend" && bun install)

step "Pilot build (the Rust tests copy bench/pilot-large/build)"
(cd "$repo/bench/pilot-large" && gleam test >/dev/null 2>&1 && echo "built")

step "Solace"
cargo install --locked --path "$repo/solace-cli"

if $app; then
    step "Tauri CLI"
    cargo tauri --version 2>/dev/null || cargo install tauri-cli --version '^2' --locked
    sh "$repo/solace-app/scripts/sidecar.sh"
    echo "Run the app: cd solace-app && cargo tauri dev"
fi

step "Done"
echo "Open a new terminal (or run: source ~/.bashrc), then run: solace serve"
echo "Agents: install and sign in to Claude Code (curl -fsSL https://claude.ai/install.sh | bash; claude) or the Codex CLI."
