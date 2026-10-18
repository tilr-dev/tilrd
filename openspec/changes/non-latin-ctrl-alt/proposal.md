# Proposal: Ctrl and Alt with non-Latin and composed characters

## Why

The code review of branch `itests` (79397e2..13f9522, 2026-10-18) (finding #14): with Ctrl or Alt and a non-ASCII character the server always encodes the physical key's US character (`tilr-screen/src/ghostty.rs:480-491`). That fixed Ctrl+C on a Russian layout (UC-079) but breaks Option+c giving `ç` on macOS, AltGr+e giving `€`, Polish `ą` (AltGr) and loses Shift on Alt+Shift punctuation.

## Decision (2026-10-18)

- Follow Unix terminal conventions (xterm, kitty, ghostty). Ctrl + a key whose character has no C0 control (non-Latin layouts) uses the physical key (`KeyInput.code`, the base layout key): Ctrl+с on a Russian layout is ^C.
- Alt + character sends ESC then the character produced, never remapped: Alt+ж is `ESC ж`; Alt+Shift keeps the shifted character.
- AltGr and macOS Option composing produce the character itself and are not Alt. Whether Option acts as Alt is the client's setting (like ghostty's `macos-option-as-alt`); the server sees Alt only when the client sends it.
- Rejected: always the US key for Ctrl and Alt (today; breaks Alt on non-Latin layouts and composed characters).
- Rejected: a client-reported layout or AltGr flag in `KeyInput` (the client already sends the composed character without Alt).

## What Changes

- Ctrl with a non-ASCII character keeps going by the physical key (unchanged).
- Alt without Ctrl sends `ESC` + the character (`codepoint`, else `text`), Shift included; no physical-key remap. Ctrl+Alt sends `ESC` + the Ctrl code of the physical key.
- A character sent without Alt (AltGr, Option composing) is sent as itself.
- The kitty keyboard protocol keeps reporting the physical key as the base layout key (unchanged).
- The requirement "Ctrl and Alt go by the physical key on a non-Latin layout" is renamed to "Ctrl goes by the physical key on a non-Latin layout"; UC-079's tags and its Alt expectation (`ESC a` → `ESC ф`) change with it.

## Capabilities

### Modified Capabilities
- `screen-frames`: see `specs/screen-frames/spec.md`.

## Non-goals

- Client-side keyboard handling in the Tilr apps, including the Option-as-Alt setting.
- Any `KeyInput` field: no protocol change, no PROTOCOL_VERSION bump, no feature.

## Impact

`crates/tilr-screen/src/ghostty.rs` and its unit tests; itests UC-079. Not breaking on the wire. A program that relied on Alt+Ф giving `ESC a` now gets `ESC ф` (as in xterm). Nothing crosses a reload.
