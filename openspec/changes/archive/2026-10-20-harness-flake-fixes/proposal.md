# Proposal: Known flakes of the test harness, phase 1

## Why

Flakes seen in phase 1 (2026-10-18, implementing attach-robustness, sighup-ignoring-shell-reaped, cloudflared-outlives-exec and the changes after them on dev; each under full-suite load, each green when run alone), recorded as items 5.3-5.16 of the former shared change `test-coverage-gaps`. Each needed its cause found and the test fixed, without behavior changes.

## What Changes

The done flake items of the former `test-coverage-gaps`, with their numbers kept (tasks.md) and the decision log per flake (design.md D1-D7):

- 5.4, 5.14 (TILR-110, TILR-125): ETXTBSY starting a copied tilrd — eedd344.
- 5.5 (TILR-115): wait for a log line instead of asserting on a snapshot — 6e9be7f.
- 5.13, 5.15 (TILR-120, TILR-126): `Daemon::start` waits for READY=1 — 6a925bd.
- 5.10 (TILR-113): the second server starts once the first has its host socket — 5b4a5a7.
- 5.3, 5.16 (TILR-127): the reload-check test waits past the server's own bounds — e69f3a1.
- 5.12 (TILR-119): the cloud leftover test waits for the shell's arguments — 926bc28.
- 5.9 (TILR-112): the lagging client compared with the pane's whole last screen — 9e1c26c.
- `local::a_server_that_stops_removes_its_socket_and_lock`: fixed by `startup-signal-window` (TILR-106, f8fb63b).

## Scope

The rest of the former `test-coverage-gaps` went, one task one change (2026-10-20), to:
`cloud-tunnels-tests` (TILR-147), `server-requirement-tests` (TILR-148), `tilr-client-tests` (TILR-149, archived), `manual-install-itest` (TILR-150), `files-root-flake` (TILR-151), `uc-043-flake` (TILR-152), `test-server-leftovers` (TILR-153), `reconnect-output-flake` (TILR-111, 5.6), `hangup-shell-flake` (TILR-117, 5.11, reopened; D8 moved there). 5.7 (itests `uc_063`) went to TILR-131 (input acknowledged and resent, its own change `acked-input`); 5.8 (itests `uc_033`) was dropped with TILR-132 (canceled; decision 2026-10-19 "Флак EADDRINUSE в itests uc_033 не расследуем, при повторе — новая задача со сбором диагностики"). The `release-process` requirements had gone to `release-process-tests` (2026-10-18).

## Capabilities

None: test code only, no requirement changes.

## Impact

`crates/tilrd/tests/{common/mod.rs,local.rs,reconnect.rs,reload.rs,files.rs,agent.rs}`, `crates/tilrd/src/cloud.rs` (a unit test). No behavior, protocol, reload or update impact. Tracker: epic TILR-76. Split from the former shared change `test-coverage-gaps` (2026-10-20: one task, one change) and archived at once: every item here was already on dev.
