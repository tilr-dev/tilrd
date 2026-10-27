# Proposal: SOCKS5 over WebSocket on the listen port

> Retro archive entry, reconstructed on 2026-10-18 from git history, Basic Memory notes and itests/USE_CASES.md (branch `itests`). Spec deltas are not kept; the resulting behavior is in `openspec/specs/`.

## Why

The Mac browser needed to reach loopback services of a remote server (Basic Memory `main/projects/tilr/tilr-proksi-v-brauzere-socks-over-ws-cherez-core-profili-wkweb-view-izvestnye-ogranicheniia`).

## What Changes

- `GET /socks` carries a SOCKS5 session in WebSocket frames with the same origin check and access token (Bearer header, `?token=` fallback); 404 when off, 403 for a bad origin, 401 for a bad token.
- A raw SOCKS5 client on the same port is told by its first byte 0x05 (RFC 1929, password = the token).
- `localhost` and `*.localhost` resolve to the server's loopback without DNS.
- Off unless `--socks` or `socks = true`.

## Capabilities

### Modified Capabilities
- `socks-proxy`

## Out of scope

- `ssh -L`-style port forwarding (open change `port-forwarding`).

## Impact

Implemented by: [`ffb376d`](https://github.com/tilr-dev/tilrd/commit/ffb376d) SOCKS5 proxy: GET /socks on the listen port carries a SOCKS5 session in WebSocket frames … (2026-10-10).
