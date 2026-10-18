# Tasks

## 1. Version 3

- [ ] 1.1 `PROTOCOL_VERSION = 3` in `crates/tilr-proto/src/lib.rs`; `tilr` and `tilr-driver` send it; verify with `panes::a_client_built_for_another_protocol_version_is_turned_away` (v2 refused with "this server speaks 3") and `cargo test -p tilr`
- [ ] 1.2 Drop `"ping"` from `HelloAck.features`; verify with `panes::hello_announces_the_clipboard_and_its_limit` expecting `["clipboard", "files", "host-events", "agent", "exec"]` and the `--no-*` tests listed in the spec
- [ ] 1.3 Proto header and `HelloAck.features` comments state D1 (version = contract, features = switches) and drop `"ping"`; verify by review of `tilr.proto`
- [ ] 1.4 itests: the driver speaks v3; a case that a v2 `Hello` is refused (spec tag `protocol/protocol-version-check#older-client`); verify with `./itests/dt` green for UC-071 and the new case, then `./itests/dt coverage`

## 2. Documentation

- [ ] 2.1 README "Protocol": replace "grows by adding ... say what they support in features" with the v3 rule (D1), list the features and what each switch is, and say the `server` plugin and tilr-mac need v3; verify by review against this proposal
- [ ] 2.2 `openspec/config.yaml` context "Protocol" paragraph states the rule and version 3; verify `openspec validate --all --strict`

## 3. Checks

- [ ] 3.1 Run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo test --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 .`; on macOS also `scripts/linux.sh`; verify all green
