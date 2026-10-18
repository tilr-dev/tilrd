# Design

## Decisions

### D1. The system ssh, one connection
`tilr` runs the user's `ssh` (their config, agent, `ProxyJump`, known hosts) with `-o ControlMaster=auto -o ControlPath=<private temp dir>/%C -o ControlPersist=60`, so a password or key prompt comes once for all steps. No SSH library in `tilr`.

### D2. Platform
`ssh <dest> uname -sm`: `Linux x86_64` → `linux-x86_64`, `Linux aarch64` → `linux-aarch64`, `Darwin arm64` → `macos-aarch64`; anything else fails with `<dest> runs <uname>, for which there is no release`.

### D3. Where the archive comes from
`--archive FILE` (a `tilr-<version>-<platform>.tar.gz` with its `.sha256` next to it, checked), else the CDN (`--cdn`, `TILR_CDN`, default `https://cdn.tilr.dev`) through the hidden `tilrd fetch-release`, which reuses `update.rs`'s download, checksum and archive checks. A platform the CDN has no `latest` for fails with `no release of tilr for <platform> on <cdn>; give one with --archive`.

### D4. Remote install
The archive goes over stdin of `ssh <dest> sh -c '<script>'`; the script (embedded in `tilr`) follows the `installation` spec: `mktemp -d`, unpack, both `--version`, then `mv` `tilr` and then `tilrd` into `~/.local/bin` (made if missing), removing the temp dir on any exit. It prints `installed tilr <version> in ~/.local/bin` or the installer's failure message and exits non-zero leaving the old files.

### D5. Start and hand over
With `--service`, `~/.local/bin/tilr service install` runs there and its output is shown. Then `~/.local/bin/tilr token` runs there (starting tilrd as the `tilr-cli` spec says) and `tilr deploy` prints the two lines of the Decision (proposal.md). If `~/.local/bin` is not on the remote's login `PATH`, it says what to add, as the installer does.

## Reload and reconnect

Nothing is handed over by deploy itself. When tilrd already runs on the remote, replacing its executable makes it reload as after `tilrd update`; its panes and clients survive.

## Open

### `--cloud` (not designed)
Registering the machine in Tilr Cloud: `tilr cloud login` runs on the remote and its device code is shown on the local terminal. Open questions: how the code and the login URL are relayed, what happens when the remote is already linked, and the flag's interplay with `--service`. To be designed after Cloud is in production; nothing of it is in groups 1-4.
