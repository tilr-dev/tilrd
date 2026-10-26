#!/bin/sh
# Deterministic regression for the macOS SDK selection used by pinned Zig.
set -eu

repo=$(CDPATH='' cd -- "$(dirname "$0")/.." && pwd -P)
fixture=$(mktemp -d)
trap 'rm -rf "$fixture"' EXIT
mkdir -p "$fixture/bin"

cat > "$fixture/bin/uname" <<'EOF'
#!/bin/sh
case "$1" in
    -s) printf '%s\n' "${MOCK_HOST:-Darwin}" ;;
    -m) printf '%s\n' arm64 ;;
    *) exit 2 ;;
esac
EOF
cat > "$fixture/bin/xcrun" <<'EOF'
#!/bin/sh
test "$1 $2 $3" = '--sdk macosx15.4 --show-sdk-path' || exit 2
test "${MOCK_SDK_MISSING:-0}" = 0 || exit 1
case "$DEVELOPER_DIR" in
    /Library/Developer/CommandLineTools)
        printf '%s\n' /Library/Developer/CommandLineTools/SDKs/MacOSX15.4.sdk ;;
    /Applications/Xcode_16.3.app/Contents/Developer)
        printf '%s\n' /Applications/Xcode_16.3.app/Contents/Developer/Platforms/MacOSX.platform/Developer/SDKs/MacOSX15.4.sdk ;;
    *) exit 2 ;;
esac
EOF
cat > "$fixture/bin/curl" <<'EOF'
#!/bin/sh
: > "$MOCK_CURL_MARKER"
exit 23
EOF
chmod +x "$fixture/bin/uname" "$fixture/bin/xcrun" "$fixture/bin/curl"

export PATH="$fixture/bin:$PATH"
export MOCK_CURL_MARKER="$fixture/curl-called"

if MOCK_SDK_MISSING=1 "$repo/scripts/install-tools.sh" "$fixture/tools" zig > "$fixture/missing.log" 2>&1; then
    echo 'expected missing macOS 15.4 SDK to fail' >&2
    exit 1
fi
grep -F 'MacOSX15.4.sdk' "$fixture/missing.log" >/dev/null || {
    echo 'missing SDK failure lacks setup guidance' >&2
    exit 1
}
test ! -e "$MOCK_CURL_MARKER" || {
    echo 'installer downloaded Zig before checking SDK' >&2
    exit 1
}

if MOCK_SDK_MISSING=0 "$repo/scripts/install-tools.sh" "$fixture/tools" zig > "$fixture/present.log" 2>&1; then
    echo 'curl fixture unexpectedly succeeded' >&2
    exit 1
fi
test -e "$MOCK_CURL_MARKER" || {
    echo 'installer did not continue after finding SDK' >&2
    exit 1
}
rm "$MOCK_CURL_MARKER"

DEVELOPER_DIR=/Applications/Xcode_16.3.app/Contents/Developer
SDKROOT=/Applications/Xcode_16.3.app/Contents/Developer/Platforms/MacOSX.platform/Developer/SDKs/MacOSX15.4.sdk
export DEVELOPER_DIR SDKROOT
if "$repo/scripts/install-tools.sh" "$fixture/tools" zig > "$fixture/xcode.log" 2>&1; then
    echo 'curl fixture unexpectedly succeeded with Xcode SDK' >&2
    exit 1
fi
test -e "$MOCK_CURL_MARKER" || {
    echo 'installer did not accept the selected Xcode macOS 15.4 SDK' >&2
    exit 1
}
rm "$MOCK_CURL_MARKER"
unset DEVELOPER_DIR SDKROOT

if MOCK_HOST=Linux "$repo/scripts/install-tools.sh" "$fixture/tools" zig > "$fixture/linux.log" 2>&1; then
    echo 'curl fixture unexpectedly succeeded on Linux' >&2
    exit 1
fi
test -e "$MOCK_CURL_MARKER" || {
    echo 'Linux installer unexpectedly required macOS SDK' >&2
    exit 1
}

python3 - "$repo/.cargo/config.toml" <<'PY'
import pathlib
import sys
import tomllib

config = tomllib.loads(pathlib.Path(sys.argv[1]).read_text())
expected = {
    "DEVELOPER_DIR": "/Library/Developer/CommandLineTools",
    "SDKROOT": "/Library/Developer/CommandLineTools/SDKs/MacOSX15.4.sdk",
}
for name, value in expected.items():
    assert config["env"][name] == {"value": value, "force": False}, name
PY

if git -C "$repo" rev-parse --is-inside-work-tree >/dev/null 2>&1; then
    if git -C "$repo" check-ignore --no-index -q .cargo/config.toml; then
        echo 'Cargo SDK configuration is ignored and will be absent in worktrees' >&2
        exit 1
    fi
else
    grep -Fx '!/.cargo/config.toml' "$repo/.gitignore" >/dev/null || {
        echo 'Cargo SDK configuration is not included by .gitignore' >&2
        exit 1
    }
fi

echo 'macOS SDK configuration and installer preflight: ok'
