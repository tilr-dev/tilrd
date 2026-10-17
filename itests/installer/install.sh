#!/bin/sh
# tilr installer and updater: curl -fsSL https://tilr.sh | sh
# Served from R2: https://cdn.tilr.dev/install.sh
#
# Puts tilrd (the tilr server) and tilr (its client, the command you type) into
# ~/.local/bin and runs tilrd as a systemd user service. Run it again to update:
# a running server picks the new executable up on its own and keeps every shell.
# A release from before the client has no tilr; then only tilrd is installed.
#
# The server was called tilr-server before. An install from then
# (~/.local/bin/tilr-server) moves to tilrd and tilr-server becomes a link to
# it: the running server, which watches the file it started from, takes the
# update through the link and keeps its shells; the service runs tilrd.
#
#   TILR_BIN_DIR     where the executables go (default: ~/.local/bin)
#   TILR_NO_SERVICE  set to 1 to install the executables only
#   TILR_CDN         where releases are fetched from
set -eu

CDN="${TILR_CDN:-https://cdn.tilr.dev}"
BIN_DIR="${TILR_BIN_DIR:-$HOME/.local/bin}"
UNIT_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user"

say() {
  echo "tilr: $*" >&2
}

die() {
  say "$*"
  exit 1
}

fetch() {
  if command -v curl >/dev/null 2>&1; then
    curl -fsSL "$1"
  elif command -v wget >/dev/null 2>&1; then
    wget -qO- "$1"
  else
    die "need curl or wget"
  fi
}

sha256() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | cut -d' ' -f1
  else
    shasum -a 256 "$1" | cut -d' ' -f1
  fi
}

# Download the release into $bin unless it is there already. The new file is
# renamed into place, which is what a running server watches for. A release is
# published as tilrd-<platform>; one from before the rename only as
# tilr-server-<platform>.
install_binary() {
  version=$(fetch "$CDN/releases/linux-$arch/latest") || die "no release for linux $arch at $CDN"
  url="$CDN/releases/$version/tilrd-linux-$arch"
  if ! sum=$(fetch "$url.sha256" 2>/dev/null); then
    url="$CDN/releases/$version/tilr-server-linux-$arch"
    sum=$(fetch "$url.sha256") || die "no release $version for linux $arch"
  fi
  want=${sum%% *}

  if [ -x "$bin" ] && [ "$(sha256 "$bin")" = "$want" ]; then
    say "tilrd $version is already installed"
    return
  fi

  mkdir -p "$BIN_DIR"
  new="$bin.new.$$"
  trap 'rm -f "$new"' EXIT
  say "downloading tilrd $version (linux $arch)"
  fetch "$url" >"$new" || die "download failed"
  [ "$(sha256 "$new")" = "$want" ] || die "checksum mismatch, nothing installed"
  chmod 755 "$new"
  "$new" --version >/dev/null 2>&1 ||
    die "tilrd $version does not run on this system (it needs glibc 2.36 or newer), nothing installed"
  mv "$new" "$bin"
  trap - EXIT
  updated=1
  say "installed $bin"
}

# The client, tilr-<platform> of the same release, next to tilrd (tilr starts
# the tilrd next to it when none runs). A release without it is no error: the
# client is left as it is, or not installed.
install_client() {
  url="$CDN/releases/$version/tilr-linux-$arch"
  if ! sum=$(fetch "$url.sha256" 2>/dev/null); then
    say "release $version has no tilr client yet; only tilrd is installed"
    [ ! -x "$client" ] || has_client=1
    return
  fi
  want=${sum%% *}

  if [ -x "$client" ] && [ "$(sha256 "$client")" = "$want" ]; then
    say "tilr $version is already installed"
    has_client=1
    return
  fi

  # tilrd is in by now, so a client that fails is said and skipped.
  new="$client.new.$$"
  trap 'rm -f "$new"' EXIT
  say "downloading tilr $version (linux $arch)"
  if ! fetch "$url" >"$new"; then
    problem="download failed"
  elif [ "$(sha256 "$new")" != "$want" ]; then
    problem="checksum mismatch"
  elif ! chmod 755 "$new" || ! "$new" --version >/dev/null 2>&1; then
    problem="it does not run on this system"
  else
    problem=
  fi
  if [ -n "$problem" ]; then
    rm -f "$new"
    trap - EXIT
    say "tilr not installed: $problem"
    [ -x "$client" ] && has_client=1
    return 0
  fi
  mv "$new" "$client"
  trap - EXIT
  has_client=1
  say "installed $client"
}

