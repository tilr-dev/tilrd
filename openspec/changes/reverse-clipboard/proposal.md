# Proposal: Copy from a pane to the client's clipboard

## Why

The clipboard goes one way only: `xclip -i` / `wl-copy` from a remote program do not reach the user's machine. Also open: a fallback that pastes the file path when nobody read the clipboard file in about 300 ms, programs that use X11 or Wayland directly (an Xvfb option), and a remote macOS (Basic Memory `main/projects/tilr/tilr-vstavka-kartinok-cherez-udalionnyi-bufer-v-pane`).

Checked at 1163abd: still one way. The server's `xclip` hands copying (`-i`, a file argument) to the real tool (`crates/tilrd/src/clipboard.rs:389-416`, `Request::Other` → `exec_next` at `clipboard.rs:522-539`); there is no `wl-copy` shim (the shims are `xclip`, `wl-paste`, `tilr`). OSC 52 (how vim, neovim, tmux and most TUI programs copy over ssh) is dropped: libghostty-vt 0.2.2 offers `Terminal::on_clipboard_write` (OSC 52 and OSC 1337 Copy, decoded), and `crates/tilr-screen` does not register it. `tilr.proto` has no server-to-client clipboard message.

## What Changes

- New push `ClipboardWrite { transfer_id, pane_id, mime, total_size, data, last }` in chunks of at most 256 KiB on the bulk lane, sent to every client subscribed to the pane that lists `"clipboard-write"` in `Hello.features`.
- Sources: OSC 52 / OSC 1337 Copy to the standard clipboard in a pane's output (`text/plain;charset=utf-8`), and the server's `xclip -selection clipboard -i [-t <mime>]` and a new `wl-copy [-t <mime>] [TEXT...]` shim in panes.
- A copy also becomes the server's shared clipboard (what `xclip -o` / `wl-paste` in any pane then read), with `--clipboard-ttl` as for uploads; content over `--clipboard-max` is refused (the shim exits non-zero with the limit; an OSC 52 write is dropped with a log line).
- A copy that reaches no client with `"clipboard-write"` stays only in the server's shared clipboard until `--clipboard-ttl`, as in tmux: it is not handed to the real `xclip` / `wl-copy` and not sent to a client that attaches later; the shim succeeds since the server clipboard holds it (Dan, 2026-10-20).
- `tilr attach` lists `"clipboard-write"` and writes text it receives to the local terminal as OSC 52, so the user's own terminal applies its clipboard policy.
- No new `HelloAck.features` entry and no PROTOCOL_VERSION bump (protocol-v3 rule): the push is part of the `"clipboard"` capability, which `--no-clipboard` turns off along with every shim; a client that does not list `"clipboard-write"` gets nothing new.

## Capabilities

### Modified Capabilities
- `clipboard-host-events`: see `specs/clipboard-host-events/spec.md` (shim list and real-tool hand-over modified; copying requirements added).

## Non-goals

- Programs that talk to X11 or Wayland directly (an Xvfb or headless compositor option).
- A remote macOS (`pbcopy` shim).
- The primary and secondary selections (`xclip -selection primary`, OSC 52 `p`/`s`): still the real tool's, or dropped.
- OSC 52 reads (`?`): never answered (libghostty ignores them), so a pane cannot read the user's clipboard.
- Pasting a file path when nobody read the clipboard file within 300 ms (a separate idea from the same note; no design recorded).
- Whether a client asks the user before taking a write: the client's policy (the server only carries it).

## Impact

`crates/tilr-proto/proto/tilr.proto` (`ClipboardWrite`, additive; `"clipboard-write"` client feature in comments), `crates/tilr-screen/src/ghostty.rs` (`on_clipboard_write`), `crates/tilrd/src/{pane,supervisor,clipboard,host,main}.rs` (link message, fan-out on the bulk lane, `wl-copy` shim, `xclip -i`, host socket request `clipboard-write`), `crates/tilr/src/{conn,attach}.rs`, README "Pasting images: the shared clipboard" and "Protocol", USE_CASES.md, itests UC-07D. Reload: nothing new crosses it; a transfer in flight to a client ends with its connection (the content stays in the server clipboard file, which already survives a reload). Update from the previous release: works; the shims are re-linked at start as today.

Not breaking: additive messages and fields only, no PROTOCOL_VERSION bump, no close code, state file or CDN change.
