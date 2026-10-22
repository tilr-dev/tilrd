#!/bin/sh
# Install the build tools tilrd needs besides Rust, at pinned versions checked against
# pinned SHA-256 sums: Zig (libghostty-vt is built with it), protoc (tilr-proto's build.rs)
# and cargo-nextest (runs the tests). Used by ci.yml, release.yml and linux.Dockerfile.
#
#   scripts/install-tools.sh DIR [TOOL...]     TOOL: zig, protoc, nextest (default: all)
#
# Puts DIR/zig, DIR/protoc and DIR/cargo-nextest in place (DIR is created) for this
# machine's OS and architecture: Linux or macOS, x86_64 or aarch64. Add DIR to PATH
# afterwards.
#
# Zig: the version libghostty-vt-sys 0.2.2 builds Ghostty with (minimum_zig_version of
# its pinned Ghostty commit); the sums are ziglang.org/download/index.json's.
# protoc and cargo-nextest: the sums are the GitHub release assets' digests (nextest's
# macOS archive is one universal binary). All three are bumped by hand (Dependabot does
# not see them).
set -eu

ZIG_VERSION=0.15.2
PROTOC_VERSION=36.2
NEXTEST_VERSION=0.9.149

[ "$#" -ge 1 ] || { sed -n '6p' "$0" >&2; exit 2; }
dir=$1
shift
[ "$#" -gt 0 ] || set -- zig protoc nextest
mkdir -p "$dir"
dir=$(CDPATH='' cd -- "$dir" && pwd -P)

case "$(uname -s)-$(uname -m)" in
    Linux-x86_64)
        zig_platform=x86_64-linux
        zig_sha256=02aa270f183da276e5b5920b1dac44a63f1a49e55050ebde3aecc9eb82f93239
        protoc_platform=linux-x86_64
        protoc_sha256=121f6c7afe1d4d0e3ea6aab9432038599250134cbf4474cb1167d2c7decd4278
        nextest_platform=x86_64-unknown-linux-gnu
        nextest_sha256=32d1d8c0567b7a80bee2bad8bb09125c846da96bc009b2de11ec08557290c770
        ;;
    Linux-aarch64 | Linux-arm64)
        zig_platform=aarch64-linux
        zig_sha256=958ed7d1e00d0ea76590d27666efbf7a932281b3d7ba0c6b01b0ff26498f667f
        protoc_platform=linux-aarch_64
        protoc_sha256=8b8f18bd2b30346efbc698dd5a73dd7c805f3ef8380f6dfc95c768f3f1852f6a
        nextest_platform=aarch64-unknown-linux-gnu
        nextest_sha256=6f82e375187cf3a1f3e336879a51940c315a5d0ce7253f7e517ad28295b74189
        ;;
    Darwin-arm64)
        zig_platform=aarch64-macos
        zig_sha256=3cc2bab367e185cdfb27501c4b30b1b0653c28d9f73df8dc91488e66ece5fa6b
        protoc_platform=osx-universal_binary
        protoc_sha256=83d0dc80672f53486f54299bca6177159f9cad1fd54d4d3143eeea53c4423e1b
        nextest_platform=universal-apple-darwin
        nextest_sha256=47b9941765a9ff94e304a3fe5444ac1c7593da655525b8b69f8f2b31a9217507
        ;;
    Darwin-x86_64)
        zig_platform=x86_64-macos
        zig_sha256=375b6909fc1495d16fc2c7db9538f707456bfc3373b14ee83fdd3e22b3d43f7f
        protoc_platform=osx-universal_binary
        protoc_sha256=83d0dc80672f53486f54299bca6177159f9cad1fd54d4d3143eeea53c4423e1b
        nextest_platform=universal-apple-darwin
        nextest_sha256=47b9941765a9ff94e304a3fe5444ac1c7593da655525b8b69f8f2b31a9217507
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
            case "$zig_platform" in
                *-macos)
                    developer_dir=${DEVELOPER_DIR:-/Library/Developer/CommandLineTools}
                    required_sdk=${SDKROOT:-/Library/Developer/CommandLineTools/SDKs/MacOSX15.4.sdk}
                    found_sdk=$(DEVELOPER_DIR="$developer_dir" \
                        xcrun --sdk macosx15.4 --show-sdk-path 2>/dev/null) || found_sdk=
                    if [ "$found_sdk" != "$required_sdk" ]; then
                        echo "Zig $ZIG_VERSION needs macOS 15.4 SDK at $required_sdk. Install Command Line Tools containing it, or set DEVELOPER_DIR and SDKROOT to an Xcode installation with macOS 15.4 SDK, then rerun this installer." >&2
                        exit 1
                    fi
                    ;;
            esac
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
        nextest)
            fetch "https://github.com/nextest-rs/nextest/releases/download/cargo-nextest-$NEXTEST_VERSION/cargo-nextest-$NEXTEST_VERSION-$nextest_platform.tar.gz" \
                "$nextest_sha256" "$tmp/nextest.tar.gz"
            rm -rf "${dir:?}/cargo-nextest-$NEXTEST_VERSION"
            mkdir "$dir/cargo-nextest-$NEXTEST_VERSION"
            tar -xzf "$tmp/nextest.tar.gz" -C "$dir/cargo-nextest-$NEXTEST_VERSION"
            ln -sf "cargo-nextest-$NEXTEST_VERSION/cargo-nextest" "$dir/cargo-nextest"
            ;;
        *) echo "unknown tool: $tool" >&2; exit 2 ;;
    esac
done
