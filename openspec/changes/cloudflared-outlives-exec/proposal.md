# Proposal: cloudflared never outlives or holds back the server

## Why

The code review of branch `itests` (79397e2..13f9522, 2026-10-18) (finding #25): a stuck `cloudflared` outlives the exec (`cloud.rs:629`). The `cloud-tunnels` spec states that no `cloudflared` survives a SIGKILL of the server, which is not met at HEAD (plain child, no parent-death signal) and itests UC-088 has no scenario. Basic Memory `main/projects/tilr/tilrd-tests-integratsionnye-testy-tilrd-na-dagger-use-cases-resheniia` also notes that `cloudflared` sits in the supervisor's process group, so a terminal's SIGHUP kills it and the server restarts it (tunnel flicker).

## What Changes

- `cloudflared` runs with a parent-death signal (Linux `PR_SET_PDEATHSIG` SIGTERM; macOS: a watchdog pipe it is wrapped with) and in a process group of its own.
- A reload that cannot reap `cloudflared` within the wait SIGKILLs it and goes on with the exec; the successor starts a new one.
- itests UC-088 scenario.

## Capabilities

### Modified Capabilities
- `cloud-tunnels`: see `specs/cloud-tunnels/spec.md`.

## Out of scope

- Cloud-side tunnel handling.

## Impact

`crates/tilrd/src/cloud.rs`. Has to survive a reload (the successor starts its own cloudflared, as now). No protocol change.
