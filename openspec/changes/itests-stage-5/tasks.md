# Tasks

## 1. Group H

- [ ] 1.1 Write UC-080…085 and 087 scenarios; verify each reports PASS or a recorded FAIL in `itests/results/`
- [ ] 1.2 Set thresholds after 3-5 runs; verify the thresholds are in itests/USE_CASES.md

## 2. Platforms

- [ ] 2.1 Distribution images and the `local` backend; verify `./dt --platform alpine` and `./dt --backend local` run group B

## 3. Checks

- [ ] 3.1 Run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo test --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 .`; on macOS also `scripts/linux.sh`; verify all green
