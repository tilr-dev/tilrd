# Proposal: tilr attach survives errors after a reconnect and restores the terminal

## Why

The code review of branch `itests` (79397e2..13f9522, 2026-10-18) (finding #22): a list or subscribe error after a reconnect exits `tilr` (`attach.rs:121,124`); Ctrl+] d is blocked for up to 5 s while connecting; the terminal is not restored on SIGTERM (`attach.rs:285-313`).

## What Changes

- After a reconnect, an error answering the list or subscribe is retried with the reconnect backoff; a pane that no longer exists ends the attach with a message.
- Ctrl+] d detaches at once, also while connecting.
- SIGTERM, SIGHUP and SIGINT outside raw input restore the terminal (cooked mode, cursor, alternate screen off) before exit.

## Capabilities

### Modified Capabilities
- `tilr-cli`: see `specs/tilr-cli/spec.md`.

## Out of scope

- Changing the prefix key.

## Impact

`crates/tilr/src/attach.rs`. Nothing crosses a reload; no protocol change.
