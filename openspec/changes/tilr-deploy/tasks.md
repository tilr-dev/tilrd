# Tasks

Written for option (a), decided 2026-10-18 (see the proposal's Decision).

## 1. Fetching a release locally

- [x] 1.1 Hidden `tilrd fetch-release --platform P --out DIR [--cdn URL]` reusing `update.rs` (download, `.sha256`, archive holds both programs); verify with `update::fetch_release_writes_a_checked_archive` and `update::fetch_release_refuses_a_bad_checksum` against the test CDN of `crates/tilrd/tests/update.rs`

## 2. The command

- [x] 2.1 `tilr deploy` argument parsing, `ssh` with a private `ControlPath` (D1), platform mapping (D2) and archive choice (D3); verify with unit tests `deploy::tests::uname_names_the_platform` and `client::deploy_names_a_platform_without_a_release` (a stub `ssh` on `PATH` answering `uname -sm` with `SunOS i86pc`)
- [x] 2.2 Remote install script and start (D4, D5); verify with `client::deploy_through_a_stub_ssh_installs_both_and_prints_how_to_connect` (stub `ssh` that runs the command locally with `HOME` in a temp dir)
- [x] 2.3 README `tilr` commands and USE_CASES.md entries; `tilr-cli` "Command set" lists `deploy` in the same commit; verify by review and `openspec validate --specs --strict`

## 3. Black-box case

- [x] 3.1 New case UC-07C (`itests/scenarios/uc_07c.py`, group G): a `laptop` host deploys to a host reached with real `ssh` (key auth), fresh and with a running tilrd and pane; a damaged `--archive`; tags `tilr-cli/tilr-deploy-installs-and-starts-tilrd-over-ssh#a-fresh-host`, `#a-damaged-archive`, `#a-server-already-running-there`; verify `./itests/dt --only scenarios --filter uc_07c` green

## 4. Checks and archive

- [x] 4.1 Before marking a change done, run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo test --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 .`; on macOS also the Linux run via `scripts/linux.sh`
- [x] 4.2 `./itests/dt --only scenarios --filter uc_07c` green; `./itests/dt coverage` exits 0
- [x] 4.3 Archive the change
