# Proposal: Copy from a pane to the client's clipboard

## Why

The clipboard goes one way only: `xclip -i` / `wl-copy` from a remote program do not reach the user's machine. Also open: a fallback that pastes the file path when nobody read the clipboard file in about 300 ms, programs that use X11 or Wayland directly (an Xvfb option), and a remote macOS (Basic Memory `main/projects/tilr/tilr-vstavka-kartinok-cherez-udalionnyi-bufer-v-pane`).

## What Changes

- To be designed: the server's `xclip -i` / `wl-copy` shims send a `ClipboardChanged` (mime, data in chunks on the bulk lane) to clients watching the pane with a `clipboard-out` feature.

## Capabilities

### Modified Capabilities
- `clipboard-host-events`

Spec deltas are not written yet (`skip_specs: true`).

## Out of scope

- Direct X11/Wayland programs.

## Impact

Additive proto messages and feature flag; data on the bulk lane, bounded like `FsData`. Nothing crosses a reload beyond the clipboard file.
