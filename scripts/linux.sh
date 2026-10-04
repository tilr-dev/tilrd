#!/bin/sh
# Run a command in the Linux build environment, with the source tree mounted.
# Without arguments, runs the whole test suite.
#
#   scripts/linux.sh                          # cargo test --workspace
#   scripts/linux.sh cargo build --release    # binary lands in the tilr-linux-target volume
#   scripts/linux.sh bash                     # a shell in the container
#
# TILR_LINUX_PLATFORM picks the architecture, e.g. linux/amd64.
set -eu
cd "$(dirname "$0")/.."

image=tilr-linux-dev
platform=${TILR_LINUX_PLATFORM:-}
suffix=$(printf '%s' "$platform" | tr -c 'a-z0-9' '-')

docker build ${platform:+--platform "$platform"} -t "$image$suffix" -f scripts/linux.Dockerfile scripts

if [ "$#" -eq 0 ]; then
    set -- cargo test --workspace
fi
tty=
if [ -t 0 ] && [ -t 1 ]; then tty=-it; fi
exec docker run --rm $tty ${platform:+--platform "$platform"} \
    -v "$PWD":/src \
    -v "tilr-linux-target$suffix":/target \
    -v "tilr-linux-cargo$suffix":/usr/local/cargo/registry \
    "$image$suffix" "$@"
