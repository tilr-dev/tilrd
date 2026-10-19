# Tasks

## 1. Cloud

- [ ] 1.1 Extend the test-played Cloud in `crates/tilrd/tests/cloud.rs` with `/v1/enroll`, `/v1/enroll/poll` (pending, approved, 404) and `DELETE /v1/machine`; verify `cloud::login_prints_the_code_and_polls_until_confirmed` and `cloud::login_keeps_the_link_once_confirmed_and_an_expired_code_links_nothing` (link file 0600, nothing written on 404)
- [ ] 1.2 Logout and revoke; verify `cloud::logout_unlinks_the_machine_whether_or_not_cloud_hears` and `cloud::a_link_cloud_revokes_stops_the_tunnel_until_linked_again` (401 and close 4001)
- [ ] 1.3 Control connection retry with the tunnel kept; verify `cloud::the_control_connection_is_retried_and_the_tunnel_kept_meanwhile` (Cloud goes away and comes back; `cloudflared` pid unchanged)
- [ ] 1.4 A stand-in `cloudflared` that exits; verify `cloud::a_cloudflared_that_exits_is_reported_and_started_again` (`tunnel_status` `failed`, restarted after 5 s)
- [ ] 1.5 No TCP port; verify `cloud::a_server_without_a_tcp_port_does_not_connect_to_cloud` (log line, Cloud sees no connection)
- [ ] 1.6 USE_CASES.md entries for 1.1-1.5; the `cloud-tunnels` Notes list shrinks; verify by review

## 2. Checks

- [ ] 2.1 Run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo test --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 .`; on macOS also the Linux run via `scripts/linux.sh`; verify all green
- [ ] 2.2 Refresh each MODIFIED requirement in `specs/` from the current `openspec/specs/` if a change archived since touched it; drop "(planned …)" from the **Verified by** lines; remove the tested items from the specs' Notes; verify `openspec validate cloud-tunnels-tests --strict`
