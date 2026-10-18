# Tasks

## 1. Version 3

- [ ] 1.1 `PROTOCOL_VERSION = 3` in `crates/tilr-proto/src/lib.rs`; verify with `panes::a_client_built_for_another_protocol_version_is_turned_away` (2 and 4 refused with "this server speaks 3", 0 too) and `cargo test -p tilr` (`tilr` built against 3 talks to the server)
- [ ] 1.2 Drop `FEATURE_PING` from the features list (`crates/tilrd/src/supervisor.rs`); verify with `panes::hello_announces_the_clipboard_and_its_limit` expecting `["clipboard", "files", "host-events", "agent", "exec"]`, and `clipboard::the_clipboard_can_be_turned_off`, `files::files_can_be_turned_off`, `agent::the_agent_is_forwarded_unless_turned_off`, `exec::exec_is_offered_unless_turned_off` green
- [ ] 1.3 `tilr-driver`: drop the default `Hello.features` of `["ping"]` (`crates/tilr-driver/src/main.rs`); verify `cargo test -p tilr-driver` and the itests in 3.2
- [ ] 1.4 Proto comments (design D1, D4, D5): header states the rule; `HelloAck.features` without `"ping"` and without the "only towards a client that has agent" clause; `KeyInput.code` empty = no physical key; the files section says a v3 client sends back the bytes it was given; verify by review of `tilr.proto` against this proposal
- [ ] 1.5 Move the empty-`code` assertions of `ghostty::tests::ctrl_and_alt_on_a_layout_that_is_not_latin_go_by_the_physical_key` into a unit test of their own, `ghostty::tests::a_key_without_a_physical_key_is_encoded_as_on_a_us_layout` (Ctrl+'с' without code: nothing; Ctrl+'c' without code: 0x03), as the screen-frames delta cites; verify `cargo test -p tilr-screen`

## 2. Documentation

- [ ] 2.1 README "Protocol": replace "The protocol grows by adding message types ... say what they support in features" with the v3 rule (D1), list the features and the switch of each, say `Ping` is always there, and that the `server` plugin and tilr-mac need v3; verify by review against this proposal
- [ ] 2.2 `openspec/config.yaml` context "Protocol" paragraph: version 3 in code; verify `openspec validate --all --strict`
- [ ] 2.3 USE_CASES.md: the features entry (`hello_announces_the_clipboard_and_its_limit`) no longer names `ping`; verify by review

## 3. Checks and archive

- [ ] 3.1 Before marking a change done, run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo test --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 .`; on macOS also the Linux run via `scripts/linux.sh`; verify all green
- [ ] 3.2 `./itests/dt` (every case starts the v3 driver; at least UC-035, UC-046, UC-064, UC-070, UC-071, UC-075, UC-077, which send or read features) green, then `./itests/dt coverage` exits 0
- [ ] 3.3 Record the implementing commit in proposal.md and archive the change; verify `openspec list` no longer shows it
