# Proposal: Access token in Hello; server.toml with the allowed origins

> Retro archive entry, reconstructed on 2026-10-18 from git history, Basic Memory notes and itests/USE_CASES.md (branch `itests`). Spec deltas are not kept; the resulting behavior is in `openspec/specs/`.

## Why

Serving shells beyond localhost (a tunnel, `pb.rexuni.dev`) needs authentication: the 2026-10-03 log already required a token checked at the handshake. The Tilr app serves each of its apps from `tilr://<name>/`, which the built-in origin list did not cover (commit message; Basic Memory `main/projects/tilr/tilr-cloud-v1-control-plane-tunneli-token-dostupa-realizatsiia`).

## What Changes

- A client must present the server's access token in `Hello.token`; the token is made on the first run, kept in the state directory (0600), printed by `tilr-server token`; `--no-auth` turns it off. Protocol version stays 2.
- `~/.config/tilr/server.toml` (`XDG_CONFIG_HOME`, `--config`, `TILR_CONFIG`) sets `allow_origins`; `--allow-origin` replaces the file's list; a file that does not parse or names an unknown key stops the server.
- `tilr://tilr` allowed by default (the Tilr app's own terminal app).

## Capabilities

### Modified Capabilities
- `access-control`

## Out of scope

- Tokens for the Unix socket (it later needs none; see `2026-10-17-tilr-client-and-unix-socket`).

## Impact

Implemented by: [`4cd2f84`](https://github.com/tilr-dev/tilrd/commit/4cd2f84) Access token, tilr Cloud, and a configuration file for the allowed origins (2026-10-09); [`fef29a6`](https://github.com/tilr-dev/tilrd/commit/fef29a6) Allow tilr://tilr by default: the Tilr app's own terminal app (2026-10-09). The same commit 4cd2f84 also holds `2026-10-09-cloud-tunnels`. Deployed on `pb.rexuni.dev` behind a manual Cloudflare tunnel (Basic Memory `main/projects/tilr/tilr-server-na-pb.rexuni.dev-systemd-cloudflare-tunnel-tilr-pb.rexuni.dev`).
