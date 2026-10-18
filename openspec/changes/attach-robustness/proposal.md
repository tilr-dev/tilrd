# Proposal: tilr attach survives errors after a reconnect, detaches at once, and restores the terminal on a signal

## Why

The code review of branch `itests` (79397e2..13f9522, 2026-10-18) (finding #22). Re-checked at 1163abd in `crates/tilr/src/attach.rs`:

- After a reconnect, `fresh.list_panes().await?` and `self.subscribe(&mut fresh).await?` (`attach.rs:120-124`) return the error out of the attach: a connection lost or a request timed out right after reconnecting (a second reload, the server busy) ends `tilr` with `tilr: <error>` and status 1 instead of another attempt.
- The reconnect attempt `Conn::connect(...).await` runs inside a `select!` branch (`attach.rs:117-141`), so keys are not read while it runs; `CONNECT_TIMEOUT` is 5 s (`conn.rs:21`), so Ctrl+] d can wait that long.
- The terminal is restored by a drop guard only (`attach.rs:283-313`); nothing handles SIGTERM, SIGHUP or SIGINT (raw mode turns Ctrl+C into a key, but `kill -INT` still kills), so `tilr` killed by one of them leaves the terminal in raw mode on the alternate screen.

## What Changes

- An error while listing panes or subscribing after a reconnect counts as a failed attempt (status line, backoff), not the end of the attach; a pane missing from the list still ends it with `[pane … is gone]`, and a socket served by another user still ends it with an error.
- Keys are read while a reconnect attempt runs; Ctrl+] d drops the attempt and detaches at once.
- On SIGTERM, SIGHUP or SIGINT an attached `tilr` restores the terminal (as on detach) and then ends by that signal (default action re-raised), printing nothing; the pane goes on.

## Capabilities

### Modified Capabilities
- `tilr-cli`: see `specs/tilr-cli/spec.md`.

## Non-goals

- Changing the prefix key, the reconnect timings or `CONNECT_TIMEOUT`.
- Restoring the terminal after SIGKILL (impossible).

## Impact

`crates/tilr/src/attach.rs` (reconnect as a pinned future polled beside input; signal streams in the event loop), `crates/tilr/src/main.rs` if the re-raise lives there. Nothing crosses a reload; no protocol change; `tilr` is client-only, so an update needs nothing on disk.
