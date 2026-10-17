#!/bin/sh
# Build release binaries and lay them out as on the CDN, in dist/cdn. With
# --publish, upload them to the tilr-cdn R2 bucket (https://cdn.tilr.dev),
# where `tilrd update` finds them:
#
#   releases/<platform>/latest                   the version ready for the platform
#   releases/<version>/tilrd-<platform>
#   releases/<version>/tilrd-<platform>.sha256
#   releases/<version>/tilr-server-<platform>          the same file, for servers
#   releases/<version>/tilr-server-<platform>.sha256   from before the rename
#   releases/<version>/tilr-<platform>           the client, which the installer
#   releases/<version>/tilr-<platform>.sha256    puts next to tilrd
#
# A server from before the rename to tilrd asks for tilr-server-<platform> and
# puts it in place under its own name; from then on it is tilrd and asks for
# tilrd-<platform>. The old name stays published while such servers are around.
#
# A version is the package version plus the commit, e.g. 0.1.0-26449dd. A
# platform is named as the binary names itself: linux-aarch64, linux-x86_64.
# Each platform has its own `latest`, so a release can be out for one and not
# yet for another.
#
#   scripts/release.sh                       # build for linux-aarch64
#   scripts/release.sh --publish             # build and upload
#   scripts/release.sh --publish linux-aarch64 linux-x86_64
set -eu
cd "$(dirname "$0")/.."

publish=
if [ "${1:-}" = --publish ]; then
    publish=1
    shift
fi
[ "$#" -gt 0 ] || set -- linux-aarch64

if [ -n "$(git status --porcelain)" ]; then
    echo "uncommitted changes: a release is built from a commit" >&2
    exit 1
fi
commit=$(git rev-parse --short HEAD)
package=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -1)
version="$package-$commit"
mkdir -p "dist/cdn/releases/$version"

for platform in "$@"; do
    case "$platform" in
        linux-aarch64) docker_platform=linux/arm64 ;;
        linux-x86_64) docker_platform=linux/amd64 ;;
        *) echo "unknown platform: $platform" >&2; exit 2 ;;
    esac
    name="tilrd-$platform"
    # The container does not own the source tree, so git there refuses to
    # say what the commit is.
    TILR_BUILD_COMMIT=$commit TILR_LINUX_PLATFORM=$docker_platform scripts/linux-build.sh
    for file in "$name" "tilr-server-$platform"; do
        cp "dist/$name" "dist/cdn/releases/$version/$file"
        (cd "dist/cdn/releases/$version" && shasum -a 256 "$file" >"$file.sha256")
    done
    client="tilr-$platform"
    cp "dist/$client" "dist/cdn/releases/$version/$client"
    (cd "dist/cdn/releases/$version" && shasum -a 256 "$client" >"$client.sha256")
    mkdir -p "dist/cdn/releases/$platform"
    printf '%s\n' "$version" >"dist/cdn/releases/$platform/latest"
done

echo "built $version for: $*"
[ -n "$publish" ] || exit 0

# The account the tilr-cdn bucket is in, as in wrangler.toml of the sh
# repository.
export CLOUDFLARE_ACCOUNT_ID="${CLOUDFLARE_ACCOUNT_ID:-8e0530af975a33f2f739127f75084531}"
put() {
    wrangler r2 object put "tilr-cdn/$1" --file "dist/cdn/$1" \
        --content-type "$2" --cache-control "$3" --remote
}
for platform in "$@"; do
    for name in "releases/$version/tilrd-$platform" "releases/$version/tilr-server-$platform" \
        "releases/$version/tilr-$platform"; do
        put "$name" application/octet-stream "public, max-age=31536000, immutable"
        put "$name.sha256" "text/plain; charset=utf-8" "public, max-age=31536000, immutable"
    done
    # Last: from here on the platform is offered this version.
    put "releases/$platform/latest" "text/plain; charset=utf-8" "no-cache"
done
echo "published $version for: $*"
