#!/bin/sh
# tilr installer and updater: curl -fsSL https://tilr.sh | sh
# Served from R2: https://cdn.tilr.dev/install.sh
#
# Puts tilrd (the tilr server) and tilr (its client, the command you type) into
# ~/.local/bin. A release is one archive with both, which is downloaded into a
# temporary directory, checked against its checksum and unpacked there; both
# programs must run before either is put in place. Failures before installation
# leave it as it was (or none at all) and exit non-zero. A reload request failure
# also exits non-zero but keeps both installed programs and reports the failure.
#
# Run it again to update: both are replaced, then an explicit reload request
# tells a running server to use the new executable and keep every shell.
# The same release again changes nothing.
#
# Nothing starts tilrd at boot: `tilr` starts it when none runs, and
# `tilr service install` sets that up when you ask for it.
#
#   TILR_BIN_DIR     where the executables go (default: ~/.local/bin)
#   TILR_CDN         where releases are fetched from
set -eu

CDN="${TILR_CDN:-https://cdn.tilr.dev}"
BIN_DIR="${TILR_BIN_DIR:-$HOME/.local/bin}"

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

# The version a program says it is (`tilrd 0.1.0-26449dd (linux-x86_64)`),
# empty if it is not there or does not run.
version_of() {
  [ -x "$1" ] || return 0
  "$1" --version 2>/dev/null | cut -d' ' -f2
}

# Whatever this run leaves behind is removed on the way out: the temporary
# directory, files staged next to the installed ones and, if nothing was
# installed, the directories it made for them.
cleanup() {
  [ -z "$tmp" ] || rm -rf "$tmp"
  rm -f "$BIN_DIR/.tilr.new.$$" "$BIN_DIR/.tilrd.new.$$" "$BIN_DIR/.tilr.old.$$"
  [ -n "$installed" ] || [ -z "$made" ] || rm -rf "$made"
}

# The release, checked and unpacked into $tmp/release. Changes nothing yet.
download() {
  archive="tilr-$version-linux-$arch.tar.gz"
  url="$CDN/releases/$version/$archive"
  sum=$(fetch "$url.sha256" 2>/dev/null) || die "no release $version for linux $arch at $CDN, nothing installed"
  want=${sum%% *}

  say "downloading tilr $version (linux $arch)"
  fetch "$url" >"$tmp/$archive" || die "download failed, nothing installed"
  [ "$(sha256 "$tmp/$archive")" = "$want" ] || die "checksum mismatch, nothing installed"
  mkdir "$tmp/release"
  tar -xzf "$tmp/$archive" -C "$tmp/release" tilrd tilr >/dev/null 2>&1 ||
    die "the archive of $version is damaged or lacks tilrd or tilr, nothing installed"
  for name in tilrd tilr; do
    file="$tmp/release/$name"
    [ -f "$file" ] && [ ! -L "$file" ] || die "$name in the archive of $version is not a file, nothing installed"
    chmod 755 "$file"
    "$file" --version >/dev/null 2>&1 ||
      die "$name $version does not run on this system (it needs glibc 2.36 or newer), nothing installed"
  done
}

# Both programs into $BIN_DIR, each renamed into place: tilr first, tilrd
# last, then explicitly request reload. If tilrd
# cannot be put in place, the tilr that was there is put back.
put_in_place() {
  # The highest directory that is not there yet: removed again on failure.
  dir=$BIN_DIR
  while [ ! -e "$dir" ]; do
    made=$dir
    dir=$(dirname "$dir")
  done
  mkdir -p "$BIN_DIR" || die "cannot make $BIN_DIR, nothing installed"
  for name in tilrd tilr; do
    { cp "$tmp/release/$name" "$BIN_DIR/.$name.new.$$" && chmod 755 "$BIN_DIR/.$name.new.$$"; } ||
      die "cannot write to $BIN_DIR, nothing installed"
  done

  had_client=
  if [ -f "$client" ]; then
    ln "$client" "$BIN_DIR/.tilr.old.$$" 2>/dev/null || cp -p "$client" "$BIN_DIR/.tilr.old.$$" ||
      die "cannot write to $BIN_DIR, nothing installed"
    had_client=1
  fi
  mv -f "$BIN_DIR/.tilr.new.$$" "$client" || die "cannot replace $client, nothing installed"
  if ! mv -f "$BIN_DIR/.tilrd.new.$$" "$bin"; then
    if [ -n "$had_client" ]; then
      mv -f "$BIN_DIR/.tilr.old.$$" "$client"
    else
      rm -f "$client"
    fi
    die "cannot replace $bin, nothing installed"
  fi
  installed=1
  say "installed tilrd and tilr $version in $BIN_DIR"
}

# What to type next. A PATH without $BIN_DIR gets the line that adds it.
hints() {
  case ":$PATH:" in
    *":$BIN_DIR:"*) ;;
    *)
      case "$BIN_DIR" in
        "$HOME"/*) shown="\$HOME/${BIN_DIR#"$HOME"/}" ;;
        *) shown=$BIN_DIR ;;
      esac
      say "$BIN_DIR is not on your PATH; add this line to ~/.profile and log in again:"
      echo "  export PATH=\"$shown:\$PATH\"" >&2
      ;;
  esac
  say "open a shell: tilr"
  say "start tilrd at boot: tilr service install"
  say "the access token for the app: tilr token"
  say "link this machine to Tilr Cloud: tilr cloud login"
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
  tmp=
  made=
  installed=
  trap cleanup EXIT
  trap 'exit 1' INT TERM HUP

  version=$(fetch "$CDN/releases/linux-$arch/latest" 2>/dev/null) || die "no release for linux $arch at $CDN"
  # It becomes part of a URL.
  case "$version" in
    "" | *[!A-Za-z0-9.+_-]*) die "$CDN names a release that is not a version, nothing installed" ;;
  esac

  if [ "$(version_of "$bin")" = "$version" ] && [ "$(version_of "$client")" = "$version" ]; then
    say "tilr $version is already installed in $BIN_DIR"
    installed=1
  else
    tmp=$(mktemp -d) || die "cannot make a temporary directory, nothing installed"
    download
    put_in_place
    "$bin" reload || die "installed tilr $version, but could not request reload"
    if command -v pgrep >/dev/null 2>&1 && pgrep -u "$(id -u)" -x tilrd >/dev/null 2>&1; then
      say "the running tilrd reloads itself and keeps its shells"
    fi
  fi
  hints
}

main "$@"
