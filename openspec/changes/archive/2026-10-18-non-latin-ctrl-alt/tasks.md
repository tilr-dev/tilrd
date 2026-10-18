# Tasks

## 1. Encoding

- [x] 1.1 In `crates/tilr-screen/src/ghostty.rs` (`encode_key`, the non-ASCII branch at :479-491 at 1163abd), remap to the physical key only with Ctrl; verify with the unit test `ghostty::tests::ctrl_on_a_layout_that_is_not_latin_goes_by_the_physical_key` (renamed from `ctrl_and_alt_on_a_layout_that_is_not_latin_go_by_the_physical_key`, Alt assertions moved out: Ctrl+с, Ctrl+я, Ctrl+ъ, Ctrl+ψ, typing) (the empty-`code` Ctrl assertions were already in `a_key_without_a_physical_key_is_encoded_as_on_a_us_layout` since protocol-v3; the renamed test keeps Ctrl+с, Ctrl+я, Ctrl+ъ, Ctrl+ψ, Ctrl+Alt+5 and typing)
- [x] 1.2 Alt without Ctrl sends `ESC` + the produced character (Shift kept, with or without `code`); Ctrl+Alt sends `ESC` + the Ctrl code; verify with the unit test `ghostty::tests::alt_sends_esc_and_the_character_produced` (Alt+ж, Alt+Shift+Ф, Alt+Shift+1 → `ESC !`, Ctrl+Alt+с → `ESC 0x03`, Alt+ж without code) and `ghostty::tests::the_kitty_protocol_gets_the_physical_key_as_the_base_layout_key` unchanged (also Alt+Shift+Ф without text; the encoder puts ESC before a one-byte character only, so the server adds it when the encoder sent the text alone and DEC mode 1036 is on; with 1036 off the text alone, checked in the test)
- [x] 1.3 Composed characters without Alt pass as themselves; verify with the unit test `ghostty::tests::composed_characters_are_sent_as_themselves` (`€`, `ą`, `ç` without Alt; Option+c sent as Alt with `c` → `ESC c`) (also Alt+ç is ESC and ç in UTF-8, not the raw byte 0xE7 the encoder would prefix from the unshifted codepoint)
- [x] 1.4 `tilr.proto` `KeyInput.code` comment: Ctrl goes by the physical key, Alt is `ESC` + the character produced; verify by review against this proposal
- [x] 1.5 README "Keys" and USE_CASES.md describe the rule (Ctrl by physical key, Alt = ESC + character, Option-as-Alt is the client's); verify by review against this proposal (README "Keys"; USE_CASES.md the keys entry)

## 2. itests

- [x] 2.1 UC-079 (`uc_079.py::ctrl_c_on_a_russian_layout_is_ctrl_c`): `("ru", "alt-ф")` expects `ESC ф`; add Alt+Shift+1 → `ESC !` and `ç` without Alt; teach `crates/tilr-driver/src/keys.rs` the keys if needed (`cargo test -p tilr-driver`); retag `screen-frames/ctrl-and-alt-go-by-the-physical-key-on-a-non-latin-layout` to `screen-frames/ctrl-goes-by-the-physical-key-on-a-non-latin-layout` and add `screen-frames/alt-sends-esc-and-the-character-produced` and `screen-frames/composed-characters-are-sent-as-themselves`; update `itests/USE_CASES.md` UC-079; verify `./itests/dt` UC-079 green (also Alt+Shift+Ф, Ctrl+Alt+с, Option as Alt `alt-c`; `ç@KeyC` on `us` for Option+c composing; tilr-driver learned shifted US punctuation, `alt-shift-!` is Digit1 with text `!`, unit test in `keys::tests`)

## 3. Checks and archive

- [x] 3.1 Before marking a change done, run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo test --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 .`; on macOS also the Linux run via `scripts/linux.sh`; verify all green (macOS and `scripts/linux.sh` cargo test green)
- [x] 3.2 `./itests/dt` for UC-079 green, then `./itests/dt coverage` exits 0 (it fails on a tag naming no requirement) (UC-079 5/5; full run 493/494: every scenario green, rust `reconnect::a_lagging_client_gets_the_last_screen_without_new_output` fails in Dagger as at the base, client 2 dropped as silent after 30 s, `connection closed` at tests/common/mod.rs:367; not this change, TILR-112)
- [x] 3.3 Record the implementing commit in proposal.md and archive the change; verify `openspec list` no longer shows it
