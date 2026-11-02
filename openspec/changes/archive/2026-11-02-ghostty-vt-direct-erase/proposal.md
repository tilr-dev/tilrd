# Direct Ghostty VT screen erase

## Why
Injecting erase escape sequences into the program parser cancels incomplete program sequences. A host screen operation must preserve that parser and terminal modes.

## What Changes
- Vendor the Rust bindings and a pinned Ghostty patch adding a direct erase operation.
- Expose erase, absolute cursor home and optional history deletion through tilr-screen.
- Apply the same patch in Cargo fallback and shared package builds; identify patched packages and cache inputs.

## Impact
Terminal screen API, native build inputs, shared package discovery and deterministic VT regression coverage.
