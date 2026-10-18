# Tasks

## 1. Peer and directory checks in tilr

- [ ] 1.1 Check directory owner/mode/symlink and peer uid; verify with itests UC-034 extended: a socket served by uid 65534 in a directory of that uid is refused by `tilr` with a message naming the owner
- [ ] 1.2 Unit tests in `crates/tilr/src/conn.rs` for the directory checks; verify by review against this proposal

## 2. Token to loopback only

- [ ] 2.1 Send the stored token only to loopback targets; verify with `client::the_stored_token_is_not_sent_to_a_remote_target` (a fake remote server sees no token)
- [ ] 2.2 README "Targets" documents the rule; verify by review against this proposal

## 3. No symlink following

- [ ] 3.1 O_NOFOLLOW for lock and log; verify with `client::a_symlinked_lock_file_is_refused`

## 4. Checks

- [ ] 4.1 Run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo test --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 .`; on macOS also `scripts/linux.sh`; verify all green
