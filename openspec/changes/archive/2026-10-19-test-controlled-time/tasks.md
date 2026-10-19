# Tasks

## 1. Spec

- [x] 1.1 Describe the failpoints-only shared clock and SOCKS negotiation timeout, including release behavior and event-based test scenarios (`openspec validate test-controlled-time --type change --strict`).

## 2. Implementation

- [x] 2.1 Add the shared clock reader and use it for pane timing and supervisor `PaneInfo` timestamps (`panes::used_at_ms_follows_subscribe_and_input`).
- [x] 2.2 Use the shared test clock for the raw SOCKS negotiation deadline under `failpoints` (`socks::a_session_that_does_not_say_where_it_goes_is_closed`).
- [x] 2.3 Add integration tests for exact pane use timestamps and stalled SOCKS closure, and record use cases in `USE_CASES.md` (`cargo test -p tilrd --features failpoints --test panes --test socks --locked`).

## 3. Verification

- [x] 3.1 Run OpenSpec validation and repository format, lint and test checks.
- [x] 3.2 Rebase on `dev`, verify committed state, archive this change and validate all specs (`git rebase dev`; `cargo test --workspace --features tilrd/failpoints --locked`; `openspec validate --all --strict`).
