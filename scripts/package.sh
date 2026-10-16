#!/bin/sh
# Package a built `tilr-server` binary as a GitHub release asset (used by release.yml).
#
#   scripts/package.sh TAG TARGET BINARY [OUT_DIR]     default OUT_DIR: target/dist
#
# BINARY is the built executable: target/TARGET/release/tilr-server for a
# `cargo build --release --locked -p tilr-server --target TARGET`, or
# dist/tilr-server-linux-<arch> for the Linux builds of scripts/release.sh (the
# same binaries as on the CDN).
#
# Output:
#   OUT_DIR/tilr-server-TAG-TARGET.tar.gz          tilr-server-TAG-TARGET/{tilr-server,LICENSE-*,README.md}
#   OUT_DIR/tilr-server-TAG-TARGET.tar.gz.sha256   "<sha256>  <archive name>" (sha256sum -c)
set -eu
if [ "$#" -lt 3 ] || [ "$#" -gt 4 ]; then
    echo "usage: $0 TAG TARGET BINARY [OUT_DIR]" >&2
    exit 2
fi
TAG=$1
TARGET=$2
BIN=$3
ROOT=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd -P)
OUT=${4:-${CARGO_TARGET_DIR:-$ROOT/target}/dist}
case $OUT in /*) ;; *) OUT=$PWD/$OUT ;; esac
NAME=tilr-server-$TAG-$TARGET

[ -x "$BIN" ] || { echo "missing $BIN" >&2; exit 1; }

STAGE=$(mktemp -d)
trap 'rm -rf "$STAGE"' EXIT
mkdir -p "$STAGE/$NAME" "$OUT"
cp "$BIN" "$STAGE/$NAME/tilr-server"
cp "$ROOT/LICENSE-APACHE" "$ROOT/LICENSE-MIT" "$ROOT/README.md" "$STAGE/$NAME/"

# COPYFILE_DISABLE: no AppleDouble (._*) entries from macOS tar.
COPYFILE_DISABLE=1 tar -czf "$OUT/$NAME.tar.gz" -C "$STAGE" "$NAME"
# shasum ships with macOS and the Ubuntu runners (perl); sha256sum does not ship with macOS.
(cd "$OUT" && shasum -a 256 "$NAME.tar.gz" >"$NAME.tar.gz.sha256")
echo "$OUT/$NAME.tar.gz"
