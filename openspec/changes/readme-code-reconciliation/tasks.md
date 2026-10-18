# Tasks

## 1. tilr and a superseded connection

- [ ] 1.1 `conn.rs` keeps the close code of a closed connection; `attach.rs` ends the attach on 4000 with `[superseded by another connection of this client]`, exit 0, terminal restored; verify `client::an_attach_superseded_by_its_own_client_id_ends` (the test reads `tilr`'s client id from the server's `PaneInfo.viewed_by` and says `Hello` with it) and `client::attached_in_a_terminal_it_types_reconnects_after_a_reload_and_detaches` still green; USE_CASES.md entry
- [ ] 1.2 README "Disconnects" and the `client-liveness` spec Note on `tilr` reconnecting after 4000; verify by review

## 2. README

- [ ] 2.1 Fix each README line listed in the proposal; verify by `grep -n` for the old wording (`16 KiB`, `about a second`, `Linux only for now`, `sent anything else first`) returning nothing and by re-reading the matching spec Notes

## 3. USE_CASES

- [ ] 3.1 Fix USE_CASES.md:361-362 and add the three missing test entries; verify with a script listing every `fn` with `#[tokio::test]`/`#[test]` in `crates/*/tests/*.rs` absent from USE_CASES.md (empty output)
- [ ] 3.2 itests/USE_CASES.md UC-082: a 65535x65535 `Resize` is clamped, the pane lives; verify by review

## 4. Checks and archive

- [ ] 4.1 Before marking a change done, run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo test --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 .`; on macOS also the Linux run via `scripts/linux.sh`; verify all green
- [ ] 4.2 No itests case is touched; `./itests/dt coverage` exits 0
- [ ] 4.3 Remove the Notes bullets of the specs (`protocol`, `client-liveness`, `clipboard-host-events`, `panes`, `access-control`, `metrics`, `reload`, `tilr-cli`) for the items fixed; verify `openspec validate --specs --strict`
- [ ] 4.4 `version.txt`: check what release-please's `simple` strategy writes (its source or docs for the version release-please-action pins in `.github/workflows/release.yml`); if it writes `version.txt`, drop the `release-process` Note, else drop the AGENTS.md line; verify by citing the source line in the commit message
- [ ] 4.5 Record the implementing commit in proposal.md and archive the change; verify `openspec list` no longer shows it
