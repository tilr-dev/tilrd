# Proposal: Report and carry fixes for libghostty-vt

## Why

Upstream issues to report: `encode_to_vec` OutOfSpace (worked around since 2026-10-03) and `RustPngDecoder` without a constructor (worked around in befaf82); vendoring Ghostty was planned for the first needed patch (grapheme clusters / mode 2027 reset by `ESC c`) (Basic Memory `main/projects/tilr/tilr-zhurnal-reshenii`).

## What Changes

- Report both issues upstream; decide vendoring when the first patch is needed.

## Capabilities

### Modified Capabilities
- `screen-frames`

No spec delta: tests, docs or process only; `skip_specs: true`.

## Out of scope

- Nothing beyond What Changes.

## Impact

`crates/tilr-screen`, `Cargo.toml` (a patched source if vendored; `cargo deny`). No protocol change.
