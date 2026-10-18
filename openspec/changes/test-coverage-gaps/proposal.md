# Proposal: Tests for the requirements that have none; known flakes

## Why

The specs written on 2026-10-18 list requirements without a test in their Notes: `cloud-tunnels` 8 (login, link kept, logout, revoke, control connection retry, failing cloudflared restarted, no TCP port no Cloud, cloudflared after SIGKILL), `release-process` 13 (none of the release scripts and workflows is tested), `installation` 1 (manual install; plus the old-`tilr` restore of the installer), `update` 2 (`tilr update`), `tilr-cli` 2 (`kill`, one client id per run), `socks-proxy` 2, `panes` 2, `exec` 1, `metrics` 1, `reload` 1 (stale state files swept), `ssh-agent-forwarding` 1, `daemon-lifecycle` 1. Known flakes (Basic Memory `main/projects/tilr/tilrd-tests-integratsionnye-testy-tilrd-na-dagger-use-cases-resheniia`): `uc_043` p90 echo over 5 s in 2 of about 7 runs; under load `files::nothing_outside_the_shared_directories_is_served` and `reconnect::a_client_that_still_sends_but_no_longer_hears_the_server_is_dropped`; test servers from `target/debug` left running on the Mac after local cargo tests.

Updated after 47a8557: the installer's service and legacy requirements are gone and UC-004 is green, so `installation` drops from 4 to 1; the archive layout adds a release-process requirement. `openspec/coverage/coverage.md` (`./itests/dt coverage`) lists the uncovered requirements and is the current count.

## What Changes

- A test per untested requirement, cargo where one machine suffices, itests otherwise; the release scripts get a dry-run test against a local directory CDN.
- Fix or quarantine each known flake with a recorded cause.

## Capabilities

### Modified Capabilities
- `cloud-tunnels`
- `release-process`
- `installation`
- `update`
- `tilr-cli`
- `socks-proxy`
- `panes`
- `exec`
- `metrics`
- `reload`
- `ssh-agent-forwarding`
- `daemon-lifecycle`

No spec delta: tests, docs or process only; `skip_specs: true`.

## Out of scope

- A real Cloudflare tunnel in tests (needs an account; stays manual).
- Load thresholds (change `itests-stage-5`).

## Impact

Tests only (`crates/*/tests/`, `itests/scenarios/`), USE_CASES.md, the specs' Notes. No behaviour change.
