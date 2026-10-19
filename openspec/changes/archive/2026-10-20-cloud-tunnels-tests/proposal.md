# Proposal: Tests for the untested cloud-tunnels requirements

## Why

The local coverage report (`./itests/dt coverage` → `openspec/coverage/coverage.md`, git-ignored; generated 2026-10-18) lists 8 `cloud-tunnels` requirements without a test. Checked at 1163abd: `crates/tilrd/tests/cloud.rs` has five tests, none of login, logout, revoke, retry, a failing `cloudflared`, `--no-listen` or a SIGKILLed server. USE_CASES.md says Cloud login/logout were checked only by hand against a local Cloud.

## Scope

- `cloud-tunnels` "cloudflared does not outlive the server" was left to `cloudflared-outlives-exec` (archived 2026-10-18), which fixed it and added its tests (UC-088); this change covers the other 7 `cloud-tunnels` requirements.
- `cloud-tunnels` "Login links the machine through a device code" is also modified by `cloud-default-url`: whichever lands second re-copies the requirement from `openspec/specs/` before archiving.

## What Changes

- A cargo test per requirement against the test-played Cloud of `crates/tilrd/tests/cloud.rs`, extended with the enrollment endpoints (`/v1/enroll`, `/v1/enroll/poll`, `DELETE /v1/machine`) and a stand-in `cloudflared` that can be made to exit; each named in its requirement's **Verified by** line (spec delta below).
- A test that finds the code differs from its requirement is kept red and the difference goes to its own change (no behavior change here).

## Capabilities

### Modified Capabilities
- `cloud-tunnels`: **Verified by** lines name the new tests (see `specs/`); requirement text unchanged.

## Non-goals

- A real Cloudflare tunnel in tests (needs an account; stays manual).
- Behavior changes.

## Impact

Tests only: `crates/tilrd/tests/cloud.rs`, USE_CASES.md, the `cloud-tunnels` spec's **Verified by** lines and Notes. No behavior, protocol, reload or update impact. The deltas are MODIFIED copies of the current requirements with a new **Verified by** line: if another change modifies one of them first (e.g. `cloud-default-url`), the copy is refreshed before this change is archived (task 2.2). Tracker: TILR-147 (epic TILR-76). Split from the former shared change `test-coverage-gaps` (2026-10-20: one task, one change).
