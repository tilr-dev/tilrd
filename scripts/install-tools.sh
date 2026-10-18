#!/bin/sh
# Install the build tools tilrd needs besides Rust, at pinned versions checked against
# pinned SHA-256 sums: Zig (libghostty-vt is built with it) and protoc (tilr-proto's
# build.rs). Used by .github/workflows/ci.yml, release.yml and scripts/linux.Dockerfile.
#
#   scripts/install-tools.sh DIR [TOOL...]     TOOL: zig, protoc (default: both)
#
# Puts DIR/zig and DIR/protoc in place (DIR is created) for this machine's OS and
# architecture: Linux or macOS, x86_64 or aarch64. Add DIR to PATH afterwards.
#
# Zig: the version libghostty-vt-sys 0.2.2 builds Ghostty with (minimum_zig_version of
# its pinned Ghostty commit); the sums are ziglang.org/download/index.json's.
# protoc: the sums are the GitHub release assets' digests. Both are bumped by hand
# (Dependabot does not see them).
set -eu

ZIG_VERSION=0.15.2
PROTOC_VERSION=36.2

[ "$#" -ge 1 ] || { sed -n '6p' "$0" >&2; exit 2; }
dir=$1
shift
[ "$#" -gt 0 ] || set -- zig protoc
mkdir -p "$dir"
dir=$(CDPATH='' cd -- "$dir" && pwd -P)

case "$(uname -s)-$(uname -m)" in
    Linux-x86_64)
        zig_platform=x86_64-linux
        zig_sha256=02aa270f183da276e5b5920b1dac44a63f1a49e55050ebde3aecc9eb82f93239
        protoc_platform=linux-x86_64
        protoc_sha256=121f6c7afe1d4d0e3ea6aab9432038599250134cbf4474cb1167d2c7decd4278
        ;;
    Linux-aarch64 | Linux-arm64)
        zig_platform=aarch64-linux
        zig_sha256=958ed7d1e00d0ea76590d27666efbf7a932281b3d7ba0c6b01b0ff26498f667f
        protoc_platform=linux-aarch_64
        protoc_sha256=8b8f18bd2b30346efbc698dd5a73dd7c805f3ef8380f6dfc95c768f3f1852f6a
        ;;
    Darwin-arm64)
        zig_platform=aarch64-macos
        zig_sha256=3cc2bab367e185cdfb27501c4b30b1b0653c28d9f73df8dc91488e66ece5fa6b
        protoc_platform=osx-universal_binary
        protoc_sha256=83d0dc80672f53486f54299bca6177159f9cad1fd54d4d3143eeea53c4423e1b
        ;;
    Darwin-x86_64)
        zig_platform=x86_64-macos
        zig_sha256=375b6909fc1495d16fc2c7db9538f707456bfc3373b14ee83fdd3e22b3d43f7f
        protoc_platform=osx-universal_binary
        protoc_sha256=83d0dc80672f53486f54299bca6177159f9cad1fd54d4d3143eeea53c4423e1b
        ;;
    *) echo "unsupported platform: $(uname -s)-$(uname -m)" >&2; exit 2 ;;
esac

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

# fetch URL SHA256 FILE: download URL to FILE and check it against SHA256.
fetch() {
    curl -sSfL --retry 3 -o "$3" "$1"
    if command -v sha256sum >/dev/null 2>&1; then
        got=$(sha256sum "$3" | cut -d' ' -f1)
    else
        got=$(shasum -a 256 "$3" | cut -d' ' -f1)
    fi
    [ "$got" = "$2" ] || { echo "SHA-256 mismatch for $1: got $got, want $2" >&2; exit 1; }
}

for tool in "$@"; do
    case "$tool" in
        zig)
            name="zig-$zig_platform-$ZIG_VERSION"
            fetch "https://ziglang.org/download/$ZIG_VERSION/$name.tar.xz" "$zig_sha256" "$tmp/zig.tar.xz"
            rm -rf "${dir:?}/$name"
            tar -xJf "$tmp/zig.tar.xz" -C "$dir"
            ln -sf "$name/zig" "$dir/zig"
            ;;
        protoc)
            fetch "https://github.com/protocolbuffers/protobuf/releases/download/v$PROTOC_VERSION/protoc-$PROTOC_VERSION-$protoc_platform.zip" \
                "$protoc_sha256" "$tmp/protoc.zip"
            rm -rf "${dir:?}/protoc-$PROTOC_VERSION"
            unzip -q "$tmp/protoc.zip" -d "$dir/protoc-$PROTOC_VERSION"
            ln -sf "protoc-$PROTOC_VERSION/bin/protoc" "$dir/protoc"
            ;;
        *) echo "unknown tool: $tool" >&2; exit 2 ;;
    esac
done
