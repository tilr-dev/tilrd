# TODO before going open source

File and line numbers are as of the 2026-10-03 review; after the 2026-10-04 changes (clipboard, supervisor reload, pane UUIDs) they moved in `supervisor.rs` and `pane.rs`.

## Security

- [x] Authentication: an access token in `Hello.token`, kept in `$XDG_STATE_HOME/tilr/token` (0600); `tilr-server token` prints it; `--no-auth` turns it off with a warning.
- [ ] One token per server, and it never expires: no revocation, no per-client tokens, no permissions (read-only and the like).
- [ ] Token guessing is not rate-limited (the length is enough: 256 bits), but attempts are neither counted nor logged per address.
- [ ] Limit the pane size: clamping to 65535×65535 lets a single `Resize`/`CreatePane` crash a pane or eat memory (`supervisor.rs:364,388`, `pane.rs:177`).
- [ ] Bound the queues: the event channel, `to_pane`, `pending_input` (`pane.rs:62`) are unbounded. Lower the incoming WS message limit from 64 MB to ~1 MB.
- [x] Ping/keepalive: the server pings clients and drops one that goes silent (`--client-timeout`, 30 s by default).
- [ ] Handshake timeout, connection limit (`accept_loop`).

## Bugs

- [x] Zombie client: packets from a client the supervisor has already dropped are ignored (`on_client_packet`). Before, a `Subscribe` from such a client left it among the pane's viewers forever; caught by the test `many_reconnects_leave_no_viewers_behind`.
- [ ] Overflow in mouse coordinates: `input.x * CELL_WIDTH_PX` (`ghostty.rs:291-292`). Clamp to the screen size.
- [x] New panes are started through `current_exe()`: the executable's path is now remembered at startup (`exe()` in `main.rs`), and after the executable is replaced the server and the panes move to the new version on their own.
- [ ] When the server stops, a shell that ignores SIGHUP is left an orphan: pane processes are killed at once, with no SIGKILL to the shell.
- [ ] `broadcast_pane_list` storm: every title change, `Subscribe`, `Resize` sends the full list to every client. Needs throttling or deltas.
- [ ] A `CreatePane` error without `request_id` (`supervisor.rs:321`): the client cannot match the refusal to its request.
- [ ] The "login shell" is not a login shell: `$SHELL` is started without `-l` (`pty.rs:24`). Fix it, or fix the comment and the proto.
- [ ] `EINTR` on a pty read/write crashes the pane (`pane.rs:130,140`).
- [ ] An exit code from a signal turns into -1 (`supervisor.rs:397`).

## Protocol (before v1)

- [x] `Hello`: the version is checked, `Hello` must be the first message; otherwise `Error` and the connection is closed.
- [x] `Ping` / `Pong` in the protocol (feature `"ping"`), so a browser client notices a server that silently went away.
- [ ] Web client: send `Ping` and reconnect when no `Pong` comes back.
- [x] Extensibility: unknown messages are ignored, features are announced in `Hello.features` / `HelloAck.features`.
- [ ] `KeyInput`: the physical key for non-Latin layouts (Ctrl+С on a Russian layout does not give `^C`).
- [ ] Document or change: scroll and size are shared by all viewers, input is accepted from unsubscribed clients.
- [ ] `seq` resets when the pane process restarts.
- [x] The pane ID is a UUID (a string), not a number: the same in the protocol, in `ps`, in `$TILR_PANE`. Protocol version 2. The server ID is in `HelloAck.server_id`.
- [ ] `tilr-server ps` does not show the shell's pid: the supervisor does not know it, the pane process does.
- [ ] `UNSPECIFIED = 0` in `KeyAction` and `MouseAction`: an unknown value currently becomes `Press`.
- [ ] Remove `PaneStatus` from the public proto.
- [ ] README: a client must escape the title and the cwd.

## Packaging

