#!/bin/sh
# Lay out built dist/PLATFORM/{tilrd,tilr} as a CDN release.
# Shared by release.sh and CI packaging of signed macOS binaries.
set -eu
if [ "$#" -ne 2 ]; then
    echo "usage: $0 VERSION PLATFORM" >&2
    exit 2
fi
version=$1
platform=$2
mkdir -p "dist/cdn/releases/$version"
archive="tilr-$version-$platform.tar.gz"
pack="dist/pack-$platform"
rm -rf "$pack" && mkdir -p "$pack"
cp "dist/$platform/tilrd" "dist/$platform/tilr" LICENSE-APACHE LICENSE-MIT README.md "$pack/"
chmod 755 "$pack/tilrd" "$pack/tilr"
# COPYFILE_DISABLE: no AppleDouble (._*) entries from macOS tar.
COPYFILE_DISABLE=1 tar -czf "dist/cdn/releases/$version/$archive" -C "$pack" \
    tilrd tilr LICENSE-APACHE LICENSE-MIT README.md
rm -rf "$pack"
(cd "dist/cdn/releases/$version" && shasum -a 256 "$archive" >"$archive.sha256")
mkdir -p "dist/cdn/releases/$platform"
printf '%s\n' "$version" >"dist/cdn/releases/$platform/latest"
