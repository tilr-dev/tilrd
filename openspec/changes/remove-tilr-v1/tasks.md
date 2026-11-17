# Tasks

Shared design: `openspec/design/unified-bus.md`. Numbers in brackets are the tasks of the
former `unified-bus` (its `tasks.md` at 75fc8ce0).

## 1. Remove tilr.v1

- [ ] 1.1 [5.2] Remove `tilr.v1` serving, `crates/tilr-proto` and `tilr.proto` (the internal schemas are already in `internal.proto`, `bus-reload`); `crates/tilr-screen` and the pane process on `tilr.pane.v1` (T1); the old-client refusal packet (B3); the four protocol tests on the bus and `tests/common/v1.rs` removed; CI step `! test -e crates/tilr-proto`; verify `bus::a_client_of_the_old_protocol_is_told_and_closed`, `bus_terminal::tilrd_provides_the_terminal_methods`, `panes::a_malformed_packet_gets_an_error_and_the_connection_stays`, `panes::a_message_type_the_server_does_not_know_is_ignored`, `panes::a_client_that_skips_hello_is_turned_away`, `panes::a_client_built_for_another_protocol_version_is_turned_away`, `rg -n "tilr_proto|tilr\.v1" crates` prints only the refusal constant
- [ ] 1.2 [11.1] README "Protocol" rewritten for `tilr.bus.v1` (transports, envelopes, the namespaces and the T2 table in short, version rule, features, close codes 4000-4003, the old-client refusal); AGENTS.md: the `tilr-proto` and protocol lines point at `crates/tilr-bus/proto/`; `openspec/config.yaml` context lines about `tilr.proto` updated; verify by review
- [ ] 1.3 [M3] Release notes of the new minor: README "Updating" says that moving from a build before the bus to a bus build is a restart of tilrd that ends open shells, while reloads between bus builds keep them; the same paragraph in the PR description for the owner's release; verify by review
- [ ] 1.4 [11.2] The main specs that name `tilr.v1` messages (`access-control`, `client-liveness`, `clipboard-host-events`, `exec`, `files`, `metrics`, `panes`, `reconnect-testing`, `reload`, `screen-frames`, `ssh-agent-forwarding`, `tilr-cli`, `update`) get MODIFIED deltas in this change with the bus names of design T2 and unchanged behavior, skipping the requirements owned by other changes (design O3); spec tags of itests updated if a `**Verified by**` line changes, `./itests/dt coverage` exits 0 (local); verify `openspec validate remove-tilr-v1 --strict` and `rg -n "ClientMessage|ServerMessage|CreatePane|Subscribe\b|HelloAck\.features" openspec/specs` empty after a dry-run archive (`openspec archive remove-tilr-v1 --dry-run` if available, else review)

## 2. Checks and archive

- [ ] 2.1 [12.1] Run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo nextest run --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 --log-opts=HEAD .`, `scripts/sdk-check.sh`; on macOS also the Linux run via `scripts/linux.sh` (the kernel must have Landlock ABI ≥ 4; note it if not); verify all green
- [ ] 2.2 [12.2, 11.3] Update the Phase 7 section of `openspec/ROADMAP.md` (this row to "Done"; `bus-gateway` and `files-and-edit-followups` rows now depend on nothing of the phase), record the commit in proposal.md and archive; verify `openspec validate --all --strict`
