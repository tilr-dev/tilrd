# Proposal: tilr deploy user@host

## Why

Idea recorded with the tilrd naming decision: `tilr deploy user@host` copies the binaries over SSH and starts tilrd there (Basic Memory `main/projects/tilr/tilrd-naming-install-and-run-model`).

## What Changes

- To be designed: copy `tilr` and `tilrd` for the remote platform (from the CDN or local), install into `~/.local/bin`, optionally `tilr service install`, print the token or connect.

## Capabilities

### Modified Capabilities
- `tilr-cli`

Spec deltas are not written yet (`skip_specs: true`).

## Out of scope

- Root installs.

## Impact

`crates/tilr`. No protocol change.
