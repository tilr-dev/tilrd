# Tasks

## 1. Tokens

- [ ] 1.1 Add `biscuit-auth` (exact version, `--locked`); check `cargo deny check` and the MSRV job; verify both green, `deny.toml` changed only if a license must be allowed (named in the commit)
- [ ] 1.2 Root key file (D6 location), minting from `rights` (D1-D3), `HelloAck.credential`, `calls` refused with the hint; verify `bus_acl::the_root_key_is_made_once`, `bus_acl::an_instance_may_call_what_its_rights_list`, `bus_acl::control_methods_need_control_rights`; README "Components" rights; USE_CASES.md entries

## 2. Verification

- [ ] 2.1 Broker authorizer with parsed-token cache, revocation check, decision cache, run and size limits (D5, D7); verify `bus_acl::a_token_of_another_server_is_refused`, `bus_acl::a_per_call_credential_authorizes_that_call`, `bus_acl::an_attenuated_token_only_narrows`, `bus_acl::authorization_that_runs_too_long_is_denied`; a criterion benchmark `benches/authorize.rs` recorded in the PR (cached decision under 1 µs on CI hardware is a note, not a gate)
- [ ] 2.2 Expiry and renewal (D2, controlled time from `test-controlled-time`); verify `bus_acl::an_expired_token_is_refused`, `bus_acl::an_instance_token_is_renewed`

## 3. Owner commands

- [ ] 3.1 `bus.token`, `bus.revoke`, `bus.rotate_key`, revocation file (D6); `tilr bus token|revoke|rotate-key`, `BusCall.credential`; verify `client::tilr_bus_token_mints_a_token_that_works`, `bus_acl::a_revoked_token_stops_working_at_once`, `bus_acl::a_stopped_instances_token_is_revoked`, `bus_acl::rotating_the_root_key_ends_every_token`; README; USE_CASES.md entries
- [ ] 3.2 Audit lines (D9); verify `bus_acl::denied_and_control_calls_are_audited`

## 4. Client library

- [ ] 4.1 `tilr-bus`: `credential()`, `Credential::attenuate`, per-call credential, `CredentialUpdate` handling; verify unit tests in `crates/tilr-bus` and the tests of 2.1

## 5. Reload

- [ ] 5.1 `bus-state 2` with token ids and renewal times; verify `reload::bus_instances_survive_a_reload` extended (instance calls still allowed after the reload, its token revoked after a later stop)

## 6. Checks and archive

- [ ] 6.1 Run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo nextest run --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 --log-opts=HEAD .`; on macOS also the Linux run via `scripts/linux.sh`; verify all green
- [ ] 6.2 Update the row in `openspec/ROADMAP.md`, record the commit in proposal.md and archive; verify `openspec validate --all --strict`
