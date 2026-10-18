# Proposal: Read-only viewer connections

## Why

A read-only viewer was proposed: a role (a property of the token or connection) without input, Fs or Exec, that does not affect the pane's size; its purpose is to be found later. A per-device `device_id` was deferred in the same note (Basic Memory `main/projects/tilr/tilrd-idei-read-only-zritel-client-id`).

## What Changes

- To be designed: a read-only token kind; input, `Fs*`, `Exec*`, `Resize` and `ClosePane` from such a connection are refused with an `Error`.

## Capabilities

### Modified Capabilities
- `access-control`
- `panes`

Spec deltas are not written yet (`skip_specs: true`).

## Out of scope

- `device_id`.

## Impact

New token kind in the state directory; additive feature flag; survives a reload (tokens are files).
