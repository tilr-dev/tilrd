# Proposal: Tests for the requirements that have none; known flakes

## Why

The specs written on 2026-10-18 list requirements without a test in their Notes; `openspec/coverage/coverage.md` (`./itests/dt coverage`, last regenerated 2026-10-18) counts 349 requirements, 35 uncovered: `cloud-tunnels` 8, `release-process` 13, `installation` 1, `update` 2, `tilr-cli` 2, `socks-proxy` 2, `panes` 2, `exec` 1, `metrics` 1, `reload` 1, `ssh-agent-forwarding` 1, `daemon-lifecycle` 1. Checked at 1163abd: none of them has gained a test (`crates/tilrd/tests/cloud.rs` has five tests, none of login, logout, revoke, retry, a failing `cloudflared`, `--no-listen` or a SIGKILLed server; `crates/tilr/tests/client.rs` runs neither `tilr kill` nor `tilr update`). USE_CASES.md says Cloud login/logout were checked only by hand against a local Cloud.

Known flakes (Basic Memory `main/projects/tilr/tilrd-tests-integratsionnye-testy-tilrd-na-dagger-use-cases-resheniia`): `uc_043` p90 echo over 5 s in 2 of about 7 runs; under load `files::nothing_outside_the_shared_directories_is_served` and `reconnect::a_client_that_still_sends_but_no_longer_hears_the_server_is_dropped`; test servers from `target/debug` left running on the Mac after local cargo tests. Not re-run for this proposal.

Flakes seen in phase 1 (2026-10-18, implementing attach-robustness, sighup-ignoring-shell-reaped and cloudflared-outlives-exec on dev; each under full-suite load, each green when run alone):

- `reload::a_broken_executable_is_not_reloaded_into` (macOS, 2 of 5 full runs): `not reloading` not logged within the test's 10 s after the executable is replaced (`crates/tilrd/tests/reload.rs:131-138`); the watch, the check child and the log line all run under the load of the other tests.
- `local::a_server_that_stops_removes_its_socket_and_lock`: a real bug, not the test's: a SIGTERM between the Unix socket listening and the signal handlers being installed kills the server without cleanup. Fixed by its own change, `startup-signal-window` (TILR-106).
- `local::a_server_with_a_port_does_not_reload_into_one_that_cannot_take_its_socket` (Linux container): `start tilrd: Text file busy` (ETXTBSY) — the harness's `spawn_env` (`crates/tilrd/tests/local.rs:33-40`) spawns a copied executable with `Command::spawn`, not with `common::spawn_copied`, which retries while another test thread's fork still holds the copy's write descriptor.
- `reconnect::a_client_that_still_sends_but_no_longer_hears_the_server_is_dropped`: the harness asserts on a log line before its reader thread has collected it (the server's stderr is drained by a thread into `Server::log`; `Server::logged` reads a snapshot).
- `reconnect::output_printed_while_nobody_watched_is_there_on_reconnect` (Linux container): timed out waiting for `printed later` on the screen after the reconnect (`tests/common/mod.rs:324`).
- itests `uc_063` (a client typing through an update): one typed line lost during an update.
- itests `uc_033`: port 7890 already in use (EADDRINUSE) in its container.

## Scope

- The 13 `release-process` requirements move to their own change, `release-process-tests` (scripts and workflows, phase "release infrastructure"); this one keeps the server and client requirements.
- `daemon-lifecycle` "No shell outlives the server" is left to `sighup-ignoring-shell-reaped`, which fixes it and turns UC-086 green.
- `cloud-tunnels` "cloudflared does not outlive the server" is left to `cloudflared-outlives-exec`, which fixes it and adds its tests (UC-088); this change covers the other 7 `cloud-tunnels` requirements.
- `cloud-tunnels` "Login links the machine through a device code" is also modified by `cloud-default-url`: whichever lands second re-copies the requirement from `openspec/specs/` before archiving.

## What Changes

- A test per remaining untested requirement (19), cargo where one machine suffices, itests otherwise; each named in its requirement's **Verified by** line (spec deltas below):
  - `cloud-tunnels` (7): against the test-played Cloud of `crates/tilrd/tests/cloud.rs`, extended with the enrollment endpoints (`/v1/enroll`, `/v1/enroll/poll`, `DELETE /v1/machine`) and a stand-in `cloudflared` that can be made to exit.
  - `exec`, `metrics`, `panes` (2), `reload`, `socks-proxy` (2), `ssh-agent-forwarding`: cargo tests in `crates/tilrd/tests/`.
  - `tilr-cli` (2) and `update` (2): cargo tests in `crates/tilr/tests/client.rs`; `tilr update` against a local CDN directory as `crates/tilrd/tests/update.rs` uses.
  - `installation` "Manual install from the CDN": a new itests case UC-007 doing the README's manual steps against `LocalCdn`.
- Fix or quarantine each known flake with a recorded cause.
- A test that finds the code differs from its requirement is kept red and the difference goes to its own change (no behavior change here).

## Capabilities

### Modified Capabilities
- `cloud-tunnels`, `exec`, `installation`, `metrics`, `panes`, `reload`, `socks-proxy`, `ssh-agent-forwarding`, `tilr-cli`, `update`: **Verified by** lines name the new tests (see `specs/`); requirement text unchanged.

## Non-goals

- A real Cloudflare tunnel in tests (needs an account; stays manual).
- Load thresholds (change `itests-stage-5`).
- The release scripts and workflows (change `release-process-tests`).
- Behavior changes.

## Impact

Tests only: `crates/tilrd/tests/{cloud,exec,metrics,panes,reload,socks,agent}.rs`, `crates/tilr/tests/client.rs`, `itests/scenarios/uc_007.py`, USE_CASES.md, itests/USE_CASES.md, the specs' **Verified by** lines and Notes. No behavior, protocol, reload or update impact. The deltas are MODIFIED copies of the current requirements with a new **Verified by** line: if another change modifies one of them first (e.g. `cloudflared-outlives-exec`), the copy is refreshed before this change is archived (task 6.2).
