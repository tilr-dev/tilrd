# Proposal: The default Cloud is wss.tilr.dev, the API host

## Why

- `cloud::DEFAULT_URL` is `https://cloud.tilr.dev` (`crates/tilrd/src/cloud.rs:37` at 1163abd), the default of `tilrd cloud login --url` / `TILR_CLOUD` (`crates/tilrd/src/main.rs:203`); README.md:379-380 and the `cloud-tunnels` spec name it too.
- The Cloud note says the default should be `https://wss.tilr.dev`, the host of the deployed Worker `tilr-cloud` (Basic Memory `main/projects/tilr/tilr-cloud-v1-control-plane-tunneli-token-dostupa-realizatsiia`).
- The Cloud repository's `wrangler.jsonc` (tilr-dev/cloud, `main`, read 2026-10-18) routes both hosts to the Worker and says: "wss.tilr.dev is the API: what machines and clients are pointed at. cloud.tilr.dev is the page; it is served from here until it has a Worker of its own, and then this route goes." Both hosts answered `/v1/enroll` (400 to an empty body) and `/v1/machine/connect` (401 without a token) the same way on 2026-10-18.
- So the default works today only through a route Cloud plans to remove.

## What Changes

- `DEFAULT_URL` becomes `https://wss.tilr.dev`; `tilrd cloud login --help`, `tilr cloud login --help` (inherits it), README "tilr Cloud" and the spec name it.
- A machine already linked keeps the URL stored in its link (`cloud.json`, field `url`, `crates/tilrd/src/cloud.rs:64,172`); nothing rewrites it.

## Capabilities

### Modified Capabilities
- `cloud-tunnels`: see `specs/cloud-tunnels/spec.md`.

## Non-goals

- Cloud production gaps (Zitadel app, tunnel domain, API token, rate limits): Cloud repository.
- Moving links made against `https://cloud.tilr.dev` (no release has shipped Cloud; such links come only from development and are re-made with `tilr cloud logout` + `login`).

## Impact

`crates/tilrd/src/cloud.rs` (constant, unit test), README.md:379-380, the `cloud-tunnels` spec. Not breaking: the control protocol and `cloud.json` are unchanged. A reload or update keeps every linked machine's stored URL; nothing new crosses a reload. No PROTOCOL_VERSION or feature change. `test-coverage-gaps` also modifies "Login links the machine through a device code" (its Verified by line): whichever lands second re-copies the requirement from `openspec/specs/` before archiving.
