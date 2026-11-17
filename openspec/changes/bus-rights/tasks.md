# Tasks

Shared design: `openspec/design/unified-bus.md`. Numbers in brackets are the tasks of the
former `unified-bus` (its `tasks.md` at 75fc8ce0).

## 1. Rights

- [ ] 1.1 [3.1] Add `biscuit-auth` (exact version, `--locked`) to `tilrd` and `tilr-bus`; `Credential::attenuate` in `tilr-bus` [from 1.3]; verify `cargo deny check` and the MSRV job green, `deny.toml` changed only if a license must be allowed (named in the commit), a unit test of `Credential::attenuate`
- [ ] 1.2 [3.2] `crates/tilrd/src/bus/acl.rs`: root key, minting from `rights` (R2, R3), `HelloAck.credential`, `calls` refused with the hint, provide/emit/subscribe/control checks, origin fact, parsed-token and decision caches, run and size limits (R6, R7); `default_authorizer` returns it; verify `bus_acl::the_root_key_is_made_once`, `bus_acl::the_owner_holds_every_right`, `bus_acl::a_tcp_participant_needs_a_token`, `bus_acl::an_instance_may_do_what_its_rights_list`, `bus_acl::providing_emitting_and_subscribing_need_rights`, `bus_acl::control_methods_need_control_rights`, `bus_acl::a_token_of_another_server_is_refused`, `bus_acl::a_per_call_credential_is_a_bearer`, `bus_acl::an_attenuated_token_only_narrows`, `bus_acl::authorization_that_runs_too_long_is_denied`, `bus_acl::a_revocation_clears_cached_decisions`; a criterion benchmark `benches/authorize.rs` recorded in the PR
- [ ] 1.3 [3.3] Expiry, renewal (`CredentialUpdate` both ways, `bus.renew`), revocation file, rotation, close code 4003, audit (R5, R8), the methods `bus.token`, `bus.revoke`, `bus.rotate_key`; verify `bus_acl::an_expired_token_is_refused`, `bus_acl::an_instance_token_is_renewed`, `bus_acl::an_outside_token_is_renewed_on_its_connection` (controlled time), `bus_acl::a_revoked_token_stops_working_at_once`, `bus_acl::a_stopped_instances_token_is_revoked`, `bus_acl::rotating_the_root_key_ends_every_token`, `bus_acl::denied_and_control_calls_are_audited`; README "The bus" rights subsection (with close code 4003); USE_CASES.md entries (own section)

## 2. Checks and archive

- [ ] 2.1 [12.1] Run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo nextest run --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 --log-opts=HEAD .`; on macOS also the Linux run via `scripts/linux.sh`; verify all green and every existing test passes under its name
- [ ] 2.2 [12.2] Update this change's row in `openspec/ROADMAP.md`, record the commit in proposal.md and archive; verify `openspec validate --all --strict`
