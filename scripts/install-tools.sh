#!/bin/sh
# Install the build tools tilrd needs besides Rust, at pinned versions checked against
# pinned SHA-256 sums: Zig (libghostty-vt is built with it), protoc (tilr-proto's build.rs)
# and cargo-nextest (runs the tests); deno and uv for the bus SDKs in sdk/ (their tests,
# scripts/gen-bus-sdk.sh, scripts/sdk-check.sh). Used by ci.yml, release.yml and
# linux.Dockerfile.
#
#   scripts/install-tools.sh DIR [TOOL...]     TOOL: zig, protoc, nextest, deno, uv
#                                              (default: zig, protoc, nextest)
#
# Puts DIR/zig, DIR/protoc, DIR/cargo-nextest, DIR/deno and DIR/uv in place (DIR is
# created) for this machine's OS and architecture: Linux or macOS, x86_64 or aarch64. Add
# DIR to PATH afterwards.
#
# Zig: the version libghostty-vt-sys 0.2.2 builds Ghostty with (minimum_zig_version of
# its pinned Ghostty commit); the sums are ziglang.org/download/index.json's.
# protoc, cargo-nextest, deno and uv: the sums are the GitHub release assets' digests
# (nextest's macOS archive is one universal binary). All are bumped by hand (Dependabot
# does not see them).
set -eu

ZIG_VERSION=0.15.2
PROTOC_VERSION=36.2
NEXTEST_VERSION=0.9.149
DENO_VERSION=2.9.8
UV_VERSION=0.13.5

[ "$#" -ge 1 ] || { sed -n '8,9p' "$0" >&2; exit 2; }
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
        rust_platform=x86_64-unknown-linux-gnu
        deno_sha256=e890975bc2549807bbcc0b584ffafda72cc7c93349dd99c80483dd784f6c4609
        uv_sha256=f68d291c75783122dad58ab77def0e13f3233f358731db5d6d7e8715603b928f
        ;;
    Linux-aarch64 | Linux-arm64)
        zig_platform=aarch64-linux
        zig_sha256=958ed7d1e00d0ea76590d27666efbf7a932281b3d7ba0c6b01b0ff26498f667f
        protoc_platform=linux-aarch_64
        protoc_sha256=8b8f18bd2b30346efbc698dd5a73dd7c805f3ef8380f6dfc95c768f3f1852f6a
        nextest_platform=aarch64-unknown-linux-gnu
        nextest_sha256=6f82e375187cf3a1f3e336879a51940c315a5d0ce7253f7e517ad28295b74189
        rust_platform=aarch64-unknown-linux-gnu
        deno_sha256=d49596c20b9788cf8f5d8ddf644103d4bde56ed239753f0bdb92ba9361d51905
        uv_sha256=4bb69cf95ed13f23713bb7bc09f241896ea0c75b7c557e615a20604b82f9c403
        ;;
    Darwin-arm64)
        zig_platform=aarch64-macos
        zig_sha256=3cc2bab367e185cdfb27501c4b30b1b0653c28d9f73df8dc91488e66ece5fa6b
        protoc_platform=osx-universal_binary
        protoc_sha256=83d0dc80672f53486f54299bca6177159f9cad1fd54d4d3143eeea53c4423e1b
        nextest_platform=universal-apple-darwin
        nextest_sha256=47b9941765a9ff94e304a3fe5444ac1c7593da655525b8b69f8f2b31a9217507
        rust_platform=aarch64-apple-darwin
        deno_sha256=9f0bf660393da2eaff044cdc7444cb08c73f235849b61d224d825a194ce7c98e
        uv_sha256=743f85035a1996ecb350659e01cd9faea9709829be09b1b4b03c73eaf3c9cd73
        ;;
    Darwin-x86_64)
        zig_platform=x86_64-macos
        zig_sha256=375b6909fc1495d16fc2c7db9538f707456bfc3373b14ee83fdd3e22b3d43f7f
        protoc_platform=osx-universal_binary
        protoc_sha256=83d0dc80672f53486f54299bca6177159f9cad1fd54d4d3143eeea53c4423e1b
        nextest_platform=universal-apple-darwin
        nextest_sha256=47b9941765a9ff94e304a3fe5444ac1c7593da655525b8b69f8f2b31a9217507
        rust_platform=x86_64-apple-darwin
        deno_sha256=4b2f5edd2639b7470f034238b321d7002d8dfb4726d8558f52c17afac370eb2c
        uv_sha256=b0f52b0e3945b4107d89024ea38e625683011fbfc05a0d98f9fa2d6dcee28c66
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
        deno)
            fetch "https://github.com/denoland/deno/releases/download/v$DENO_VERSION/deno-$rust_platform.zip" \
                "$deno_sha256" "$tmp/deno.zip"
            rm -rf "${dir:?}/deno-$DENO_VERSION"
            unzip -q "$tmp/deno.zip" -d "$dir/deno-$DENO_VERSION"
            ln -sf "deno-$DENO_VERSION/deno" "$dir/deno"
            ;;
        uv)
            fetch "https://github.com/astral-sh/uv/releases/download/$UV_VERSION/uv-$rust_platform.tar.gz" \
                "$uv_sha256" "$tmp/uv.tar.gz"
            rm -rf "${dir:?}/uv-$UV_VERSION"
            mkdir "$dir/uv-$UV_VERSION"
            tar -xzf "$tmp/uv.tar.gz" -C "$dir/uv-$UV_VERSION" --strip-components=1
            ln -sf "uv-$UV_VERSION/uv" "$dir/uv"
            ;;
        *) echo "unknown tool: $tool" >&2; exit 2 ;;
    esac
done
