# Proposal: a server starting leaves the shared runtime files of the user's other servers usable

## Why

Itsaplan TILR-122 (phase 1, bug, high) and TILR-123. Two `tilrd` of one user share the runtime directory: the tools for the panes (`bin/xclip`, `bin/wl-paste`, `bin/tilr`), the shell startup wrappers (`shell/`) and the host socket name `<server_id>.sock`. Checked at 44d615a:

- `install_shims` (`crates/tilrd/src/clipboard.rs:73-96`) removed every link in `bin/` and made it again, at every start. A second server does that after it logs `listening on` (`supervisor.rs:434`, then `set_up_shims`, `supervisor.rs:543`), while the first server's panes run `tilr`, `xclip` and `wl-paste` from there: one landing in between is not found (exit 127). This is how `files::two_servers_of_one_user_keep_their_host_sockets` failed on macOS at `files.rs:1146` (`tilr edit` exited 127; noted in `test-coverage-gaps` 5.10). Reproduced: `clipboard::tests::the_shims_are_never_missing_while_another_server_installs_them` found the link missing 35045 times in 2000 installs, and `files::a_second_server_starting_leaves_the_first_ones_panes_their_tools` (a pane of the first server checking `$TILR_SHIMS/tilr` while three second servers start) failed 3 of 3 at 44d615a.
- `shell::install_wrappers` (`crates/tilrd/src/shell.rs:103`) rewrote the wrappers at every start through `reload::write_file`, whose temporary name `.<name>.tmp` is the same for every server: two servers starting at once write the same temporary file and the later rename fails (`ENOENT`), and then the server removes the whole `shell/` directory (`supervisor.rs:802-805`), which the other server's new panes start through. Reproduced by `shell::tests::servers_starting_at_once_each_install_the_wrappers` (fails at once at 44d615a).
- `host::choose` (`crates/tilrd/src/host.rs:130`, from ddcc453) tells whether another live server holds `<server_id>.sock` by connecting to it. On macOS the live server then logs `host socket: refused a connection: peer credentials: Socket is not connected (os error 57)` for every server that starts: the probe has closed before the credential check, and macOS cannot read the credentials of a closed peer. Seen 3 times in the first server's log of the new test.

## What Changes

- The pane tools: a link that already points at this executable is left alone; one to change is made aside (`bin/.<name>.<pid>.tmp`) and renamed over the old one, so the name is always there. Links of tools this server does not serve are still removed.
- The shell wrappers: written only when their content differs, through a temporary file of the write's own (`.<name>.<pid>-<n>.tmp`), so two servers starting at once both succeed.
- The probe of `<server_id>.sock` stays (it is how tmux's client and `ssh -O check` tell a live server from a stale socket); the server takes a connection whose peer has already closed before the credential check as a probe and closes it with a debug line instead of a warning. The same applies to the Unix socket (whose staleness `tilrd` checks the same way, `local.rs:278`) and the agent sockets. A connection from another uid is still refused and logged as before.

## Capabilities

### Modified Capabilities
- `clipboard-host-events`: new requirements "Another server starting leaves the pane tools in place" and "A probe of the host socket is not a refused connection".
- `panes`: new requirement "The shell wrappers survive another server starting".
- `access-control`: new requirement "A connection gone before its uid check is not refused".

## Non-goals

- Tools per server (e.g. `bin/<pid>/`, as the agent sockets went to `agent/<pid>/` in 5e0f703): `$TILR_SHIMS` = `<runtime dir>/bin` is documented for profiles (`PATH="$TILR_SHIMS:$PATH"`), so the shared directory stays; the last server to start still decides which executable `tilr` in it is, and a server started with the clipboard off still removes `xclip` / `wl-paste` the other's panes use.
- Deciding the host socket's liveness without connecting (TILR-123's suggested fix): the shared name `<server_id>.sock` carries no pid, so knowing its holder would need a new pid file kept in step with the socket, and a probe would still be needed for a holder from before such a file. Connecting is the proven check; only its log line was wrong.
- The startup order (`listening on` before the tools are installed) and the tests' wait for it (`test-coverage-gaps` 5.10, TILR-113).
- Temporary files left by a server killed between the write and the rename (dot-files in `bin/` and `shell/`, never on a lookup path of a tool name).

## Impact

`crates/tilrd/src/clipboard.rs` (`install_shims`), `crates/tilrd/src/reload.rs` (`write_shared_file`), `crates/tilrd/src/shell.rs`, `crates/tilrd/src/local.rs` (`peer_gone`), `crates/tilrd/src/host.rs`, `crates/tilrd/src/supervisor.rs`, `crates/tilrd/src/agent.rs`, `crates/tilrd/tests/files.rs`, README, USE_CASES.md. No protocol change, nothing new crosses a reload, `state-formats` unchanged. A reload keeps the pid and the executable path, so its links are found right and left alone. An update from the previous release: the updated server's first start replaces links and wrappers the old one made by a rename, without a gap; an old server of the same user still running keeps its remove-then-link at its own next start. Not breaking.
