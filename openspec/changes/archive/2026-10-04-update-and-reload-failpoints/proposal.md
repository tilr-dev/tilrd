# Proposal: Failpoints: updates and reloads that go wrong halfway

> Retro archive entry, reconstructed on 2026-10-18 from git history, Basic Memory notes and itests/USE_CASES.md (branch `itests`). Spec deltas are not kept; the resulting behavior is in `openspec/specs/`.

## Why

The update and reload paths had to be tested where they fail; the tests found that a new version crashing after the exec loses the server and every shell, the only protection being the pre-exec `state-formats` check (Basic Memory `main/projects/tilr/tilr-ustanovka-i-obnovlenie-install.sh-relizy-na-r2-tilr-update`).

## What Changes

- Test-only feature `failpoints` (`fail.rs`, `TILR_FAILPOINTS="name=action;..."`, actions error/panic/exit/kill/sleep:MS) at `reload-before-exec`, `supervisor-resume`, `pane-reload-before-exec`, `pane-resume`, `pane-resume:<id>`, `update-before-rename`; not in release builds.
- Use cases: a download cut off, a CDN answering something else, an unwritable directory, an update killed before the swap, two updates at once; a reload failing before the handover, a pane that cannot take over or crashes taking over, a version crashing after taking over, a server killed mid-reload, an executable replaced again during a reload.
- `update` runs one at a time, removes downloads left by a killed update and skips the download when the release is already on disk.
- `scripts/update-test.sh <host> <old-version>` checks an update on a real machine from the CDN (6 scenarios, 31 checks, green on `ubu-test`).

## Capabilities

### Modified Capabilities
- `update`
- `reload`

## Out of scope

- Surviving a crash after the exec (open change `reload-rollback`).

## Impact

Implemented by: [`af46f7d`](https://github.com/tilr-dev/tilrd/commit/af46f7d) Tests for updates and reloads that go wrong halfway (2026-10-04).
