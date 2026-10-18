# Tasks

## 1. The contract in this repository

- [ ] 1.1 Check the requirement against the cited tests (`panes::a_pane_is_told_apart_by_its_id_in_its_shell_and_in_ps` asserts the version line; `reload::an_executable_that_reads_another_state_format_is_not_reloaded_into` reads `state-formats`); add `panes::ps_server_version_fails_when_nothing_listens` for the second scenario; verify `cargo test -p tilrd --test panes`
- [ ] 1.2 The `--help` comments of `ps`, `token` and `state-formats` in `crates/tilrd/src/main.rs` say "kept for the Tilr app; see the daemon-lifecycle spec"; verify by review

## 2. desktop-mac (`tilrd-rename`, repository tilr-dev/desktop-mac)

- [ ] 2.1 Commit the prepared rename on `tilrd-rename`; verify the app's own tests (`scripts/test_server_installation.swift`, `test_server_release.py`, `test_server_updates.swift`) pass
- [ ] 2.2 `bundle_server.py` / `release_server.py` / `verify_release.py` take `tilrd` from `tilr-vX.Y.Z-<target>.tar.gz` (or build `-p tilrd`); verify a bundle built from a GitHub release runs `tilrd --version`
- [ ] 2.3 Drop the `state-formats` equality check (`ServerInstallation.swift:216-219`); after the SIGUSR1 tell a refused reload by `ps --server-version`; verify with an app-side test updating from a server before `4298b15` (reload refused, terminals kept, the app says so) and from one after (reloaded)
- [ ] 2.4 Attach to a tilrd already serving the port; verify by starting `tilr` before the app and opening the app
- [ ] 2.5 The `server` plugin speaks protocol v3 (after `protocol-v3`); verify the app connects to a `dev` tilrd

## 3. Release

- [ ] 3.1 On the owner's word "в main": tilrd `dev` → `main` with desktop-mac `tilrd-rename`; the app's feed recreated for `tilrd` with `--min-app-build` = the first app build with 2.1-2.5; verify `cdn.tilr.dev/server/macos/arm64/stable.json` names `tilrd` and an older app build is refused by the feed

## 4. Checks and archive

- [ ] 4.1 Before marking a change done, run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo test --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 .`; on macOS also the Linux run via `scripts/linux.sh`
- [ ] 4.2 No itests case is touched; `./itests/dt coverage` exits 0
- [ ] 4.3 Record the commits (this repository and desktop-mac) in proposal.md and archive the change
