# Tasks

## 1. Encoding

- [ ] 1.1 In `ghostty.rs`, remap to the physical key only with Ctrl; Alt without Ctrl sends `ESC` + the produced character (Shift kept); Ctrl+Alt sends `ESC` + the Ctrl code; verify with unit tests in `crates/tilr-screen` for Ctrl+с (ru), Alt+ж, Alt+Shift+Ф, Alt+Shift+1, Ctrl+Alt+с, and the existing kitty tests unchanged
- [ ] 1.2 Composed characters without Alt pass as themselves; verify with unit tests for `€`, `ą`, `ç` without Alt and Option+c sent as Alt
- [ ] 1.3 itests UC-079: Alt+ф expects `ESC ф`, add Alt+Shift+1 and `ç` without Alt; retag `screen-frames/ctrl-and-alt-go-by-the-physical-key-on-a-non-latin-layout` to `screen-frames/ctrl-goes-by-the-physical-key-on-a-non-latin-layout` and add the two new requirement tags; verify `./itests/dt` UC-079 green and `./itests/dt coverage` exits 0
- [ ] 1.4 README key input section and USE_CASES.md describe the rule (Ctrl by physical key, Alt = ESC + character, Option-as-Alt is the client's); verify by review against this proposal

## 2. Checks

- [ ] 2.1 Run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo test --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 .`; on macOS also `scripts/linux.sh`; verify all green
