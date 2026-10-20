# Tasks

## 1. Protocol and screen model

- [x] 1.1 `Frame.links` (`repeated string`) and `Run.link` (`uint32`) in `tilr.proto`, with comments (D1, D3); README "Protocol" describes them; verify `cargo test -p tilr-proto` and review of README
- [x] 1.2 Run building reads URIs only in rows with links and splits runs on link changes (D2, D3); verify with unit tests `ghostty::tests::a_hyperlink_goes_with_its_run` and `ghostty::tests::an_overlong_hyperlink_is_left_out`, and the existing `ghostty::tests::styles_are_split_into_runs` unchanged
- [x] 1.3 After a reload links are plain text, new ones work; verify with `reload::hyperlinks_are_plain_text_after_a_reload_and_new_ones_work`; README "Reloading without closing shells" states the limit; USE_CASES.md lists the tests

## 2. tilr attach

- [ ] 2.1 `draw` wraps linked runs in OSC 8 (D5); verify with a unit test `attach::tests::a_linked_run_is_drawn_inside_osc_8` and by hand (`printf '\e]8;;https://example.com\e\\x\e]8;;\e\\'` in `tilr` inside a terminal that supports OSC 8)

## 3. Checks and archive

- [ ] 3.1 Run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo test --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 .`; on macOS also the Linux run via `scripts/linux.sh`; verify all green
- [x] 3.2 `./itests/dt --only scenarios --filter uc_079` and `./itests/dt --only scenarios --filter uc_06a` (input and reload with frames) green, then `./itests/dt coverage` exits 0
- [ ] 3.3 Record the commit in proposal.md ("Implemented by") and archive the change; verify `openspec validate --all --strict`

## Verification status — 2026-10-20

- Passed: protocol (9 tests), screen (38 tests), attach OSC 8 unit test, standalone hyperlink reload integration test; fmt, clippy with and without failpoints, Rust 1.90 check, cargo deny, gitleaks; UC-079 (7/7 including fixtures), UC-06A (3/3 including fixtures), coverage exit 0; OpenSpec strict (53/53).
- Pending: manual OSC 8 link affordance in a supporting terminal (computer-use access to WezTerm was denied; awaiting Dan).
- Blocked: native workspace run timed out in the existing command-line rejection reload test's successor `state-formats`, then aborted on a poisoned test log mutex (TILR-170). Linux workspace run timed out in the existing file-watch test (TILR-166). Root causes remain unproven; no blind reruns or timeout increases.
- Archive remains pending until the required checks and manual verification are complete. Rebased against freshly fetched `origin/dev` (6d32977); implementation commit a585eca.
- On 2026-10-20 OpenSpec requires the installed Node 26, while the default PATH picks Node 18: recheck with `node --version` and `/opt/homebrew/bin/node --version`; use `PATH=/opt/homebrew/bin:$PATH openspec validate --all --strict`. Remove the PATH override once the default Node supports the installed OpenSpec.