- [x] `LICENSE-MIT` + `LICENSE-APACHE` (MIT OR Apache-2.0), CI (`.github/workflows/ci.yml`); `SECURITY.md` comes from the tilr-dev org defaults. README is there.
- [x] Remove `.cargo/config.toml` from the repository, describe the workaround in the docs.
- [x] Describe the build requirements: `zig` 0.15.2, `protoc`, network (ghostty clone) (README, "Build"). Still open: for the proto, commit the generated code or use `protox`.
- [ ] README: the risk of depending on `libghostty-vt = "=0.2.2"` (a third-party wrapper, pre-1.0).
- [ ] Manifests: `description`, `repository`, `rust-version` are done; still missing: `version` on the path dependencies.
- [ ] Public API: drop `anyhow` from the `Screen` trait, `DecodeError` with `source()` and `#[non_exhaustive]`, docs for pub items.
- [ ] `cfg(unix)` or a clear compile error on Windows.
- [x] Scenario-based integration tests of the server (`crates/tilr-server/tests/`, listed in `USE_CASES.md`): connecting, reconnecting, scrolling, clipboard, reload.
- [ ] Unit tests for `pty` and for input encoding in `pane`.
- [x] `cargo fmt`.
- [ ] Check whether the name `tilr` is free on crates.io.
- [ ] Documentation site: Astro + Starlight (`npm create astro@latest -- --template starlight`), modelled on herdr.dev/docs.

## Three platforms (Linux, macOS, Windows)

- [x] The pty and the shell live in the pane process, which talks to the supervisor over stdin/stdout (no descriptor passing, no pane restart).
- [ ] Server settings: `--max-cols`, `--max-rows`, `--scrollback-lines`; the supervisor clamps, a resize error does not crash the pane.
- [ ] Check that `tilr-screen` (libghostty-vt) builds on Windows. Fallback engine: `alacritty_terminal`.
- [ ] A `Pty` trait in the pane process: the Unix implementation (the current `pty.rs`) and ConPTY.
- [ ] Windows: a Job Object instead of signals, a default shell (`COMSPEC`/PowerShell), `path_from_pwd` for `file:///C:/...`.
- [ ] CI matrix on three platforms. Linux and macOS run in CI now (`.github/workflows/ci.yml`); Windows is missing. `scripts/linux.sh` runs the Linux suite locally.

## tilr Cloud

Done: `tilr-server cloud login|logout|status`, the control connection,
starting `cloudflared` when Cloud says so (`src/cloud.rs`). Cloud itself lives in a separate repository.

- [ ] Check with a real `cloudflared` and a real tunnel: so far only a stub script.
- [ ] End-to-end encryption client ↔ server: TLS currently ends at Cloudflare, frames are visible there.
- [ ] Handing the access token to the client (by hand today: `tilr-server token` → paste into the client). Needs client pairing without Cloud: a QR code or a code.
- [ ] `--allow-origin` for a web client not opened from localhost is set by hand.
- [ ] `cloudflared` is not installed automatically: it must be on `PATH` or given with `--cloudflared`.
- [ ] A `cloudflared` that outlives a crashed server keeps running.
- [ ] `cloud status` does not ask the running server, it only reads the link file: the tunnel state is only visible in the log and on the Cloud page.

## Clipboard

- [x] The server's shared clipboard: `ClipboardPut` / `ClipboardAck`, a size limit, a time to live, stand-in `xclip` and `wl-paste` on the panes' PATH.
- [ ] Check pasting in a real Claude Code on a remote Linux (only the commands it runs have been checked so far).
- [ ] Programs that read X11 or Wayland directly (not through `xclip` / `wl-paste`): fallback is pasting a file path, or a real X selection.
- [ ] Remote macOS: programs read the clipboard through `osascript`; a PATH stand-in does not work there.
- [ ] The other direction: copying in a pane (`xclip -i`, `wl-copy`) → the client's clipboard.
- [ ] Memory: an upload accumulates in the supervisor's memory up to the limit, per client.

## Reloading without losing shells

- [x] The supervisor replaces itself with the new executable (`exec`), the panes stay; triggered by SIGUSR1 and by the executable being replaced on disk.
- [ ] Client connections are dropped by a reload (the client reconnects). They could be handed over too.
- [ ] An update done by repointing a symlink is not noticed: the server watches the path it was started from.
- [x] State is handed to the successor in a file it finds by name (the supervisor's pid, the pane's UUID), not by a `--resume` argument: the command line does not change after a reload.
- [ ] The supervisor's state file is named by pid. If the successor dies before reading it, the file stays; a server that gets the same pid in the same directory takes it for its own and fails to start the first time. Such files are currently cleaned up after an hour.
- [x] A version for the supervisor's and the pane's state format; before `exec` the server asks the new executable (`state-formats`) and does not reload into one that reads another format.
- [ ] Carrying state across formats (a format change currently needs a restart).
- [ ] If the new supervisor crashes after `exec`, the panes are lost: the check before `exec` covers only starting and the state format.
