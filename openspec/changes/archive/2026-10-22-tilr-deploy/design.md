# Design

## Decisions

### D1. The system ssh, one connection
`tilr` runs the user's `ssh` (their config, agent, `ProxyJump`, known hosts) with `-o ControlMaster=auto -o ControlPath=<private temp dir>/%C -o ControlPersist=60`, so a password or key prompt comes once for all steps. No SSH library in `tilr`.

### D2. Platform
`ssh <dest> uname -sm`: `Linux x86_64` → `linux-x86_64`, `Linux aarch64` → `linux-aarch64`, `Darwin arm64` → `macos-aarch64`; anything else fails with `<dest> runs <uname>, for which there is no release`.

### D3. Where the archive comes from
`--archive FILE` (a `tilr-<version>-<platform>.tar.gz` with its `.sha256` next to it, checked), else the CDN (`--cdn`, `TILR_CDN`, default `https://cdn.tilr.dev`) through the hidden `tilrd fetch-release`, which reuses `update.rs`'s download, checksum and archive checks. The same internal helper accepts `--archive FILE` to validate and stage a supplied archive without HTTP or executing foreign-platform programs. A platform the CDN has no `latest` for fails with `no release of tilr for <platform> on <cdn>; give one with --archive`.

### D4. Remote install
The archive goes over stdin of `ssh <dest> sh -c '<script>'`; the script (embedded in `tilr`) follows the `installation` spec: `mktemp -d`, unpack, both `--version`, then `mv` `tilr` and then `tilrd` into `~/.local/bin` (made if missing), removing the temp dir on any exit. It prints `installed tilr <version> in ~/.local/bin` or the installer's failure message and exits non-zero leaving the old files.

### D5. Start and hand over
When an executable client is already installed and can list panes from a running server, it obtains the persistent token before either program is replaced. This avoids a new client's protocol handshake with an old server while the executable watcher waits to reload. With no running server, the new installed client obtains the token after replacement and optional service installation.

For a host without a running daemon, token acquisition follows service installation so a refused installation cannot leave a daemon started by the token command. With `--service`, `~/.local/bin/tilr service install` then runs there and its output is shown. Before invoking it, deploy snapshots the service configuration, the crontab where available, and the enabled/lingering state. A failed install restores the previous configuration together with the binaries. Only after token acquisition and optional service installation succeed does `tilr deploy` print the connection commands and token. If `~/.local/bin` is not on the remote's login `PATH`, it says what to add, as the installer does.

## Reload and reconnect

Nothing is handed over by deploy itself. When tilrd already runs on the remote, replacing its executable makes it reload as after `tilrd update`; its panes and clients survive.
