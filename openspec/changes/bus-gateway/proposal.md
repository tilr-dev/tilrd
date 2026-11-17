# Proposal: HTTP gateway to the bus

Зависит от: component-bus, bus-acl-biscuit

## Why

Decision of 2026-10-26 (owner): outside access to the bus goes through a gateway component.
The prototype's gateway (`runtime-tmp/runtimes/gateway/main.ts`, Deno): `POST /call/<method>` and
`/call/<instance>/<method>` on `GATEWAY_ADDR` (default `127.0.0.1:8787`), the body passed as the
payload, the reply always labelled `application/json`, errors as `{error:{code,message}}` with
`denied` 403, `not_found` 404, `unavailable` 503, `timeout` 504, else 500; no HTTP
authentication, the HTTP method not checked, and every caller acting as the instance `gw`, whose
ACL allowed `bus.*`, `debug.*` and `term.*` — any local HTTP client could reload the bus or act
as any instance (`docs/how-it-works.md` §10). `bus-acl-biscuit` gives the means to fix that:
per-call credentials.

## What Changes

- The gateway is Rust, a subcommand of the tilrd executable (`tilrd gateway`), run by the bus as
  an ordinary instance of a built-in module (`[modules.gateway] builtin = "gateway"`), so it
  ships in the existing archive and is updated with tilrd.
- Routes: `POST /call/<method>`, `POST /call/<instance>/<method>`; other paths 404, other HTTP
  methods 405.
- Authentication: `Authorization: Bearer <token>` (a Biscuit minted with `tilr bus token` and
  possibly attenuated); forwarded as `Call.credential`; no token = 401 without touching the bus.
  The gateway instance's own rights are empty.
- Bind: loopback addresses only (`127.0.0.1`, `::1`); other addresses refused at start. `Host`
  must name a loopback address or `localhost` (DNS rebinding); requests with an `Origin` header
  are refused unless listed in `allow_origins` of the gateway's arguments.
- Content types pass through both ways; trace id returned in `X-Tilr-Trace`; `X-Tilr-Timeout-Ms`
  sets the call deadline (capped by the bus's `call_timeout_ms`).
- Limits: body up to 16 MiB minus framing (413 above), headers 16 KiB, 256 requests at once
  (503 above), idle connections closed after 60 s.
- Status mapping extended: `busy` 429, `too_large` 413, `bad_request` 400, `not_supported` 501.

## Capabilities

### New Capabilities
- `bus-gateway`: the gateway's HTTP contract, authentication, bind rules, limits.

## Non-goals

- TLS and non-loopback binds (remote callers reach the machine through tilrd's existing paths:
  ssh, Cloud tunnel, a reverse proxy the user runs); WebSocket or streaming over HTTP.
- Sessions, cookies, OAuth; minting tokens over HTTP.
- An "act as instance" method (`debug.call` of the prototype).
- A Deno gateway.

## Impact

`crates/tilrd/src/gateway.rs` (new; HTTP/1.1 server on `hyper` 1.x — a new dependency checked by
`cargo deny`), `crates/tilrd/src/main.rs` (`tilrd gateway`), `crates/tilrd/src/bus/config.rs`
(`builtin` modules; reload of built-in instances after a tilrd reload into a new executable),
README ("Components": the gateway), USE_CASES.md. No tilr.proto change; no state format change
(the gateway holds no state). Update from the previous release: nothing runs unless `bus.toml`
names the gateway.
