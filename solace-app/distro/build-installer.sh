#!/bin/sh
# Builds the Windows installer on Debian 13 (D-224, WINDOWS.md): `solace`, then
# the distro, then the Windows app, cross-compiled with MinGW so no Visual Studio
# is needed. Run it as root on a checkout with Linux line endings.
#
# It needs, once:
#
#   apt-get install --no-install-recommends ca-certificates curl gcc g++ libc6-dev \
#       make mmdebstrap debian-archive-keyring mingw-w64 nsis
#   (install Rust with rustup)
#   rustup target add x86_64-pc-windows-gnu
#   cargo install tauri-cli --version '^2' --locked
#
# The installer lands in
# solace-app/target/x86_64-pc-windows-gnu/release/bundle/nsis/.
set -eu

repo=$(cd "$(dirname "$0")/../.." && pwd)

cargo build --release --locked --manifest-path "$repo/solace-cli/Cargo.toml"
sh "$repo/solace-app/distro/build.sh" \
    "$repo/solace-cli/target/release/solace" \
    "$repo/solace-app/distro/solace-distro.tar"

cd "$repo/solace-app"
cargo tauri build --target x86_64-pc-windows-gnu --bundles nsis
