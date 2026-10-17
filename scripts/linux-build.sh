#!/bin/sh
# Build the release binaries for Linux (the server tilrd and its client tilr)
# in the container and copy them to dist/.
# TILR_LINUX_PLATFORM picks the architecture (default: this machine's).
set -eu
cd "$(dirname "$0")/.."
mkdir -p dist
scripts/linux.sh sh -c '
    cargo build --release -p tilrd -p tilr &&
    cp /target/release/tilrd "/src/dist/tilrd-linux-$(uname -m)" &&
    cp /target/release/tilr "/src/dist/tilr-linux-$(uname -m)"
'
ls -l dist
