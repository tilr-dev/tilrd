# Tasks

## 1. Ownership of service files

- [ ] 1.1 Marker check on install and uninstall; verify with itests UC-022 `uninstall_leaves_what_it_did_not_write` plus a new install case with a foreign `tilr.service`
- [ ] 1.2 Locale-independent crontab detection; verify with UC-021 run under `LANG=de_DE.UTF-8`

## 2. Private state directory

- [ ] 2.1 chmod 0700 an existing state directory; verify with itests UC-005 starting from a 0755 `~/.local/state/tilr`

## 3. Checks

- [ ] 3.1 Run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo test --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 .`; on macOS also `scripts/linux.sh`; verify all green
