# Proposal: A reload finds the state of a release that used another runtime directory

## Why

The code review of branch `itests` (79397e2..13f9522, 2026-10-18) (finding #3): without `XDG_RUNTIME_DIR`, Linux moved the runtime directory from `$TMPDIR` (`std::env::temp_dir()`) to `/tmp` (`tilr-proto/src/local.rs:41-48`). A reload from an older release into this one finds no state there; the socket bind then fails with AddrInUse, a required socket is fatal (`supervisor.rs:621`) and the shells die.

## What Changes

- When `XDG_RUNTIME_DIR` and `TILR_RUNTIME_DIR` are unset and a resuming server finds no state in `/tmp/tilr-<uid>`, it also looks in `$TMPDIR/tilr-<uid>` (the old location) and resumes from there.
- A resuming server that finds the socket path taken by its own inherited listener does not treat it as AddrInUse.

## Capabilities

### Modified Capabilities
- `daemon-lifecycle`: no requirement change.
- `reload`: see `specs/reload/spec.md`.

## Out of scope

- Moving the runtime directory again or changing its default.

## Impact

`crates/tilr-proto/src/local.rs`, `crates/tilrd/src/supervisor.rs`. Must survive a reload from the previous release (the point of the change); no protocol or state-format change.
