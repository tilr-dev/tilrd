#!/bin/sh
# Build a release binary for Linux in the container and copy it to dist/.
# TILR_LINUX_PLATFORM picks the architecture (default: this machine's).
set -eu
cd "$(dirname "$0")/.."
mkdir -p dist
scripts/linux.sh sh -c '
    cargo build --release -p tilrd &&
    cp /target/release/tilrd "/src/dist/tilrd-linux-$(uname -m)"
'
ls -l dist
