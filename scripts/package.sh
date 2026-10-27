#!/bin/sh
# Package built `tilrd` and `tilr` binaries as a GitHub release asset (used by release.yml).
#
#   scripts/package.sh TAG TARGET BIN_DIR [OUT_DIR]     default OUT_DIR: target/dist
#
# BIN_DIR holds the built executables tilrd and tilr: target/TARGET/release for a
# `cargo build --release --locked -p tilrd -p tilr --target TARGET`, or
# dist/linux-<arch> for the Linux builds of scripts/release.sh (the same binaries
# as in the CDN's archive).
#
# Output:
#   OUT_DIR/tilr-TAG-TARGET.tar.gz          tilr-TAG-TARGET/{tilrd,tilr,LICENSE-*,README.md}
#   OUT_DIR/tilr-TAG-TARGET.tar.gz.sha256   "<sha256>  <archive name>" (sha256sum -c)
set -eu
if [ "$#" -lt 3 ] || [ "$#" -gt 4 ]; then
    echo "usage: $0 TAG TARGET BIN_DIR [OUT_DIR]" >&2
    exit 2
fi
TAG=$1
TARGET=$2
BIN_DIR=$3
ROOT=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd -P)
OUT=${4:-${CARGO_TARGET_DIR:-$ROOT/target}/dist}
case $OUT in /*) ;; *) OUT=$PWD/$OUT ;; esac
NAME=tilr-$TAG-$TARGET

for exe in tilrd tilr; do
    [ -x "$BIN_DIR/$exe" ] || { echo "missing $BIN_DIR/$exe" >&2; exit 1; }
done

STAGE=$(mktemp -d)
trap 'rm -rf "$STAGE"' EXIT
mkdir -p "$STAGE/$NAME" "$OUT"
cp "$BIN_DIR/tilrd" "$BIN_DIR/tilr" "$STAGE/$NAME/"
cp "$ROOT/LICENSE-APACHE" "$ROOT/LICENSE-MIT" "$ROOT/README.md" "$STAGE/$NAME/"

# COPYFILE_DISABLE: no AppleDouble (._*) entries from macOS tar.
COPYFILE_DISABLE=1 tar -czf "$OUT/$NAME.tar.gz" -C "$STAGE" "$NAME"
# shasum ships with macOS and the Ubuntu runners (perl); sha256sum does not ship with macOS.
(cd "$OUT" && shasum -a 256 "$NAME.tar.gz" >"$NAME.tar.gz.sha256")
echo "$OUT/$NAME.tar.gz"
