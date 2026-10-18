# Design

## Context

At 44d615a a starting server installs into the runtime directory shared by all servers of the user: links in `bin/` (`install_shims`, `crates/tilrd/src/clipboard.rs:73`), removed and made again, and the shell wrappers in `shell/` (`install_wrappers`, `crates/tilrd/src/shell.rs:103`), written through `reload::write_file` with the fixed temporary name `.<name>.tmp`. It then binds the host socket at `<server_id>.sock` unless another live server holds it, which it tells by connecting (`host::choose`, `host.rs:130`). The panes of a running server use all three at any time.

## Decisions

### D1. Links are replaced by a rename, and left alone when right
A link already pointing at this executable is not touched (the common case: the same `tilrd` started again, a reload). Otherwise the link is made at `bin/.<name>.<pid>.tmp` and renamed over `bin/<name>`; `rename(2)` replaces the name atomically, so a lookup finds the old link or the new one, never nothing. This is how a file in a shared place is replaced without readers noticing (the durable-file-writes rule, 42a4cc2, for state files; `ln -sfn` followed by `mv -T`, as deploy tools swap a `current` link). The temporary name carries the pid so two servers starting at once do not use each other's.
- Rejected: per-server tool directories (`bin/<pid>/`, as `agent/<pid>/` in 5e0f703). It removes the interference entirely, but `$TILR_SHIMS` then names a directory that changes per server, and the documented `PATH="$TILR_SHIMS:$PATH"` profiles and the wrappers keep the shared path; a server's directory would also have to be swept after a kill. The shared directory with an atomic swap is the smaller change and fixes the observed failure.

### D2. Shared wrappers: written only when changed, through a temporary file of their own
`reload::write_shared_file` compares the file with the content first and does nothing when it matches; otherwise it writes `.<name>.<pid>-<n>.tmp` (pid and a per-process counter), syncs and renames it, as `write_file` does. `write_file` keeps `.<name>.tmp`: the state files are per process and their sweep matches that name (`reload.rs`, `.state.tmp`).

### D3. A probe is not a refused connection
`host::choose` keeps its connect: a server accepting it is alive, whatever pid it has, which is exactly what tmux's client and `ssh -O check` test, and a refused connection marks the socket stale. The pid route (as `agent/<pid>/`) needs the pid in the name, and `<server_id>.sock` has none; a pid file next to it would be a second file to keep in step and would still need the connect for a holder from before it. What was wrong is the log: the probe closes at once, and on macOS `getpeereid` of a closed peer fails with `ENOTCONN`, which the accept loop took for a refusal. `local::peer_gone` recognises that error; the host socket, the Unix socket and the agent sockets close such a connection with a debug line. On Linux the credentials of a closed peer are still readable, so the check passes and the connection ends at its first read, as before. A connection from another uid fails the check with its uid and is still refused with the warning.

## Reload and reconnect

A reload keeps the pid and execs the same path, so the successor finds its links right and leaves them, and finds the wrappers unchanged; a reload into another executable path swaps the links by rename. The host socket choice after a reload is unchanged (`<server_id>-<pid>.sock` if its own is there). Clients are not involved; nothing new crosses the exec.

## Backpressure

Not applicable: no client data.
