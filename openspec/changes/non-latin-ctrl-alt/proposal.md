# Proposal: Ctrl and Alt with non-Latin and composed characters

Блокер: Which rule should Ctrl/Alt with a non-ASCII character follow? Options: (a) keep today's rule (always the US key) — simple, breaks Option+c / AltGr; (b) remap only when Ctrl is held, Alt alone sends the character, AltGr never remaps; (c) remap only when the client says the layout is non-Latin (`KeyInput` gets a flag). Recommendation: (b), with a client-reported AltGr bit as a later additive field.

Зависит от: protocol-feature-flags

## Why

The code review of branch `itests` (79397e2..13f9522, 2026-10-18) (finding #14): with Ctrl or Alt and a non-ASCII character the server always encodes the physical key's US character (`tilr-screen/src/ghostty.rs:480-491`). That fixed Ctrl+C on a Russian layout (UC-079) but breaks Option+c giving `ç` on macOS, AltGr+e giving `€`, Polish `ą` (AltGr) and loses Shift on Alt+Shift punctuation.

## What Changes

- To be decided; candidate rule: the physical-key remap applies only to Ctrl (and Ctrl+Alt) combinations; Alt alone with a non-ASCII character sends the character (ESC-prefixed when the terminal's meta mode asks for it); AltGr (reported by the client) never remaps; Shift is kept with Alt.
- Clients may need to report AltGr / Option-as-Meta explicitly in `KeyInput`.

## Capabilities

### Modified Capabilities
- `screen-frames`

Spec deltas are not written yet (`skip_specs: true`).

## Out of scope

- Client-side keyboard handling in the Tilr apps.

## Impact

`crates/tilr-screen/src/ghostty.rs`; possibly a `KeyInput` field (additive, feature flag per `protocol-feature-flags`). Nothing crosses a reload.
