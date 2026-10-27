# Proposal: Shared clipboard for image paste (xclip / wl-paste shims)

> Retro archive entry, reconstructed on 2026-10-18 from git history, Basic Memory notes and itests/USE_CASES.md (branch `itests`). Spec deltas are not kept; the resulting behavior is in `openspec/specs/`.

## Why

Claude Code running on a remote host reads a pasted image through `xclip` / `wl-paste` (found by reading the Claude Code 2.1.289 binary); an image copied on the user's machine had no way to reach it (Basic Memory `main/projects/tilr/tilr-vstavka-kartinok-cherez-udalionnyi-bufer-v-pane`).

## What Changes

- `ClipboardPut{transfer_id, mime, total_size, data, last}` uploads in chunks, answered by `ClipboardAck`; `HelloAck.clipboard_max_bytes` announces the limit.
- One shared clipboard file per user in the runtime directory; `xclip` and `wl-paste` in `<runtime>/bin` are links to the server, first on the panes' PATH; other requests go to the real tools.
- `--clipboard-max` (20 MB at the time), `--clipboard-ttl` (300 s), `--no-clipboard`; an oversize upload is refused on its first chunk.
- The client sends Ctrl+V as a `KeyInput` after the last chunk.
- Protocol: unknown messages are ignored instead of answered with an error; `Hello`/`HelloAck` carry a feature list.

## Capabilities

### Modified Capabilities
- `clipboard-host-events`
- `protocol`

## Out of scope

- The reverse direction (`xclip -i` / `wl-copy` to the client; open change `reverse-clipboard`).
- Programs that talk to X11 or Wayland directly.

## Impact

Implemented by: [`58dcaea`](https://github.com/tilr-dev/tilrd/commit/58dcaea) Shared clipboard for image paste; supervisor reloads itself (2026-10-04). The same commit also holds the supervisor reload (`2026-10-04-supervisor-reload`). Verified with Claude's literal commands in tests, not in a real Claude Code on remote Linux.
