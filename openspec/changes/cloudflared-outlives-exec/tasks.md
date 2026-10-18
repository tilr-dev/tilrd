# Tasks

## 1. Process group and parent-death signal

- [ ] 1.1 Start `cloudflared` with `process_group(0)` and, on Linux, `PR_SET_PDEATHSIG` SIGTERM with the `getppid` check (design D1); verify with `cloud::cloudflared_is_not_in_the_servers_process_group` (the stand-in's pgid differs from the server's; SIGHUP to the server's group leaves its pid)
- [ ] 1.2 itests UC-088: new `itests/scenarios/uc_088.py::cloudflared_is_gone_after_the_server_is_sigkilled` (Cloud played by the test as in UC-07A, stand-in `cloudflared`), spec tag `cloud-tunnels/cloudflared-does-not-outlive-the-server#sigkill`; itests/USE_CASES.md UC-088 points to it; verify `./itests/dt uc_088` green

## 2. Pid file and sweep

- [ ] 2.1 Write and remove `<runtime dir>/cloudflared/pid`; sweep a live leftover running this directory's `config.yml` at start (design D2); verify with `cloud::a_cloudflared_left_by_a_killed_server_is_stopped` (SIGKILL the server, start another on the same runtime directory: the old stand-in is gone before the new one starts, the log has `stopped a cloudflared left by an earlier server`) and a unit test that a pid running something else is left alone
- [ ] 2.2 README "tilr Cloud" says `cloudflared` runs in its own process group and does not outlive the server (SIGKILL: at once on Linux, at the next start elsewhere); USE_CASES.md entries for the new tests; verify by review against this proposal

## 3. Stuck cloudflared at a reload

- [ ] 3.1 Test only (behavior exists): `cloud::a_cloudflared_ignoring_sigterm_does_not_hold_back_a_reload` with a stand-in that traps SIGTERM; verify it passes and the `reload` spec's Verified by names it; USE_CASES.md entry
- [ ] 3.2 Update the `cloud-tunnels` Notes (drop "cloudflared does not outlive the server" from the untested list and the "not met at HEAD" bullet); verify `openspec validate --specs --strict`

## 4. Checks and archive

- [ ] 4.1 Before marking a change done, run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo test --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 .`; on macOS also the Linux run via `scripts/linux.sh`; verify all green
- [ ] 4.2 `./itests/dt uc_088 uc_07a uc_013` green, then `./itests/dt coverage` exits 0 and `openspec/coverage/` is regenerated
- [ ] 4.3 Record the commit in proposal.md ("Implemented by") and archive the change; verify `openspec list` no longer shows it
