# Tasks

## 1. Flake

- [x] 1.1 `files::nothing_outside_the_shared_directories_is_served`: establish the startup root lookup race (design D1), reproduce it deterministically by waiting for `files on` with the old fixture order, and keep fixture creation before server startup. The regression test waits for `files on` after setup; it fails with the old order and passes with the fixed order.
- The other test of the former shared item (5.1 of `test-coverage-gaps`), `reconnect::a_client_that_still_sends_but_no_longer_hears_the_server_is_dropped`, was closed by TILR-115 (archived change `harness-flake-fixes`, 5.5).

## 2. Checks

- [x] 2.1 Run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo test --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 .`; on macOS also the Linux run via `scripts/linux.sh`; verify all green
