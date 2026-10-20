# Tasks

Research change (TILR-138): this change is designed, not implemented. Groups 1+ are cut into implementation tasks once the owner has answered Q1-Q9 (and Q4b, Q5a, Q7b, Q7c) in design.md; the spec deltas are rewritten to the answers first.

## 0. Decision

- [x] 0.1 Write proposal, design (options Q1-Q9 (and Q4b, Q5a, Q7b, Q7c) with recommendations), spec deltas for the recommended options and this plan; verify `openspec validate multi-user-sessions --strict`
- [ ] 0.2 Owner answers Q1-Q9 (and Q4b, Q5a, Q7b, Q7c); spec deltas (including the requirements in design "Requirements to modify after the answers"), design and the groups below updated to the answers; the `Блокер:` line removed; verify `openspec validate multi-user-sessions --strict`

## 1. System mode and its paths

- [ ] 1.1 `tilrd --system`: root and capability check before anything is bound, `--no-auth` refused, `/run/tilr`, `/var/lib/tilr`, `/etc/tilr/server.toml` defaults (daemon-lifecycle "System mode needs root", "Paths and modes of a system server"); verify `local::system_mode_needs_root` (non-root run) and itests UC-03C (`uc_03c.py::tilrd_system_refuses_to_start_without_root`); README "Running as a system service"; UC-03C added to itests/USE_CASES.md
- [ ] 1.2 Per-user directories `/run/tilr/users/<uid>/` owned by root (0711), sockets bound by root and handed to the user (0600), only `panes/` owned by the user and never opened by root (daemon-lifecycle "Per-user directories of a system server", "Root makes nothing inside a user's directory"); verify itests UC-03C (`uc_03c.py::a_user_cannot_plant_a_link_in_her_directory`, `uc_03c.py::a_link_in_panes_is_not_written_through_as_root`, `uc_03c.py::a_crashed_pane_leaves_no_state_behind`); UC-03C entry in itests/USE_CASES.md; README "Running as a system service" (the directories)

## 2. Who the connection is

- [ ] 2.1 Unix socket in system mode: peer uid → connection's user, `HelloAck.user`, feature `multi-user` in `tilr.proto` (README "Protocol"); client accepts a uid-0 server on the system socket; verify itests UC-03C (`uc_03c.py::two_users_on_the_system_socket_are_told_who_they_are`, `uc_03c.py::a_user_outside_the_group_cannot_reach_the_socket`); UC-03C entries in itests/USE_CASES.md; README "Who may connect" (the group `tilr`)
- [ ] 2.2 Per-user tokens, `tilr token [--new]` for the caller, no root token, TCP `Hello` maps a token to its user (access-control "A system server keeps a token per user"); verify `auth::a_system_server_keeps_a_token_per_user` (as root) and itests UC-03C (`uc_03c.py::a_users_token_opens_only_that_users_panes`); README "Who may connect" (per-user tokens, no root token); USE_CASES.md entry

## 3. Panes as their user

- [ ] 3.1 Pane process drops to the user (initgroups, setgid, setuid, cannot regain root) before the pty; environment from the password entry; start directory (panes "A system server runs a pane as its connection's user", "The supervisor trusts no pane process about who it is"); supervisor takes nothing about identity or routing from a pane; verify itests UC-03D (`uc_03d.py::two_users_get_panes_as_themselves`, `uc_03d.py::roots_environment_does_not_leak_into_a_pane`, `uc_03d.py::a_pane_process_cannot_claim_another_pane`); UC-03D added to itests/USE_CASES.md; README "Running as a system service" (who a pane runs as)
- [ ] 3.2 Pane visibility per user, `PaneInfo.user`, `CreatePane.run_as` for root only (panes "A connection sees and drives only its user's panes", "Only root may ask for a pane as another user"); verify itests UC-03D (`uc_03d.py::bob_does_not_see_or_drive_alices_panes`, `uc_03d.py::only_root_runs_a_pane_as_another_user`) and `panes::run_as_on_a_per_user_server_is_refused`; README "Protocol" (`PaneInfo.user`, `CreatePane.run_as`); USE_CASES.md entry
- [ ] 3.3 Exec runs as the connection's user; file requests through a per-user helper process; `files` and `exec` spec deltas; verify itests UC-03D (`uc_03d.py::files_and_exec_run_as_the_user`); README "Files" / "Exec" notes; USE_CASES.md entries

## 4. Per-user sockets

- [ ] 4.1 Host-events, agent and shim sockets in the user's directory, accepting that user and root; host events, clipboard and agent streams only to that user's connections (access-control "The host-events and agent sockets check the peer's uid"); `clipboard-host-events`, `ssh-agent-forwarding` deltas; verify itests UC-035 extended (`uc_035.py::on_a_system_server_bob_cannot_reach_alices_host_socket`, `uc_035.py::alices_clipboard_goes_to_alice_only`); itests/USE_CASES.md UC-035; README host events / agent sections

## 5. Reload, tilr, service

- [ ] 5.1 Reload in system mode: supervisor state records each pane's uid, pane processes re-exec as their user, new state version in `tilrd state-formats` (`reload` delta); verify `reload::a_system_server_reloads_with_panes_of_two_users` (as root) and itests UC-03D (`uc_03d.py::a_reload_keeps_each_users_panes_as_that_user`); USE_CASES.md entry; README "Reload" (system server)
- [ ] 5.2 `tilr` falls back to `/run/tilr/tilrd.sock`, never starts a system server, permission denied there is an error, not "no server" (daemon-lifecycle "tilr finds a system server and does not start one", "tilr outside the group tilr is told so", `tilr-cli` delta); verify `client::tilr_uses_the_system_server_when_the_user_has_none`, `client::tilr_outside_the_group_reports_the_system_server_and_starts_nothing`; README "tilr" (which socket it uses); USE_CASES.md entry
- [ ] 5.3 `tilr service install --system` (systemd system unit, launchd LaunchDaemon) — its own change if the owner wants it

## 6. Checks and archive

- [ ] 6.1 Run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo test --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 .`; on macOS also the Linux run via `scripts/linux.sh`; verify all green
- [ ] 6.2 `./itests/dt UC-034 UC-035 UC-03C UC-03D` green, spec tags of the cases match the `**Verified by**` lines, then `./itests/dt coverage` exits 0
- [ ] 6.3 Record the commits in proposal.md ("Implemented by"), move the row in openspec/ROADMAP.md to "Done" and archive the change; verify `openspec validate --all --strict`
