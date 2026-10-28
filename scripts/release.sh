#!/bin/sh
# Build a release and lay it out as on the CDN, in dist/cdn. With --publish,
# upload it to the tilr-cdn R2 bucket (https://cdn.tilr.dev), where the
# installer and `tilr update` find it:
#
#   releases/<platform>/latest                                 the version ready for the platform
#   releases/<version>/tilr-<version>-<platform>.tar.gz        tilrd and tilr
#   releases/<version>/tilr-<version>-<platform>.tar.gz.sha256
#   install.sh                                                 scripts/install.sh (uploaded, not
#                                                              laid out), served by tilr.sh
#
# The archive holds the server tilrd and its client tilr at its top level,
# with LICENSE-APACHE, LICENSE-MIT and README.md; both programs are installed
# and updated together.
#
# A version is the package version plus the commit, e.g. 0.1.0-26449dd; the package
# version is TILR_BUILD_VERSION if set (the release workflow sets the tag's, e.g.
# 0.3.0-rc.1), else the root Cargo.toml's. A
# platform is named as the binary names itself: linux-aarch64, linux-x86_64,
# macos-aarch64, macos-x86_64.
# --channel stable (default) publishes latest and install.sh; --channel beta
# publishes beta only. Both use the same local staging layout with latest.
# Each platform has its own pointer, so a release can be out for one and not
# yet for another.
#
#   scripts/release.sh                       # build for linux-aarch64
#   scripts/release.sh --publish             # build and upload
#   scripts/release.sh --publish linux-aarch64 linux-x86_64
#   scripts/release.sh --upload-only --channel beta linux-aarch64 linux-x86_64
#                                            # upload what earlier runs left in dist/cdn
#
# A build also leaves the two binaries in dist/<platform>/ (scripts/linux-build.sh or
# scripts/macos-build.sh),
# which .github/workflows/release.yml packs for the GitHub release.
#
# --upload-only builds nothing: it uploads dist/cdn as earlier runs of this
# script laid it out (one per platform, e.g. on separate machines, as
# .github/workflows/release.yml does), after checking that every platform has
# the same version and that the archives match their checksums. Only the
# platforms given are checked and uploaded; the `latest` of any other platform
# on the CDN is left as it is.
#
# Uploading uses AWS CLI with AWS_ACCESS_KEY_ID and AWS_SECRET_ACCESS_KEY derived
# from the R2 token scoped to object read/write in tilr-cdn. --publish and
# --upload-only also need CLOUDFLARE_ACCOUNT_ID exported (the bucket's account;
# in CI, the CLOUDFLARE_ACCOUNT_ID variable of the cdn environment).
# By hand, Infisical /cloudflare (prod) supplies CF_ACCOUNT_ID and R2_CDN_TOKEN.
# Set CLOUDFLARE_ACCOUNT_ID from CF_ACCOUNT_ID, AWS_ACCESS_KEY_ID to the token ID,
# and AWS_SECRET_ACCESS_KEY to the lowercase hex SHA-256 of R2_CDN_TOKEN (no newline).
# See README's Releases section for the Infisical command and key derivation example.
set -eu
cd "$(dirname "$0")/.."

build=1
publish=
case "${1:-}" in
    --publish) publish=1; shift ;;
    --upload-only) build=; publish=1; shift ;;
esac
channel=stable
if [ "${1:-}" = --channel ]; then
    channel=${2:-}
    [ "$#" -ge 2 ] || { echo "--channel needs stable or beta" >&2; exit 2; }
    shift 2
fi
case "$channel" in
    stable) pointer=latest ;;
    beta) pointer=beta ;;
    *) echo "unknown channel: $channel (expected stable or beta)" >&2; exit 2 ;;
esac
[ "$#" -gt 0 ] || set -- linux-aarch64
if [ -n "$publish" ] && [ -z "${CLOUDFLARE_ACCOUNT_ID:-}" ]; then
    echo "CLOUDFLARE_ACCOUNT_ID is not set: export the account id of the tilr-cdn bucket" \
        "(see README Releases for Infisical setup and derived AWS keys)" >&2
    exit 2
fi

for platform in "$@"; do
    case "$platform" in
        linux-aarch64 | linux-x86_64 | macos-aarch64 | macos-x86_64) ;;
        *) echo "unknown platform: $platform" >&2; exit 2 ;;
    esac
done

if [ -n "$build" ]; then
    if [ -n "$(git status --porcelain)" ]; then
        echo "uncommitted changes: a release is built from a commit" >&2
        exit 1
    fi
    commit=$(git rev-parse --short HEAD)
    package=${TILR_BUILD_VERSION:-$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -1)}
    version="$package-$commit"

    for platform in "$@"; do
        case "$platform" in
            linux-aarch64) docker_platform=linux/arm64 ;;
            linux-x86_64) docker_platform=linux/amd64 ;;
        esac
        case "$platform" in
            linux-*)
                # The container cannot read the host worktree's Git metadata.
                TILR_BUILD_VERSION=$package TILR_BUILD_COMMIT=$commit \
                    TILR_LINUX_PLATFORM=$docker_platform scripts/linux-build.sh ;;
            macos-*)
                TILR_BUILD_VERSION=$package TILR_BUILD_COMMIT=$commit \
                    scripts/macos-build.sh "$platform" ;;
        esac
        scripts/cdn-layout.sh "$version" "$platform"
    done

    echo "built $version for: $*"
else
    version=
    for platform in "$@"; do
        latest="dist/cdn/releases/$platform/latest"
        [ -f "$latest" ] || { echo "nothing built for $platform: no $latest" >&2; exit 1; }
        this=$(cat "$latest")
        [ -z "$version" ] || [ "$this" = "$version" ] ||
            { echo "$platform is built as $this, not $version" >&2; exit 1; }
        version=$this
        archive="tilr-$version-$platform.tar.gz"
        (cd "dist/cdn/releases/$version" && shasum -a 256 -c "$archive.sha256")
    done
fi
[ -n "$publish" ] || exit 0

# S3 object access works with bucket-scoped credentials.
put() {
    aws s3 cp "$4" "s3://tilr-cdn/$1" \
        --endpoint-url "https://${CLOUDFLARE_ACCOUNT_ID}.r2.cloudflarestorage.com" \
        --region auto --content-type "$2" --cache-control "$3"
}
for platform in "$@"; do
    name="releases/$version/tilr-$version-$platform.tar.gz"
    put "$name" application/gzip "public, max-age=31536000, immutable" "dist/cdn/$name"
    put "$name.sha256" "text/plain; charset=utf-8" "public, max-age=31536000, immutable" "dist/cdn/$name.sha256"
    # Last: from here on the platform is offered this version.
    put "releases/$platform/$pointer" "text/plain; charset=utf-8" "no-cache" "dist/cdn/releases/$platform/latest"
done
# After every latest, so the installer never offers what the CDN does not have
# yet; never after a failed upload (set -e).
if [ "$channel" = stable ]; then
    put install.sh "text/x-shellscript; charset=utf-8" "public, max-age=300" scripts/install.sh
fi
echo "published $version for: $*"
