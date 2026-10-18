#!/bin/sh
# Build the release binaries for Linux (the server tilrd and its client tilr)
# in the container and copy them to dist/<platform>/, e.g.
# dist/linux-aarch64/tilrd; scripts/release.sh packs them into the release
# archive.
# TILR_LINUX_PLATFORM picks the architecture (default: this machine's).
set -eu
cd "$(dirname "$0")/.."
mkdir -p dist
# shellcheck disable=SC2016 # expanded in the container
scripts/linux.sh sh -c '
    out="/src/dist/linux-$(uname -m)" &&
    cargo build --release -p tilrd -p tilr &&
    mkdir -p "$out" &&
    cp /target/release/tilrd /target/release/tilr "$out/"
'
ls -l dist/*/
