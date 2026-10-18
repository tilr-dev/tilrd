# Tasks

## 1. Docs

- [ ] 1.1 Fix each README line listed; verify by `grep -n` for the old wording (none left) and by re-reading the matching spec Notes
- [ ] 1.2 Fix USE_CASES.md:342-343 and add the three missing test entries; verify with a script listing every `fn` in `crates/*/tests/*.rs` absent from USE_CASES.md (empty output)
- [ ] 1.3 Decide the 4000-reconnect question with the owner; record the answer in README and the `client-liveness` spec Notes; verify by review against this proposal
- [ ] 1.4 Remove the contradiction bullets from the specs' Notes once fixed; verify `openspec validate --specs --strict`

## 2. Checks

- [ ] 2.1 Run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo test --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 .`; on macOS also `scripts/linux.sh`; verify all green
