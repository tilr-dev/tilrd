#!/bin/sh
# Build a release and lay it out as on the CDN, in dist/cdn. With --publish,
# upload it to the tilr-cdn R2 bucket (https://cdn.tilr.dev), where the
# installer and `tilr update` find it:
#
#   releases/<platform>/latest                                 the version ready for the platform
#   releases/<version>/tilr-<version>-<platform>.tar.gz        tilrd and tilr
#   releases/<version>/tilr-<version>-<platform>.tar.gz.sha256
#
# The archive holds the server tilrd and its client tilr at its top level
# (and LICENSE and README.md, when there are such files); both are installed
# and updated together.
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
    # The container does not own the source tree, so git there refuses to
    # say what the commit is.
    TILR_BUILD_COMMIT=$commit TILR_LINUX_PLATFORM=$docker_platform scripts/linux-build.sh
    archive="tilr-$version-$platform.tar.gz"
    pack="dist/pack-$platform"
    rm -rf "$pack" && mkdir -p "$pack"
    cp "dist/$platform/tilrd" "dist/$platform/tilr" "$pack/"
    chmod 755 "$pack/tilrd" "$pack/tilr"
    extra=
    for file in LICENSE README.md; do
        if [ -f "$file" ]; then
            cp "$file" "$pack/"
            extra="$extra $file"
        fi
    done
    # shellcheck disable=SC2086 # extra is a list of plain file names
    COPYFILE_DISABLE=1 tar -czf "dist/cdn/releases/$version/$archive" -C "$pack" tilrd tilr $extra
    rm -rf "$pack"
    (cd "dist/cdn/releases/$version" && shasum -a 256 "$archive" >"$archive.sha256")
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
    name="releases/$version/tilr-$version-$platform.tar.gz"
    put "$name" application/gzip "public, max-age=31536000, immutable"
    put "$name.sha256" "text/plain; charset=utf-8" "public, max-age=31536000, immutable"
    # Last: from here on the platform is offered this version.
    put "releases/$platform/latest" "text/plain; charset=utf-8" "no-cache"
done
echo "published $version for: $*"
