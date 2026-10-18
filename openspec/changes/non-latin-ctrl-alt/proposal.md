# Proposal: Ctrl and Alt with non-Latin and composed characters

Зависит от: protocol-v3

## Why

The code review of branch `itests` (79397e2..13f9522, 2026-10-18) (finding #14): with Ctrl or Alt and a non-ASCII character the server always encodes the physical key's US character. That fixed Ctrl+C on a Russian layout (UC-079) but breaks Option+c giving `ç` on macOS when the client reports it with Alt, AltGr characters a client reports with Alt, and Alt on non-Latin layouts (Alt+ж gives `ESC ;`, not `ESC ж`).

Still so at 1163abd: `crates/tilr-screen/src/ghostty.rs:479-491` remaps any key with Ctrl or Alt whose character is not ASCII to its US-layout character (`key = physical; unshifted = Some(us)`, text `us` with Shift as ASCII upper case); the unit test `ghostty::tests::ctrl_and_alt_on_a_layout_that_is_not_latin_go_by_the_physical_key` (`ghostty.rs:1879-1930`) and UC-079 (`itests/scenarios/uc_079.py:165-200`, `("ru", "alt-ф"): b"\x1ba"`) pin Alt+Ф = `ESC a`; the proto comment on `KeyInput.code` says the same (`tilr.proto:195-211`).

## Decision (2026-10-18)

- Follow Unix terminal conventions (xterm, kitty, ghostty). Ctrl + a key whose character has no C0 control (non-Latin layouts) uses the physical key (`KeyInput.code`, the base layout key): Ctrl+с on a Russian layout is ^C.
- Alt + character sends ESC then the character produced, never remapped: Alt+ж is `ESC ж`; Alt+Shift keeps the shifted character.
- AltGr and macOS Option composing produce the character itself and are not Alt. Whether Option acts as Alt is the client's setting (like ghostty's `macos-option-as-alt`); the server sees Alt only when the client sends it.
- Rejected: always the US key for Ctrl and Alt (today; breaks Alt on non-Latin layouts and composed characters).
- Rejected: a client-reported layout or AltGr flag in `KeyInput` (the client already sends the composed character without Alt).

## What Changes

- Ctrl with a non-ASCII character keeps going by the physical key (unchanged).
- Alt without Ctrl sends `ESC` + the character (`text`, else `codepoint`), Shift included; no physical-key remap. Ctrl+Alt sends `ESC` + the Ctrl code of the physical key. Alt with an empty `code` follows the same rule (`ESC` + the character), where today it sends the text alone.
- A character sent without Alt (AltGr, Option composing) is sent as itself (already so; pinned by tests).
- The kitty keyboard protocol keeps reporting the physical key as the base layout key (unchanged).
- The requirement "Ctrl and Alt go by the physical key on a non-Latin layout" is renamed to "Ctrl goes by the physical key on a non-Latin layout"; UC-079's tags and its Alt expectation (`ESC a` → `ESC ф`) change with it.
- The proto comment on `KeyInput.code` describes the new Alt rule (a comment only; no field changes).

## Capabilities

### Modified Capabilities
- `screen-frames`: see `specs/screen-frames/spec.md`.

## Non-goals

- Client-side keyboard handling in the Tilr apps, including the Option-as-Alt setting.
- Any `KeyInput` field: no protocol change, no PROTOCOL_VERSION bump, no feature. It lands after `protocol-v3` only because both edit the `KeyInput.code` comment and the same unit tests.
- Keys `tilr` sends (it reads characters from its own terminal, `code` empty).

## Impact

`crates/tilr-screen/src/ghostty.rs` and its unit tests; `crates/tilr-proto/proto/tilr.proto` (`KeyInput.code` comment); `crates/tilr-driver/src/keys.rs` only if its `alt-` keys need the produced character; itests UC-079 (`uc_079.py`, its tags) and `itests/USE_CASES.md` UC-079; README "Keys"; USE_CASES.md. Not breaking on the wire. A program that relied on Alt+Ф giving `ESC a` now gets `ESC ф` (as in xterm). Reload: nothing crosses a reload (encoding is per key; the pane state format is unchanged); a pane process takes the new encoding when it execs into the new executable. Update from the previous release: no effect beyond the new encoding.
