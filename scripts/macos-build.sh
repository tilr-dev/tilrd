#!/bin/sh
# Build and notarize the exact executables packaged for GitHub and the CDN.
set -eu
cd "$(dirname "$0")/.."
case "${1:-}" in
    macos-aarch64) target=aarch64-apple-darwin ;;
    macos-x86_64) target=x86_64-apple-darwin ;;
    *) echo "expected macos-aarch64 or macos-x86_64" >&2; exit 2 ;;
esac
[ "$(uname -s)" = Darwin ] || { echo "macOS releases must be built on macOS" >&2; exit 2; }
case "${TILR_SIGN_IDENTITY:-}" in
    "Developer ID Application: "*) ;;
    *) echo "TILR_SIGN_IDENTITY must name a Developer ID Application identity" >&2; exit 2 ;;
esac
: "${TILR_NOTARY_PROFILE:?set TILR_NOTARY_PROFILE to a notarytool keychain profile}"
platform=$1
cargo build --release --locked -p tilrd -p tilr --target "$target"
# Stage copies: signing must not mutate Cargo's cached build outputs.
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT HUP INT TERM
for exe in tilrd tilr; do
    cp "target/$target/release/$exe" "$stage/$exe"
    set -- --force --sign "$TILR_SIGN_IDENTITY" --options runtime --timestamp
    if [ -n "${TILR_SIGN_KEYCHAIN:-}" ]; then
        set -- "$@" --keychain "$TILR_SIGN_KEYCHAIN"
    fi
    codesign "$@" "$stage/$exe"
    codesign --verify --strict "$stage/$exe"
done
# Raw executables cannot carry stapled tickets. Apple's service records tickets
# for both signed binaries inside this ZIP; distribute the same executable bytes.
# Include the client too, without any intermediate unsigned files.
python3 - "$stage" <<'ZIP'
from pathlib import Path
import sys
import zipfile
stage = Path(sys.argv[1])
with zipfile.ZipFile(stage / "notarization.zip", "w", zipfile.ZIP_DEFLATED) as archive:
    for name in ("tilrd", "tilr"):
        archive.write(stage / name, name)
ZIP
set -- notarytool submit "$stage/notarization.zip" --keychain-profile "$TILR_NOTARY_PROFILE" --wait --output-format json
if [ -n "${TILR_SIGN_KEYCHAIN:-}" ]; then
    set -- "$@" --keychain "$TILR_SIGN_KEYCHAIN"
fi
xcrun "$@" > "$stage/notarization.json"
python3 - "$stage/notarization.json" <<'CHECK'
import json
import sys
with open(sys.argv[1]) as response:
    result = json.load(response)
if result.get("status") != "Accepted":
    sys.exit("Apple did not accept notarization; no release packaged")
CHECK
mkdir -p "dist/$platform"
cp "$stage/tilrd" "$stage/tilr" "dist/$platform/"
