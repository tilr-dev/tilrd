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
# A version is the package version plus the commit, e.g. 0.1.0-26449dd. A
# platform is named as the binary names itself: linux-aarch64, linux-x86_64.
# Each platform has its own `latest`, so a release can be out for one and not
# yet for another.
#
#   scripts/release.sh                       # build for linux-aarch64
#   scripts/release.sh --publish             # build and upload
#   scripts/release.sh --publish linux-aarch64 linux-x86_64
#   scripts/release.sh --upload-only linux-aarch64 linux-x86_64
#                                            # upload what earlier runs left in dist/cdn
#
# A build also leaves the two binaries in dist/<platform>/ (scripts/linux-build.sh),
# which .github/workflows/release.yml packs for the GitHub release.
#
# --upload-only builds nothing: it uploads dist/cdn as earlier runs of this
# script laid it out (one per platform, e.g. on separate machines, as
# .github/workflows/release.yml does), after checking that every platform has
# the same version and that the archives match their checksums. Only the
# platforms given are checked and uploaded; the `latest` of any other platform
# on the CDN is left as it is.
#
# Uploading uses wrangler, which needs Cloudflare credentials: `wrangler login`
# by hand, or CLOUDFLARE_API_TOKEN (an API token with R2 write access to the
# bucket's account) in CI. --publish and --upload-only also need
# CLOUDFLARE_ACCOUNT_ID, the account the tilr-cdn bucket is in, exported in the
# environment (e.g. via `infisical run --path /cloudflare -- scripts/release.sh ...`;
# in CI, the CLOUDFLARE_ACCOUNT_ID variable of the `cdn` environment).
set -eu
cd "$(dirname "$0")/.."

build=1
publish=
case "${1:-}" in
    --publish) publish=1; shift ;;
    --upload-only) build=; publish=1; shift ;;
esac
[ "$#" -gt 0 ] || set -- linux-aarch64
if [ -n "$publish" ] && [ -z "${CLOUDFLARE_ACCOUNT_ID:-}" ]; then
    echo "CLOUDFLARE_ACCOUNT_ID is not set: export the account id of the tilr-cdn bucket" \
        "(e.g. via \`infisical run --path /cloudflare\`)" >&2
    exit 2
fi

for platform in "$@"; do
    case "$platform" in
        linux-aarch64 | linux-x86_64) ;;
        *) echo "unknown platform: $platform" >&2; exit 2 ;;
    esac
done

if [ -n "$build" ]; then
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
        esac
        # The container does not own the source tree, so git there refuses to
        # say what the commit is.
        TILR_BUILD_COMMIT=$commit TILR_LINUX_PLATFORM=$docker_platform scripts/linux-build.sh
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

# The account the tilr-cdn bucket is in, checked above.
export CLOUDFLARE_ACCOUNT_ID
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
# After every latest, so the installer never offers what the CDN does not have
# yet; never after a failed upload (set -e).
wrangler r2 object put tilr-cdn/install.sh --file scripts/install.sh \
    --content-type "text/x-shellscript; charset=utf-8" --cache-control "public, max-age=300" --remote
echo "published $version for: $*"