# What to type next: tilr, if it is there.
hints() {
  if [ -n "$has_client" ]; then
    case ":$PATH:" in
      *":$BIN_DIR:"*) ;;
      *) say "$BIN_DIR is not on your PATH; add it to run tilr" ;;
    esac
    say "open a shell: tilr; the access token for the app: tilr token; link this machine to Tilr Cloud: tilr cloud login"
  else
    say "the access token for the app: $bin token; link this machine to Tilr Cloud: $bin cloud login"
  fi
}

# An install from before the rename: tilr-server, a file, and no tilrd yet.
# tilrd becomes a hard link to it (the same file, so a running server sees no
# change) before the download, and tilr-server a link to tilrd after it.
adopt_legacy() {
  [ -f "$legacy" ] && [ ! -L "$legacy" ] && [ ! -e "$bin" ] || return 0
  ln "$legacy" "$bin" 2>/dev/null || cp -p "$legacy" "$bin" || die "cannot move $legacy to $bin"
  migrated=1
}

link_legacy() {
  [ -n "$migrated" ] || return 0
  { ln -s tilrd "$legacy.new.$$" && mv -f "$legacy.new.$$" "$legacy"; } ||
    die "cannot link $legacy to $bin"
  say "tilr-server is tilrd now: $legacy links to $bin"
}

# The server starts from a login shell so that panes get the PATH and the
# environment of ~/.profile, as they would over SSH.
write_unit() {
  cat <<EOF
[Unit]
Description=tilr terminal server

[Service]
ExecStart=/bin/sh -lc 'exec "$bin"'
Restart=on-failure

[Install]
WantedBy=default.target
EOF
}

install_service() {
  if [ "${TILR_NO_SERVICE:-}" = 1 ]; then
    return
  fi
  if ! systemctl --user show-environment >/dev/null 2>&1; then
    if [ -n "$has_client" ]; then
      say "no systemd user session here, so no service was set up; tilr starts the server when none runs"
    else
      say "no systemd user session here, so no service was set up; start the server with: $bin"
    fi
    return
  fi

  unit="$UNIT_DIR/tilr.service"
  mkdir -p "$UNIT_DIR"
  if [ ! -f "$unit" ] || [ "$(write_unit)" != "$(cat "$unit")" ]; then
    write_unit >"$unit"
    systemctl --user daemon-reload
  fi

  if systemctl --user is-active --quiet tilr.service; then
    [ -z "$updated" ] || say "the running server reloads itself and keeps its shells"
    [ -z "$migrated" ] || say "tilr.service runs $bin from its next start"
  else
    systemctl --user enable --now tilr.service >/dev/null 2>&1 || true
    sleep 1
    systemctl --user is-active --quiet tilr.service ||
      die "the service did not start, see: journalctl --user -u tilr.service"
    say "started tilr.service, listening on ws://127.0.0.1:7890"
  fi

  # Without lingering the service, and every shell in it, ends at logout.
  if [ "$(loginctl show-user "$(id -un)" -p Linger --value 2>/dev/null)" != yes ]; then
    loginctl enable-linger "$(id -un)" >/dev/null 2>&1 ||
      say "to keep the server running after you log out: sudo loginctl enable-linger $(id -un)"
  fi
}

main() {
  [ "$(uname -s)" = Linux ] || die "only Linux is supported for now, see https://tilr.dev"
  case "$(uname -m)" in
    x86_64 | amd64) arch=x86_64 ;;
    aarch64 | arm64) arch=aarch64 ;;
    *) die "no release for $(uname -m)" ;;
  esac
  bin="$BIN_DIR/tilrd"
  client="$BIN_DIR/tilr"
  legacy="$BIN_DIR/tilr-server"
  updated=
  migrated=
  has_client=

  adopt_legacy
  install_binary
  install_client
  link_legacy
  install_service
  hints
}

main "$@"
