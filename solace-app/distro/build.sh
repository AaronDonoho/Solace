#!/bin/sh
# Builds the Solace WSL distro for Windows (D-224): Debian 13 slim with `solace`,
# Git, bubblewrap and curl, and no agent or language toolchain.
#
# Run it on Debian 13 with mmdebstrap installed, and pass a `solace` built on
# Debian 13 too, so its glibc matches:
#
#   build.sh <path to solace> <output.tar>
set -eu

if [ "$#" -ne 2 ]; then
    echo "usage: build.sh <path to solace> <output.tar>" >&2
    exit 2
fi

SOLACE_BINARY=$1
DISTRO_FILES=$(cd "$(dirname "$0")" && pwd)
export SOLACE_BINARY DISTRO_FILES

# Licenses in /usr/share/doc/*/copyright stay; the rest of the docs, manuals and
# translations go. Without recommended packages, Git doesn't pull in Perl's extras.
# procps is for the `kill` program: Solace uses it to end a timed-out command's
# whole process group and ignores a failure, so its absence would go unnoticed.
mmdebstrap \
    --variant=minbase \
    --components=main \
    --include=ca-certificates,curl,git,bubblewrap,procps \
    --aptopt='APT::Install-Recommends "false"' \
    --dpkgopt='path-exclude=/usr/share/man/*' \
    --dpkgopt='path-exclude=/usr/share/locale/*' \
    --dpkgopt='path-exclude=/usr/share/doc/*' \
    --dpkgopt='path-include=/usr/share/doc/*/copyright' \
    --customize-hook='install -m 755 "$SOLACE_BINARY" "$1/usr/local/bin/solace"' \
    --customize-hook='install -m 755 "$DISTRO_FILES/solace-desktop" "$1/usr/local/bin/solace-desktop"' \
    --customize-hook='install -m 644 "$DISTRO_FILES/wsl.conf" "$1/etc/wsl.conf"' \
    --customize-hook='chroot "$1" useradd --create-home --shell /bin/bash --uid 1000 solace' \
    trixie "$2"
