# Tasks

## 1. Protocol and OSC 52

- [x] 1.1 `ClipboardWrite` in `tilr.proto` (`ServerMessage` variant), the `"clipboard-write"` client feature and the `"clipboard"` server feature comment (D4); README "Protocol" lists them; verify `cargo test -p tilr-proto` and review of README
- [x] 1.2 Register `on_clipboard_write` in `tilr-screen` (standard location only), send it over the link (`CLIPBOARD` kind) with the limit check (D1); verify with unit test `ghostty::tests::osc_52_is_reported_as_a_clipboard_write`
- [x] 1.3 Supervisor: publish to the store with the TTL, fan out to subscribed clients with `"clipboard-write"` in chunks on the bulk lane, one transfer per client (D1, D2); verify with `clipboard::osc_52_in_a_pane_reaches_the_watching_client`, `clipboard::a_large_copy_comes_in_chunks_and_a_newer_one_replaces_it`, `clipboard::nothing_is_copied_to_clients_with_the_clipboard_turned_off`; USE_CASES.md entries

## 2. Shims

- [x] 2.1 `xclip -selection clipboard -i` and a new `wl-copy` shim (parse, limit, publish, host socket `clipboard-write`) (D3); `TILR_CLIPBOARD_MAX` in the pane environment; verify with unit tests `clipboard::tests::xclip_copy_requests`, `clipboard::tests::wl_copy_requests` and `clipboard::xclip_in_a_pane_reaches_the_watching_client`, `clipboard::a_copy_over_the_limit_is_refused`
- [x] 2.2 A copy no client takes stays in the store only: the shim exits 0 without running the real tool, and a client subscribing later gets nothing; verify with `clipboard::a_copy_nobody_takes_stays_in_the_shared_buffer`
- [x] 2.3 Update `clipboard::the_clipboard_can_be_turned_off` for `wl-copy`; README "Pasting images: the shared clipboard" gets "Copying from a pane"; verify the test and review of README

## 3. tilr

- [x] 3.1 `tilr` lists `"clipboard-write"` and writes text transfers as OSC 52 (D5); verify with `attach::tests::copied_text_goes_to_the_terminal_as_osc_52`; README "The client: tilr" says so

## 4. itests

- [x] 4.1 UC-07D in itests/USE_CASES.md and `itests/scenarios/uc_07d.py::a_copy_in_vim_reaches_the_clients_clipboard` (vim `"+y` with OSC 52 via `:set clipboard`, or `printf` OSC 52, and `xclip -i` of an image; driver lists `"clipboard-write"` and records transfers), tagged `clipboard-host-events/a-copy-in-a-pane-reaches-the-clients`; verify `./itests/dt UC-07D` green

## 5. Checks and archive

- [x] 5.1 Run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo test --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 .`; on macOS also the Linux run via `scripts/linux.sh`; verify all green
- [x] 5.2 `./itests/dt UC-075 UC-07D` green, then `./itests/dt coverage` exits 0
- [x] 5.3 Record the commit in proposal.md ("Implemented by") and archive the change; verify `openspec validate --all --strict`
