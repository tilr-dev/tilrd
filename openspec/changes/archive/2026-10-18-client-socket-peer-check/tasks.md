# Tasks

## 1. Peer and directory checks in tilr

- [x] 1.1 Check directory owner/mode/symlink (default socket, before probing or starting a server) and peer uid (every socket, before the handshake); verify with itests UC-03A (`uc_03a.py::a_runtime_directory_another_user_made_in_tmp_is_not_entered`, `uc_03a.py::a_socket_another_user_serves_at_a_named_path_is_refused_before_a_byte`: a socket served by another uid is refused with a message naming the owner, 0 bytes sent) and UC-034 extended (`tilr` of uid 65534 refuses root's server on its side)
- [x] 1.2 Unit test `local::tests::a_runtime_directory_of_this_user_alone_is_trusted` in `crates/tilr-proto/src/local.rs` (the check is shared there) and `client::a_runtime_directory_open_to_others_or_a_link_is_refused_and_nothing_started`; verify by review against this proposal

## 2. Token to loopback only

- [x] 2.1 Send the stored token only to loopback targets; verify with unit test `target::tests::the_stored_token_goes_to_loopback_targets_only` (resolution of the target: no stored token for a non-loopback URL; a token given for it still goes)
- [x] 2.2 README "Which server" documents the rule; verify by review against this proposal

## 3. No symlink following

- [x] 3.1 O_NOFOLLOW (and a regular file of the user's) for lock and log; verify with `client::a_symlinked_lock_file_is_refused` (lock and log) and `local::a_symlinked_lock_file_is_refused` (tilrd)

## 4. Checks

- [x] 4.1 Run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo test --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 .`; on macOS also `scripts/linux.sh`; verify all green
