# Proposal: The default Cloud URL

## Why

`cloud::DEFAULT_URL` is still `https://cloud.tilr.dev` (verified at 13f9522, `crates/tilrd/src/cloud.rs:37`); the Cloud note says it should be `https://wss.tilr.dev`, the host of the deployed Worker `tilr-cloud` (Basic Memory `main/projects/tilr/tilr-cloud-v1-control-plane-tunneli-token-dostupa-realizatsiia`). Whether that is still current was not recorded.

## What Changes

- Set the default to the host that serves `/v1/enroll` and `/v1/machine/connect` in production, after checking it.

## Capabilities

### Modified Capabilities
- `cloud-tunnels`: see `specs/cloud-tunnels/spec.md`.

## Out of scope

- Cloud production gaps (Zitadel app, tunnel domain, API token, rate limits): Cloud repository.

## Impact

`crates/tilrd/src/cloud.rs`, `crates/tilr` help texts, README, the `cloud-tunnels` spec ("Login links the machine through a device code" names the default). A linked server keeps its stored URL, so a reload or update does not move it.
