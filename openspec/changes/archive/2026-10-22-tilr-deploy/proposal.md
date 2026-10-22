# Proposal: tilr deploy user@host

## Why

Idea recorded with the tilrd naming decision (2026-10-16): a user with SSH access installs and starts Tilr with one command, no root; "`tilr deploy user@host` from the local machine — goes over SSH, drops the binary, starts tilrd" (Basic Memory `main/projects/tilr/tilrd-naming-install-and-run-model`). At 1163abd `tilr` has no such command (`crates/tilr/src/main.rs:52-120`, spec `tilr-cli` "Command set"); a remote machine is set up by hand with `curl -fsSL https://tilr.sh | sh`, which needs the remote to reach the CDN.

## What Changes

- New command `tilr deploy [--service] [--archive FILE] [--cdn URL] <destination>` (design D1-D5): finds the remote platform over `ssh`, gets the matching release archive on the local machine (from the CDN, checked against its `.sha256`, or `--archive`), sends it over the same SSH connection, installs `tilrd` and `tilr` into the remote `~/.local/bin` with the installer's rules (checked in a temporary directory, both or neither, the server last), optionally runs `tilr service install` there, starts tilrd (`tilr token`) and prints how to attach and how to connect the app (see Decision).
- A hidden `tilrd fetch-release --platform P --out DIR [--cdn URL]` that downloads and checks an archive with `tilrd update`'s code, so `tilr` gains no HTTP or archive dependencies.
- Not breaking; no protocol change.

## Capabilities

### Modified Capabilities
- `tilr-cli`: see `specs/tilr-cli/spec.md`.

## Non-goals

- Root or system-wide installs; a remote without `sh`, `tar`, `mkdir`, `mv`, `uname`.
- macOS remotes from the CDN (the CDN has Linux archives only): they need `--archive`.
- An SSH transport for `tilr` or the Tilr app (rejected for now), and Cloud registration (`--cloud`, deferred to a separate design).
- Changing the remote's `PATH` or shell files (the installer's rule: advised, never edited).

## Impact

`crates/tilr/src/main.rs` and a new `crates/tilr/src/deploy.rs`, `crates/tilrd/src/{main,update}.rs` (hidden `fetch-release`), README (`tilr` commands), USE_CASES.md, itests (a new case with an SSH host), the `tilr-cli` spec. Nothing crosses a reload; a remote tilrd already running reloads into the deployed one as after any update (`update` spec "Live programs survive an update").

## Decision (2026-10-18)

`tilr deploy` installs tilrd on the remote over ssh, then prints ready commands to connect: `ssh -t <dest> '~/.local/bin/tilr'` for the terminal, and `ssh -N -L 7890:127.0.0.1:7890 <dest>` plus the remote token for the app (option (a)). No new transport.

Cloud registration (`--cloud`) needs a separate design after Cloud is in production.

Industry note: Tailscale and Coder install, print a link and attach the machine to the account; VS Code Remote-SSH installs and connects over ssh with no account. `tilr deploy` uses SSH without an account.

An SSH transport in `tilr` (`tilr --ssh <dest>`, option (b)) is rejected for now; it overlaps `multi-server` / `quic-transport`.
