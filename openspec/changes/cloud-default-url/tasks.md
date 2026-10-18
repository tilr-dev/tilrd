# Tasks

## 1. Default

- [ ] 1.1 `DEFAULT_URL = "https://wss.tilr.dev"` in `crates/tilrd/src/cloud.rs`; `the_control_address_follows_the_cloud_address` uses the new host; add `cloud::tests::the_default_cloud_is_the_api_host` (asserts the constant and its control URL `wss://wss.tilr.dev/v1/machine/connect`); verify `cargo test -p tilrd --bin tilrd cloud::tests` and `tilrd cloud login --help` showing `[default: https://wss.tilr.dev]`
- [ ] 1.2 README "tilr Cloud" (README.md:379-380) names `https://wss.tilr.dev`; verify `grep -rn cloud.tilr.dev README.md crates/` prints only lines about Cloud's page, if any

## 2. Checks and archive

- [ ] 2.1 Before marking a change done, run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo test --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 .`; on macOS also the Linux run via `scripts/linux.sh`
- [ ] 2.2 `./itests/dt --only scenarios --filter uc_07a` (UC-07A writes `cloud.json` with the played Cloud's URL, so the default does not reach it) green; `./itests/dt coverage` exits 0
- [ ] 2.3 Record the commit in proposal.md and archive the change
