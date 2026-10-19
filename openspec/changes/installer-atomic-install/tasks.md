# Tasks

Written for option (c), decided 2026-10-18 (see the proposal's Decision).

## 0. Check

- [x] 0.1 UC-004 on `dev`: fixed by 3ba0e93 in `itests/installer/install.sh` (UC-001..006 green per its commit and itests/USE_CASES.md group A)

## 1. One installer

- [ ] 1.1 `git mv itests/installer/install.sh scripts/install.sh`; itests mount it from `scripts/` (`itests/src/tilr_itests/main.py:517`, `itests/tt/__init__.py`), itests/README.md and the `installation` spec's wording ("the installer, `scripts/install.sh`") follow; verify `./itests/dt --only scenarios --filter uc_00` (UC-001..006 PASS)
- [ ] 1.2 README "Install" names `scripts/install.sh` as what `tilr.sh` serves; verify by review

## 2. Published with the release

- [ ] 2.1 `scripts/release.sh` uploads `install.sh` after every `latest`, never after a failed upload; verify with the new test `release::the_installer_is_uploaded_after_latest` in `crates/tilrd/tests/release.rs` (a `dist/cdn` laid out in a temp dir, `CLOUDFLARE_ACCOUNT_ID` set, a stub `wrangler` on `PATH`; both scenarios of the spec; the harness is shared with `release-process-tests` 1.1) run in CI
- [ ] 2.2 The `release-process` spec's Notes drop "The release scripts and workflows have no automated tests" for this requirement; verify `openspec validate --specs --strict`

## 3. First release

- [ ] 3.1 After the owner merges the first release PR (his explicit word "в main"), check the `cdn` job's log names `install.sh` last; verify `curl -fsSL https://tilr.sh | diff - scripts/install.sh` is empty and `curl -fsSL https://tilr.sh | sh` installs that release on a clean Linux user

## 4. Checks and archive

- [ ] 4.1 Before marking a change done, run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo test --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 .`; on macOS also the Linux run via `scripts/linux.sh`
- [ ] 4.2 `./itests/dt --only scenarios --filter uc_00` green, then `./itests/dt coverage` exits 0
- [ ] 4.3 Record the commit in proposal.md ("Implemented by") and archive the change
