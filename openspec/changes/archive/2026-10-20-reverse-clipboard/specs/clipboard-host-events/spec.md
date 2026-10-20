## MODIFIED Requirements

### Requirement: The clipboard can be turned off
With `--no-clipboard` the `"clipboard"` feature MUST be absent and `clipboard_max_bytes` 0; a `ClipboardPut` transfer MUST be answered once, on its first chunk, with `ClipboardAck` `ok` false and `the clipboard is turned off on this server`; the panes MUST get neither the server's `xclip`/`wl-paste`/`wl-copy` nor `TILR_CLIPBOARD_DIR`, `TILR_CLIPBOARD_MAX` or `TILR_CLIPBOARD_TTL` (its `tilr` stays); and no `ClipboardWrite` MUST be sent.

**Verified by**: `clipboard::the_clipboard_can_be_turned_off` and `clipboard::nothing_is_copied_to_clients_with_the_clipboard_turned_off`.

#### Scenario: Turned off
- **WHEN** the server runs with `--no-clipboard` and a client uploads an image
- **THEN** the `ClipboardAck` has `ok` false and an error containing `turned off`
- **AND** in a pane `command -v xclip wl-paste wl-copy` finds nothing and `$TILR_CLIPBOARD_DIR` is empty

#### Scenario: OSC 52 while turned off
- **WHEN** the server runs with `--no-clipboard` and a pane prints an OSC 52 write
- **THEN** a subscribed client with `"clipboard-write"` receives no `ClipboardWrite`

### Requirement: The panes get the server's xclip and wl-paste first on PATH
With the clipboard on, the server SHALL link its executable as `xclip`, `wl-paste` and `wl-copy` (and `tilr`) in `bin/` of the runtime directory, put that directory first on every pane's `PATH`, and set `TILR_SHIMS` to it, `TILR_CLIPBOARD_DIR` to the clipboard directory, `TILR_CLIPBOARD_MAX` to `--clipboard-max` in bytes and, with a `--clipboard-ttl`, `TILR_CLIPBOARD_TTL` to it in seconds, so a profile that resets `PATH` can restore it with `PATH="$TILR_SHIMS:$PATH"`.

**Verified by**: `clipboard::a_pasted_image_reaches_a_program_that_reads_the_clipboard` and `clipboard::the_clipboard_can_be_turned_off`.

#### Scenario: Claude Code's commands
- **WHEN** a PNG is on the clipboard and a pane runs Claude Code's `xclip -selection clipboard -t TARGETS -o | grep image/png` and then `xclip -selection clipboard -t image/png -o > file`
- **THEN** the check succeeds and the file is the uploaded image byte for byte

#### Scenario: wl-copy is the server's
- **WHEN** a pane runs `command -v wl-copy`
- **THEN** it prints `$TILR_SHIMS/wl-copy`

### Requirement: Other requests go to the real tools
While the clipboard is empty, and for what it does not serve (the primary selection, `--watch`, `wl-paste` with a file argument), the server's `xclip`/`wl-paste`/`wl-copy` MUST hand over to the next tool of that name on `PATH` that is not itself, or fail with `the tilr clipboard is empty and no other <tool> is installed`. Copying to the clipboard selection is served by the server (see "A copy in a pane reaches the clients").

