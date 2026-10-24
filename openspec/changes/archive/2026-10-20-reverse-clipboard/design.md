# Design

## Context

At 1163abd: the shared clipboard is a file in `<runtime>/clipboard` (`Store`, `crates/tilrd/src/clipboard.rs:113-163`) written by client uploads (`ClipboardPut`) and read by the `xclip` / `wl-paste` shims in panes; copying in a pane goes to the real tool or fails. The host socket (`crates/tilrd/src/host.rs`) takes JSON lines from programs in panes (`tilr edit`), checks the peer uid, and the supervisor fans `HostEvent`s out to subscribed clients with `"host-events"`. Pane processes talk to the supervisor over a tagged link (`crates/tilrd/src/link.rs`). libghostty-vt 0.2.2 decodes OSC 52 and OSC 1337 Copy into `on_clipboard_write(location, contents[mime, data])`.

## Decisions

### D1. One push for both sources, sent by the supervisor
A pane's OSC 52 write goes from the screen model's callback to the pane process, which sends it to the supervisor over the link (new link kind `CLIPBOARD`, payload `mime` + data, only for the standard location, every copy up to what the link carries: the supervisor checks the clipboard is on and `--clipboard-max`, so a pane process started by an older server, without the limit in its environment, works after a reload too). A shim copy goes to the supervisor over the host socket (D3). The supervisor publishes the content to the `Store` with its TTL, then sends `ClipboardWrite` chunks to each client subscribed to that pane with `"clipboard-write"` in `Hello.features`. Rejected: the pane process writing the store itself (the TTL and the limit live in the supervisor; one writer keeps last-writer-wins simple).

### D2. Chunks on the bulk lane, one transfer per client at a time
`ClipboardWrite` chunks are at most 256 KiB, on the bulk lane, waiting for room as `FsData` does. The supervisor keeps the content once (shared `Bytes`) while any client is still being sent it. A new write for a client that is still being sent an older one ends the older one before it starts (no `last` chunk; the client drops a transfer whose `transfer_id` changes before `last`, and a chunk of a transfer older than its newest, which a chunk under way on another thread can make; ids only grow, also across a reload, through the supervisor state), so a program copying in a loop cannot queue up content: per client at most one transfer is in flight, bounded by `--clipboard-max`.

### D3. Shim to server: through the store, announced on the host socket
`xclip -selection clipboard -i [-t <mime>] [FILE]` and `wl-copy [-t <mime>] [TEXT...]` read their content (stdin, the file, or the arguments joined by spaces; `-t` default `text/plain;charset=utf-8`), refuse it over the limit (`TILR_CLIPBOARD_MAX` in the pane environment, set with the shims), publish it to the store with the TTL (`TILR_CLIPBOARD_TTL`; temp file, rename, as uploads), then send `{"name":"clipboard-write","pane_id":"$TILR_PANE","copy_id":<inode of the stored file>}` on `$TILR_SOCK`. The server re-reads the store and, if it still holds that copy (not an upload or another copy made meanwhile), fans it out (D1, D2) and answers `{"ok":true,"clients":N}`. With N = 0 the shim exits 0 too: the content stays in the store only, never piped to a real `xclip`/`wl-copy` (Dan, 2026-10-20, as in tmux; a pipe to the real tool was dropped). Rejected: base64 content in the JSON line (the host socket caps a connection at 1 MiB, and images are larger); a temp file per copy (the store already is one).

### D4. Feature naming under the protocol-v3 rule
Server side, the push belongs to `"clipboard"` (`--no-clipboard` turns off both directions and removes the shims, including `wl-copy`); no new `HelloAck.features` entry. Client side, `"clipboard-write"` in `Hello.features` says the client takes writes, as `"host-events"` does for `HostEvent`. A client not listing it gets no `ClipboardWrite`.

### D5. tilr attach
`tilr` lists `"clipboard-write"` and, for a completed transfer whose MIME type is text, writes `ESC ] 52 ; c ; <base64> ESC \` to its terminal; other types are ignored. The user's terminal decides whether to allow it (as it does for ssh sessions).

## Reload and reconnect

Nothing new crosses a reload: the content is in the store file, which survives as today; a transfer in flight ends with the connection when the supervisor execs; OSC 52 output read by the old pane process before the exec has been sent, and output after it is parsed by the successor. A client that reconnects gets no replay of earlier writes (a paste on its side uses its own clipboard). A shim copy during the exec gets no answer on the host socket and exits non-zero (`cannot reach the tilr server`), as `tilr edit` does today.

## Backpressure

Bulk lane, chunks wait for room (bounded by the bulk window); at most one transfer per client in flight (D2) and one copy of the content in the supervisor; OSC 52 data held by the pane process only until it is written to the link.
