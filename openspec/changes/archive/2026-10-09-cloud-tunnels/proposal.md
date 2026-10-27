# Proposal: tilr Cloud: machine login and cloudflared tunnels

> Retro archive entry, reconstructed on 2026-10-18 from git history, Basic Memory notes and itests/USE_CASES.md (branch `itests`). Spec deltas are not kept; the resulting behavior is in `openspec/specs/`.

## Why

Decisions of 2026-10-04 (Dan): a control plane plus Cloudflare tunnels, a trial on Tilr's domain, Zitadel/OIDC, Workers and Durable Objects. The server only carries the tunnel; Cloud does not touch panes (Basic Memory `main/projects/tilr/tilr-cloud-v1-control-plane-tunneli-token-dostupa-realizatsiia`, `main/projects/tilr/tilr-zhurnal-reshenii`).

## What Changes

- `tilr-server cloud login|logout|status` links the machine by a device code.
- A linked server keeps a control connection and runs `cloudflared` when told to, pointed at its own port only, with a configuration it writes itself (values validated).
- A server without authentication takes no tunnel; `cloudflared` is stopped on a reload and restarted by the successor.
- Tests in `tests/auth.rs` and `tests/cloud.rs`, Cloud and cloudflared played by the test.

## Capabilities

### Modified Capabilities
- `cloud-tunnels`

## Out of scope

- Telemetry from the server to Cloud (Dan, 2026-10-06: not sent until he says so).
- Zitadel app, tunnel domain and API token in production (Cloud repository).

## Impact

Implemented by: [`4cd2f84`](https://github.com/tilr-dev/tilrd/commit/4cd2f84) Access token, tilr Cloud, and a configuration file for the allowed origins (2026-10-09).
