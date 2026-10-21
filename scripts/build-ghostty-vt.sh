#!/usr/bin/env bash
# Print the completed shared prefix on stdout; build diagnostics go to stderr.
set -euo pipefail

# Keep in sync with libghostty-vt-sys 0.2.2 build.rs and the Cargo SDK defaults.
revision=a887df42c56f6de86c0fe6da9c4eeca37931e083
host=$(uname -s)
arch=$(uname -m)
zig_version=$(zig version)
sdk=none
developer=none
if [[ "$host" == Darwin ]]; then
    export DEVELOPER_DIR=${DEVELOPER_DIR:-/Library/Developer/CommandLineTools}
    export SDKROOT=${SDKROOT:-/Library/Developer/CommandLineTools/SDKs/MacOSX15.4.sdk}
    [[ -d "$SDKROOT" ]] || { echo "Missing SDK: $SDKROOT; set DEVELOPER_DIR and SDKROOT to the macOS 15.4 SDK." >&2; exit 1; }
    sdk=$(cd "$SDKROOT" && pwd -P)
    developer=$DEVELOPER_DIR
fi
cache=${TILR_GHOSTTY_VT_CACHE:-$HOME/.cache/tilr/ghostty-vt}
mkdir -p "$cache"
cache=$(cd "$cache" && pwd -P)
key=$(printf '%s\n' "$revision" "$zig_version" "$host" "$arch" "$sdk" "$developer" ReleaseFast baseline | git hash-object --stdin)
prefix=$cache/$revision/$key
if [[ -f "$prefix/.complete" ]]; then
    printf '%s\n' "$prefix"
    exit 0
fi
mkdir -p "$(dirname "$prefix")"
lock=${prefix}.lock
mkdir "$lock" 2>/dev/null || { echo "Build already in progress: $lock; rerun after it finishes." >&2; exit 1; }
work=
trap 'rm -rf "$work"; rmdir "$lock"' EXIT
if [[ -f "$prefix/.complete" ]]; then
    printf '%s\n' "$prefix"
    exit 0
fi
work=$(mktemp -d "${prefix}.tmp.XXXXXX")
{
    git init -q "$work/source"
    git -C "$work/source" remote add origin https://github.com/ghostty-org/ghostty.git
    git -C "$work/source" fetch --depth 1 origin "$revision"
    git -C "$work/source" checkout --detach FETCH_HEAD
    cd "$work/source"
    zig build -Demit-lib-vt=true -Doptimize=ReleaseFast -Dcpu=baseline \
        -Demit-xcframework=false -Dapp-runtime=none --prefix "$work/install" --cache-dir "$work/zig-cache"
    test -f "$work/install/lib/libghostty-vt.a"
    test -f "$work/install/share/pkgconfig/libghostty-vt-static.pc"
    # Upstream embeds the temporary install prefix. Make the package relocatable.
    pc=$work/install/share/pkgconfig/libghostty-vt-static.pc
    sed 's|^prefix=.*|prefix=${pcfiledir}/../..|' "$pc" > "$pc.new"
    mv "$pc.new" "$pc"
    touch "$work/install/.complete"
    mv "$work/install" "$prefix"
} >&2
printf '%s\n' "$prefix"
