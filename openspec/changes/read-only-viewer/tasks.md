# Tasks

## 0. Decision

- [x] 0.1 Decided 2026-10-18: deferred until a use is named; groups 1+ are the ready plan (option (a)) and wait for that use; verify `openspec validate read-only-viewer --strict`

## 1. Token

- [ ] 1.1 `viewer-token` file, `tilrd token --viewer [--new]` and `tilr token --viewer [--new]`, read at each TCP `Hello` (D1); `HelloAck.read_only`; verify with `auth::the_viewer_token_makes_a_read_only_connection`; README "Who may connect"

## 2. Gate

- [ ] 2.1 `HelloAck.read_only`, `Viewer.read_only` in `tilr.proto`; README "Protocol"; verify `cargo test -p tilr-proto`
- [ ] 2.2 Allow list in the supervisor, no host events / agent streams / clipboard writes to read-only connections, empty features (D2, D3); reconcile the `protocol` "Server feature flags" wording ("on this connection"); verify with `auth::a_read_only_connection_sees_frames_and_changes_nothing`; USE_CASES.md entries
- [ ] 2.3 Replacing the token cuts off viewers (D4); verify with `auth::replacing_the_viewer_token_cuts_off_its_viewers`

## 3. itests

- [ ] 3.1 UC-03B in itests/USE_CASES.md and `itests/scenarios/uc_03b.py::a_viewer_sees_the_pane_and_nothing_it_sends_reaches_it` (a second driver on another host with the viewer token types, pastes, resizes; the pane's shell records nothing), tagged `access-control/a-read-only-connection-watches-and-changes-nothing`; verify `./itests/dt UC-03B` green

## 4. Checks and archive

- [ ] 4.1 Run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo test --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 .`; on macOS also the Linux run via `scripts/linux.sh`; verify all green
- [ ] 4.2 `./itests/dt UC-031 UC-03B` green, then `./itests/dt coverage` exits 0
- [ ] 4.3 Record the commit in proposal.md ("Implemented by") and archive the change; verify `openspec validate --all --strict`
