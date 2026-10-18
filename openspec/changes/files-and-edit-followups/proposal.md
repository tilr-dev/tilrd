# Proposal: Follow-ups for files and tilr edit

## Why

Open items from the files work: `tilr edit --wait`; a clear error for paths outside the roots such as `/tmp`; the delete confirmation counts only direct entries; image and PDF preview; upload and download of large files; a SOCKS allowed-hosts setting (Basic Memory `main/projects/tilr/tilr-files-failovyi-protokol-ctx.open-i-tilr-edit-dizain-reshenie`, `main/projects/tilr/tilr-todo-failovyi-i-socks-protokoly-servera-vkliuchaiutsia-i-otkliuchaiutsia-v-konfige`).

## What Changes

- Triage each item into server, client or drop; server items get their own requirements here.

## Capabilities

### Modified Capabilities
- `files`
- `clipboard-host-events`
- `socks-proxy`

Spec deltas are not written yet (`skip_specs: true`).

## Out of scope

- Client UI items (previews) unless a server API is needed.

## Impact

Depends on the items kept.