**Verified by**: `clipboard::a_pasted_image_reaches_a_program_that_reads_the_clipboard` (no real tool in the tests' `PATH`: the check fails before the first paste).

#### Scenario: Nothing pasted yet
- **WHEN** nothing was pasted and no other `xclip`/`wl-paste` is installed, and a pane runs Claude Code's image check
- **THEN** the check exits non-zero

#### Scenario: The primary selection
- **WHEN** a pane runs `echo hi | xclip -selection primary -i` and no other `xclip` is installed
- **THEN** it exits non-zero naming the missing tool and no `ClipboardWrite` is sent

## ADDED Requirements

### Requirement: A copy in a pane reaches the clients
When a program in a pane copies to the clipboard, by OSC 52 or OSC 1337 Copy (`text/plain;charset=utf-8`) or by the server's `xclip -selection clipboard -i [-t <mime>]` or `wl-copy [-t <mime>]`, the server SHALL make it the shared clipboard (with `--clipboard-ttl`) and SHALL send it as `ClipboardWrite` to every client subscribed to that pane that lists `"clipboard-write"` in `Hello.features`, and to no other client.

**Verified by**: `clipboard::osc_52_in_a_pane_reaches_the_watching_client`, `clipboard::xclip_in_a_pane_reaches_the_watching_client` and itests UC-07D (`uc_07d.py::a_copy_in_vim_reaches_the_clients_clipboard`).

#### Scenario: OSC 52 from a pane
- **WHEN** clients A (with `"clipboard-write"`) and B (without) watch a pane that prints `ESC ] 52 ; c ; aGk= BEL`
- **THEN** A receives a `ClipboardWrite` with that `pane_id`, `mime` `text/plain;charset=utf-8` and data `hi`, B receives nothing
- **AND** `xclip -selection clipboard -o` in another pane prints `hi`

#### Scenario: An image copied with xclip
- **WHEN** a pane runs `xclip -selection clipboard -t image/png -i < pic.png` while client A watches it
- **THEN** the shim exits 0, A receives the PNG byte for byte, and `wl-paste -t image/png` in another pane prints it

#### Scenario: Only the watching clients
- **WHEN** client C with `"clipboard-write"` watches another pane
- **THEN** C receives nothing

### Requirement: Copied content goes in chunks on the bulk lane
`ClipboardWrite` SHALL carry the content in chunks of at most 256 KiB of one `transfer_id`, `mime` and `total_size` in each, the last with `last`, on the client's bulk lane; a newer copy for a client still being sent an older one SHALL end the older transfer without a `last` chunk, and a client SHALL take only a transfer that ends with `last`.

**Verified by**: `clipboard::a_large_copy_comes_in_chunks_and_a_newer_one_replaces_it`.

#### Scenario: A 1 MiB copy
- **WHEN** a pane copies 1 MiB with `xclip -i`
- **THEN** the client receives 4 chunks of one `transfer_id`, the last with `last`, whose data is the content

#### Scenario: Copies in a loop
- **WHEN** a pane copies 100 times in a row while the client reads slowly
- **THEN** the server's memory stays bounded by one copy per client and the client ends with the last copy

### Requirement: Copies over the limit are refused
A shim copy over `--clipboard-max` SHALL fail with `<tool>: too large: <size>, the clipboard takes at most <limit>` and leave the clipboard as it was; an OSC 52 write over the limit SHALL be dropped with the log line `clipboard: a copy over the limit was dropped`.

**Verified by**: `clipboard::a_copy_over_the_limit_is_refused`.

#### Scenario: xclip over the limit
- **WHEN** the server runs with `--clipboard-max 1K` and a pane copies 2 KiB with `xclip -selection clipboard -i`
- **THEN** `xclip` exits non-zero with `too large`, no `ClipboardWrite` is sent, and the clipboard holds what it held before

### Requirement: A copy nobody takes stays in the shared buffer only
When a copy reaches no client with `"clipboard-write"`, the server SHALL keep it only in the shared clipboard (until `--clipboard-ttl`): a shim copy SHALL exit 0 without running the next real `xclip` / `wl-copy` on `PATH`, and a client that connects or subscribes to the pane later SHALL NOT be sent it.

**Verified by**: `clipboard::a_copy_nobody_takes_stays_in_the_shared_buffer`.

#### Scenario: A real xclip is installed
- **WHEN** no client watches the pane, a fake `xclip` later on `PATH` records whether it ran, and a pane runs `echo hi | xclip -selection clipboard -i`
- **THEN** the shim exits 0, the fake `xclip` did not run, and `xclip -selection clipboard -o` prints `hi`

#### Scenario: A client that comes later
- **WHEN** a copy reached no client and then a client with `"clipboard-write"` subscribes to the pane
- **THEN** that client receives no `ClipboardWrite` for it

### Requirement: tilr passes copied text to its terminal
`tilr` SHALL list `"clipboard-write"` in `Hello.features` and SHALL write a completed `ClipboardWrite` whose MIME type is text to its terminal as `ESC ] 52 ; c ; <base64> ESC \`; other types SHALL be ignored.

**Verified by**: unit test `attach::tests::copied_text_goes_to_the_terminal_as_osc_52` (crates/tilr/src/attach.rs).

#### Scenario: Text copied in the attached pane
- **WHEN** `tilr` is attached to a pane in which a program copies `hi`
- **THEN** `tilr` writes `ESC ] 52 ; c ; aGk= ESC \` to its terminal
