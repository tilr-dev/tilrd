# Proposal: Default listen addresses; tilr service install; releases ship tilr

> Retro archive entry, reconstructed on 2026-10-18 from git history, Basic Memory notes and itests/USE_CASES.md (branch `itests`). Spec deltas are not kept; the resulting behavior is in `openspec/specs/`.

## Why

A bare `tilrd` no longer listened on TCP 7890 without `--listen`, which the installer's unit relied on; the decision was TCP 127.0.0.1:7890 plus the Unix socket by default, with the service optional (UC-023) and installed only on request (Basic Memory `main/projects/tilr/tilrd-tests-integratsionnye-testy-tilrd-na-dagger-use-cases-resheniia`, stage 4).

## What Changes

- `tilrd` listens on 127.0.0.1:7890 and the Unix socket by default; `--no-listen` keeps the socket alone; a taken port stops a service with a clear error, a daemon `tilr` starts goes on without TCP.
- `tilr service install|uninstall`: a systemd user unit (`Type=notify`, `READY=1` from tilrd), a crontab `@reboot` fallback without linger, or a LaunchAgent on macOS; uninstall removes only what install wrote.
- Releases ship `tilr-<platform>` next to `tilrd`.
- itests groups A, C and D. Result: 415 cases, 412 PASS, 3 FAIL: the installer's bad checksums (UC-004), left red.

## Capabilities

### Modified Capabilities
- `daemon-lifecycle`
- `tilr-cli`
- `installation`
- `release-process`

## Out of scope

- Fixing the installer's UC-004 defects (open change `installer-atomic-install`).
- Whether the installer sets up the service by default (open change `installer-service-default`).

## Impact

Implemented by: [`13f9522`](https://github.com/tilr-dev/tilrd/commit/13f9522) tilrd listens on 127.0.0.1:7890 and the unix socket by default … (2026-10-17). Commits on branch `itests` (not yet on `main`; their subjects are to be rewritten before the merge, so the hashes may change). The same commit also holds `2026-10-17-socket-hardening`.
