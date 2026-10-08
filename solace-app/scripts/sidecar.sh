#!/bin/sh
# Copies the release `solace` CLI to where Tauri's externalBin looks:
# binaries/solace-<target triple>.
set -e
cd "$(dirname "$0")/.."
triple=$(rustc -vV | sed -n 's/^host: //p')
mkdir -p binaries
cp ../solace-cli/target/release/solace "binaries/solace-$triple"
