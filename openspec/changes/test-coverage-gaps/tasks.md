# Tasks

## 1. Cloud

- [ ] 1.1 Tests for the 8 untested `cloud-tunnels` requirements against the test-played Cloud; verify each appears in the spec as `cloud::<name>` and the Notes list shrinks

## 2. Release process

- [ ] 2.1 A dry-run test of `scripts/release.sh --upload-only` against a local directory CDN (layout, `latest` last, account id required); verify in CI

## 3. The rest

- [ ] 3.1 Tests for the other untested requirements listed in Why; verify `grep -c 'Requirements without a test: none'` rises accordingly

## 4. Flakes

- [ ] 4.1 Find and fix each flake or mark it with its cause; verify 10 consecutive green runs of the affected tests (`cargo test` and `./dt` for uc_043)
- [ ] 4.2 Find who leaves `target/debug` test servers running; verify `pgrep -f target/debug/tilrd` empty after `cargo test`

## 5. Checks

- [ ] 5.1 Run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo test --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 .`; on macOS also `scripts/linux.sh`; verify all green
