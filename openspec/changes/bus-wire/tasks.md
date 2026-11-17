# Tasks

Shared design: `openspec/design/unified-bus.md`. Numbers in brackets are the tasks of the
former `unified-bus` (its `tasks.md` at 75fc8ce0).

## 1. Crate tilr-bus

- [x] 1.1 [1.1] `crates/tilr-bus`: `proto/bus.proto` (design B4), codec for WebSocket (flag byte, zstd, no double compression) and stream sockets (u32 length), 16 MiB limit both ways, error codes, workspace member; verify unit tests `codec::tests::small_envelopes_stay_plain`, `codec::tests::large_envelopes_are_compressed`, `codec::tests::a_compressed_chunk_is_not_compressed_again`, `codec::tests::rejects_garbage`, `codec::tests::a_frame_over_the_limit_is_refused`, `codec::tests::unknown_envelope_variants_are_ignored`; `cargo deny check`
- [x] 1.2 [1.2] Payload schemas `proto/tilr/{pane,server,fs,exec,clipboard,host,agent}/v1/*.proto` with the types copied from `tilr.proto` (field numbers kept, design T1), `PaneOutput{frame | images}`, and the requests and replies of every method of the phase (T2, T4-T9, P6, R3-R5, B10), so that no later change edits a schema; verify `cargo test -p tilr-bus` and a unit test decoding a `tilr.v1` `Frame` fixture as `tilr.pane.v1.Frame`
- [x] 1.3 [1.3] Async Rust client: connect (fd, instance socket, WebSocket with access token or credential), `call`, `provide`, `stream` with credits, `emit`, `subscribe`, `activate`, `CredentialUpdate`, with the frozen surface of this change's design (`Credential::attenuate` is `bus-rights`'s); verify unit tests in `crates/tilr-bus`
- [x] 1.4 [1.4] Test runtime `crates/tilr-bus/examples/test_runtime.rs` (`test.echo`, `test.whoami`, `test.from`, `test.env`, `test.sleep`, `test.nested`, `test.deadline`, `test.close_bus`, `test.record`, `test.count`, `test.flood`, `test.bulk`, `test.fail`, `TEST_NO_HELLO`), connecting over fd, instance socket or WebSocket; verify it is built by the tests and not in the release archive (`scripts/release.sh` lists only `tilrd` and `tilr`)

## 2. Checks and archive

- [ ] 2.1 [12.1] Run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo nextest run --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 --log-opts=HEAD .`; on macOS also the Linux run via `scripts/linux.sh`; verify all green and every existing test passes under its name
- [ ] 2.2 [12.2] Update this change's row in `openspec/ROADMAP.md`, record the commit in proposal.md and archive; verify `openspec validate --all --strict`
