# clipboard-host-events Specification

## Purpose

What crosses between the programs in the panes and the user's client outside the terminal stream: the shared clipboard (`ClipboardPut`, read back by the server's own `xclip` and `wl-paste`), and host events (`tilr edit` / `tilr open` in a pane reaching the clients watching it as `HostEvent`) over the host socket `$TILR_SOCK`. Who may connect to the server's main socket is in the `access-control` spec; the `tilr` client's other commands are in the `tilr-cli` spec.

## Requirements

### Requirement: The clipboard and its limit are announced
Unless the server runs with `--no-clipboard`, it SHALL list `"clipboard"` in `HelloAck.features` and set `HelloAck.clipboard_max_bytes` to `--clipboard-max` (default `20MB`; units K, M, G in powers of 1024).

**Verified by**: `panes::hello_announces_the_clipboard_and_its_limit` and `clipboard::an_upload_over_the_limit_is_refused_and_leaves_no_stale_content`.

#### Scenario: Limit in the HelloAck
- **WHEN** the server runs with `--clipboard-max 3MB`
- **THEN** `HelloAck.features` is `["clipboard", "ping", "files", "host-events", "agent", "exec"]` and `clipboard_max_bytes` is 3145728

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

### Requirement: An upload becomes the clipboard on its last chunk
`ClipboardPut` chunks of one `transfer_id` (`mime`, `total_size` in the first) SHALL be joined per client; the content MUST become readable only once `last` came with exactly `total_size` bytes, then a `ClipboardAck` `ok` true follows. What the client sends after the last chunk SHALL be handled after it: no need to wait for the ack.

**Verified by**: `clipboard::a_pasted_image_reaches_a_program_that_reads_the_clipboard` and UC-075 (`uc_075.py::an_image_put_on_the_clipboard_is_pasted_byte_for_byte`).

#### Scenario: Paste without waiting for the ack
- **WHEN** a client uploads an image in several chunks and, without waiting for the ack, runs Claude Code's image check in a pane
- **THEN** the check finds the image, and a `ClipboardAck` with `ok` true and `transfer_id` 1 arrives

#### Scenario: Assembled in order
- **WHEN** a transfer comes in chunks, the last with `last`
- **THEN** nothing is readable before the last chunk and the whole content after it (unit test `clipboard::tests::transfer_in_chunks_publishes_on_the_last`, crates/tilrd/src/clipboard.rs)

### Requirement: The first chunk empties the clipboard
The first chunk of an upload (a new `transfer_id` from the client) SHALL empty the clipboard and void whatever transfer that client was sending, so a failed upload never leaves an older image to be pasted.

**Verified by**: `clipboard::an_upload_over_the_limit_is_refused_and_leaves_no_stale_content` and UC-075 (`uc_075.py::the_clipboard_takes_exactly_its_limit_and_refuses_one_byte_more_leaving_it_empty`).

#### Scenario: A refusal after a paste
- **WHEN** an image was pasted and the next upload is refused
- **THEN** the image check in the pane no longer finds an image

### Requirement: Bad uploads are refused with a reason
An upload MUST be refused with one `ClipboardAck` `ok` false, its error meant for the user, when `mime` is empty, not printable ASCII or over 127 bytes (`has no valid MIME type`), `total_size` is 0 (`is empty`) or over the limit (`too large: <size>, the server accepts at most <limit>`, on the first chunk), or the data is more or less than announced. The rest of a refused transfer MUST be ignored.

**Verified by**: `clipboard::an_upload_over_the_limit_is_refused_and_leaves_no_stale_content`.

#### Scenario: A gigabyte announced
- **WHEN** the limit is 64 KiB and a client starts a transfer announcing 1 GiB
- **THEN** the ack has `ok` false and an error containing `too large: 1 GB` and `at most 64 KB`

#### Scenario: Exactly the limit
- **WHEN** the limit is 1 MiB and a client uploads exactly 1 MiB, then 1 MiB + 1 byte, then 20 bytes announcing 10
- **THEN** the first is taken and pasted whole, the others are refused (UC-075 `uc_075.py::the_clipboard_takes_exactly_its_limit_and_refuses_one_byte_more_leaving_it_empty`)

#### Scenario: Unit checks
- **WHEN** a transfer has no MIME type, lies about its size, or is oversized
- **THEN** it is refused and the clipboard is empty (unit tests `clipboard::tests::transfer_without_a_type_is_refused`, `clipboard::tests::transfer_that_lies_about_its_size_is_refused`, `clipboard::tests::oversized_transfer_is_refused_and_leaves_the_clipboard_empty`, crates/tilrd/src/clipboard.rs)

### Requirement: One clipboard shared by all panes
The server SHALL keep one clipboard per user and machine, in `clipboard/content` of the runtime directory (`$TILR_RUNTIME_DIR`, else `$XDG_RUNTIME_DIR/tilr`, else `tilr-<uid>` in the temp directory; directories 0700), shared by every pane and client; an upload replaces it whole, so readers see the old content or the new, never a mix.

**Verified by**: `clipboard::the_clipboard_is_shared_by_all_panes_and_replaced_by_the_next_paste`.

#### Scenario: Another client's pane
- **WHEN** client A uploads `from the first paste` and client B opens a pane later and runs `xclip -selection clipboard -o`
- **THEN** B's program reads `from the first paste`, and after B uploads `replaced`, `wl-paste --no-newline` in A's pane prints `replaced`

### Requirement: Content expires
Uploaded content SHALL stop being served `--clipboard-ttl` seconds after it was stored (default 300; 0 keeps it until replaced).

**Verified by**: `clipboard::pasted_content_expires`, the unit test `clipboard::tests::store_round_trip_and_expiry` (crates/tilrd/src/clipboard.rs) and UC-075 (`uc_075.py::clipboard_content_expires`).

#### Scenario: One-second TTL
- **WHEN** the server runs with `--clipboard-ttl 1`, an image is pasted and found, and 2 s pass
- **THEN** the image check in the pane no longer finds it

### Requirement: The panes get the server's xclip and wl-paste first on PATH
With the clipboard on, the server SHALL link its executable as `xclip`, `wl-paste` and `wl-copy` (and `tilr`) in `bin/` of the runtime directory, put that directory first on every pane's `PATH`, and set `TILR_SHIMS` to it, `TILR_CLIPBOARD_DIR` to the clipboard directory, `TILR_CLIPBOARD_MAX` to `--clipboard-max` in bytes and, with a `--clipboard-ttl`, `TILR_CLIPBOARD_TTL` to it in seconds, so a profile that resets `PATH` can restore it with `PATH="$TILR_SHIMS:$PATH"`.

**Verified by**: `clipboard::a_pasted_image_reaches_a_program_that_reads_the_clipboard` and `clipboard::the_clipboard_can_be_turned_off`.

#### Scenario: Claude Code's commands
- **WHEN** a PNG is on the clipboard and a pane runs Claude Code's `xclip -selection clipboard -t TARGETS -o | grep image/png` and then `xclip -selection clipboard -t image/png -o > file`
- **THEN** the check succeeds and the file is the uploaded image byte for byte

#### Scenario: wl-copy is the server's
- **WHEN** a pane runs `command -v wl-copy`
- **THEN** it prints `$TILR_SHIMS/wl-copy`

### Requirement: The server's xclip and wl-paste answer from the clipboard
Run as `xclip -selection clipboard -o [-t TARGETS|<mime>]` (option prefixes as xclip takes them) or `wl-paste [-l] [-t <mime>]`, the executable SHALL print the type list (xclip adds `TARGETS`; text content adds `text/plain`) or the content for its type; a text request MUST get content only when it is text, else exit non-zero with `target <t> not available, the clipboard holds <mime>`.

**Verified by**: `clipboard::a_pasted_image_reaches_a_program_that_reads_the_clipboard`.

#### Scenario: wl-paste and a text request
- **WHEN** a PNG is on the clipboard and a pane runs `wl-paste -l`, `wl-paste --type image/png` and `xclip -selection clipboard -o`
- **THEN** the first prints `image/png\n`, the second the image, and the third exits non-zero

#### Scenario: Parsing
- **WHEN** the tools are run with the argument forms of xclip and wl-paste
- **THEN** they are read as type list, content or other (unit tests `clipboard::tests::xclip_requests`, `clipboard::tests::wl_paste_requests`, `clipboard::tests::answers_for_an_image`, `clipboard::tests::answers_for_text`, crates/tilrd/src/clipboard.rs)

### Requirement: Other requests go to the real tools
While the clipboard is empty, and for what it does not serve (the primary selection, `--watch`, `wl-paste` with a file argument), the server's `xclip`/`wl-paste`/`wl-copy` MUST hand over to the next tool of that name on `PATH` that is not itself, or fail with `the tilr clipboard is empty and no other <tool> is installed`. Copying to the clipboard selection is served by the server (see "A copy in a pane reaches the clients").

**Verified by**: `clipboard::a_pasted_image_reaches_a_program_that_reads_the_clipboard` (no real tool in the tests' `PATH`: the check fails before the first paste).

#### Scenario: Nothing pasted yet
- **WHEN** nothing was pasted and no other `xclip`/`wl-paste` is installed, and a pane runs Claude Code's image check
- **THEN** the check exits non-zero

#### Scenario: The primary selection
- **WHEN** a pane runs `echo hi | xclip -selection primary -i` and no other `xclip` is installed
- **THEN** it exits non-zero naming the missing tool and no `ClipboardWrite` is sent

### Requirement: Panes know their server and its host socket
Every pane SHALL have `TILR_PANE` (its id), `TILR_SERVER` (`HelloAck.server_id`) and `TILR_SOCK` (the host socket) in its environment and the server's `tilr` first on `PATH`. The host socket SHALL be `<server_id>.sock` in the runtime directory (macOS: `~/Library/Application Support/tilr` unless `TILR_RUNTIME_DIR` or `XDG_RUNTIME_DIR` is set; over 103 bytes: under `/tmp/tilr-<uid>/`), or `<server_id>-<pid>.sock` when that exists (its own) or another live server is at `<server_id>.sock`.

**Verified by**: `files::tilr_edit_in_a_pane_asks_the_clients_watching_it_to_open_the_file`, `files::two_servers_of_one_user_keep_their_host_sockets` and `reload::a_second_server_keeps_its_host_socket_across_a_reload`.

#### Scenario: Pane environment
- **WHEN** a client opens a pane and runs `test "$TILR_PANE" = <pane id> && test "$TILR_SERVER" = <server_id> && test -S "$TILR_SOCK"`
- **THEN** the command exits 0

#### Scenario: Two servers of one user
- **WHEN** two `tilrd` of one user run on different Unix sockets with one state directory, and each has a pane with a client watching it
- **THEN** the panes' `TILR_SOCK` differ, the first server's is `<server_id>.sock`, and `tilr edit` works in a pane of each

#### Scenario: Reload keeps the path
- **WHEN** the second server (on `<server_id>-<pid>.sock`) reloads and a pane created before the reload runs `tilr edit`
- **THEN** it reaches that server's clients

### Requirement: The host socket is private and goes with the server
The host socket SHALL be mode 0600 in a 0700 directory, replacing a stale socket at its path (one no server accepts on) but refusing a file that is not a socket; the server MUST remove it when it stops (a reload keeps it), and only while that file is still the socket it bound. At start the server SHALL remove `<server_id>-<pid>.sock` files of pids not running. If it cannot be bound the server SHALL run on without host events (no `"host-events"` feature).

**Verified by**: `files::tilr_edit_fails_with_no_client_watching_or_outside_a_pane`, `files::two_servers_of_one_user_keep_their_host_sockets` and UC-035 (`uc_035.py::another_user_cannot_send_tilr_edit_to_the_pane`).

#### Scenario: Stop removes the socket
- **WHEN** the server is sent SIGTERM and exits
- **THEN** the path in `$TILR_SOCK` no longer exists

#### Scenario: Stopping one of two servers
- **WHEN** of two servers of one user the one on `<server_id>-<pid>.sock` stops, and then the other
- **THEN** after the first stop the other's `<server_id>.sock` still exists and `tilr edit` works in its panes

#### Scenario: Modes
- **WHEN** a pane's `$TILR_SOCK` is looked at
- **THEN** the socket is 0600 and its directory 0700

### Requirement: The host socket serves the user's own programs only
The server SHALL check the peer uid of every connection to the host socket and MUST refuse one from another uid (log `host socket: refused a connection`) even where the socket's permissions were opened by hand, so another user cannot send a `HostEvent` to the user's clients.

**Verified by**: UC-035 (`uc_035.py::another_user_cannot_send_tilr_edit_to_the_pane`).

#### Scenario: Another uid
- **WHEN** uid 65534 runs `tilr edit /etc/hostname` with the pane's `TILR_SOCK` and `TILR_PANE`, first with the socket at 0600, then with the socket 0666 in a 0755 directory
- **THEN** the first fails with permission denied, the second is refused by the server, and no `HostEvent` reaches the watching client

### Requirement: The host socket speaks JSON lines
A program SHALL send one JSON object per line, `{"pane_id", "name" (default "open"), "kind", "path", "args"}`, at most 1 MiB per connection, and get one JSON line per request: `{"ok":true,"clients":N}`, or `{"ok":false,"error":...}` with `no such pane: <id>`, `no tilr client attached`, or `bad request: ...` for a line that is not such JSON.

**Verified by**: `files::tilr_edit_fails_with_no_client_watching_or_outside_a_pane`.

#### Scenario: Not JSON
- **WHEN** a program writes `not json\n` to the host socket
- **THEN** it reads a line starting with `{"ok":false,"error":"bad request`

#### Scenario: Minimal request and reply
- **WHEN** a request has only `pane_id`, and a request reaches two clients
- **THEN** its name is `open`, and the reply is `{"ok":true,"clients":2}` (unit tests `host::tests::a_request_needs_only_the_pane`, `host::tests::replies_carry_what_applies`, crates/tilrd/src/host.rs)

### Requirement: Host events reach every listening client watching the pane
For a request about a pane, the server SHALL send `HostEvent{pane_id, name, kind, path, args}` to every client subscribed to that pane that listed `"host-events"` in `Hello.features`, and to no other client (one watching without the feature, one watching another pane), logging `host event delivered`.

**Verified by**: `files::tilr_edit_in_a_pane_asks_the_clients_watching_it_to_open_the_file` and UC-076 (`uc_076.py::tilr_edit_and_open_reach_every_client_watching_the_pane_and_no_other`).

#### Scenario: Two clients watch the pane
- **WHEN** clients A and B with `"host-events"` (B on another host) watch a pane and `tilr edit notes/todo.txt` runs in it
- **THEN** both get the `HostEvent` with that `pane_id`

#### Scenario: Viewers without the feature
- **WHEN** a third client watches the pane without `"host-events"` and a fourth with it watches another pane
- **THEN** neither gets the event

### Requirement: tilr edit and tilr open say what to open
`tilr edit FILE` and `tilr open [--kind KIND] TARGET [-- ARGS...]` in a pane SHALL send name `open`, kind `file` (for `edit` and a target without a scheme) with the path made absolute, `url` for a target with a scheme, or the `--kind` given, the target as is; trailing arguments go in `args`. On success they exit 0.

**Verified by**: `files::tilr_edit_in_a_pane_asks_the_clients_watching_it_to_open_the_file` and UC-076 (`uc_076.py::tilr_edit_and_open_reach_every_client_watching_the_pane_and_no_other`).

#### Scenario: Edit a relative file
- **WHEN** `cd ~/project && tilr edit notes.md` runs in a watched pane
- **THEN** the event has name `open`, kind `file`, path `<home>/project/notes.md` and empty `args`, and the command exits 0

#### Scenario: URL with arguments, and another kind
- **WHEN** `tilr open https://example.com/a -- --float` and then `tilr open --kind image ../x.png` run in the pane
- **THEN** the events are kind `url`, path `https://example.com/a`, args `["--float"]`, and kind `image`, path `../x.png`
- **AND** a target is a URL only when its scheme is letters, digits, `+`, `-`, `.` before `://` (unit test `host::tests::urls_are_told_from_files`, crates/tilrd/src/host.rs)

### Requirement: tilr edit fails with a reason when nobody listens
`tilr edit` / `tilr open` SHALL exit 1 with, on stderr, `tilr: no tilr client attached` when no client with `"host-events"` watches the pane, `tilr: not inside a tilr pane` without `TILR_PANE`/`TILR_SOCK`, `tilr: no such pane: <id>` for a pane the server lacks, and `tilr: cannot reach the tilr server: ...` (waiting at most 10 s).

**Verified by**: `files::tilr_edit_fails_with_no_client_watching_or_outside_a_pane` and UC-076 (`uc_076.py::tilr_edit_with_nobody_listening_fails_and_says_why`).

#### Scenario: Only an old client watches
- **WHEN** the only client watching the pane did not list `"host-events"` and `tilrd edit notes.md` runs with that pane's `TILR_PANE` and `TILR_SOCK`
- **THEN** it exits 1 with stderr `tilr: no tilr client attached\n`

#### Scenario: Listener comes and goes
- **WHEN** `tilr edit x.txt` runs with no listener, then with a listener subscribed, then after it unsubscribed
- **THEN** it fails, succeeds (the listener gets the event), and fails again

### Requirement: The pane's tilr hands other commands to the next tilr
Run as `tilr` (the link on the panes' `PATH`), the server's executable SHALL answer only `edit`, `open` and help, and MUST hand every other command to the next `tilr` on `PATH` that is not itself (the user's client), or fail with `tilr: no other tilr is installed`. The server also answers `tilrd edit` / `tilrd open`.

**Verified by**: `files::tilr_edit_in_a_pane_asks_the_clients_watching_it_to_open_the_file`.

#### Scenario: Another command without another tilr
- **WHEN** `tilr ctl apps` runs in a pane where no other `tilr` is on `PATH`
- **THEN** it exits non-zero and stderr contains `no other tilr is installed`

### Requirement: The tilr client answers edit and open the same way
The `tilr` client SHALL answer `tilr edit` and `tilr open` with the same request on `$TILR_SOCK` and the same exit statuses and messages as the server's `tilr`, for a pane whose profile reset `PATH`.

**Verified by**: `client::tilr_edit_in_a_pane_asks_the_server_of_the_pane`.

#### Scenario: The client's own tilr edit
- **WHEN** the `tilr` client runs `tilr edit notes.md` with `TILR_PANE=pane-1` and `TILR_SOCK` naming a socket
- **THEN** it sends a JSON line with `pane_id` `pane-1`, `kind` `file` and the absolute path of `notes.md`, and exits 0 on `{"ok":true,"clients":1}`

### Requirement: tilr edit and tilr open talk only to a host socket of the user's
`tilr edit` and `tilr open`, run as the pane's `tilr`, as `tilrd edit` / `tilrd open` or as the `tilr` client, SHALL read the peer credentials of `$TILR_SOCK` after connecting (`SO_PEERCRED` on Linux, `getpeereid` on macOS) and MUST NOT write a byte to a socket whose peer uid is not their own euid: they SHALL exit 1 with `tilr: cannot reach the tilr server: socket <path> is served by another user (uid N), not by this one (uid M): nothing was sent`.

**Verified by**: unit test `local::tests::a_peer_of_another_uid_is_refused` (crates/tilr-proto/src/local.rs), itests `UC-03A` (`uc_03a.py::tilr_edit_says_nothing_to_a_host_socket_another_user_serves`).

#### Scenario: Another user's socket at TILR_SOCK
- **WHEN** another uid serves a socket (mode 0666, in a directory of mode 0755) and the user runs `tilr edit notes.md`, `tilr open https://example.com` and `tilrd edit notes.md` with `TILR_PANE` set and `TILR_SOCK` naming that socket
- **THEN** each exits 1 with stderr containing `is served by another user (uid N)`, and the other user's listener received 0 bytes on every connection

#### Scenario: The user's own server
- **WHEN** `tilr edit notes.md` runs in a pane of the user's server with a listening client watching it
- **THEN** it exits 0 and the client receives the `HostEvent`

### Requirement: Another server starting leaves the pane tools in place
The links in `bin/` of the runtime directory are shared by all servers of the user. A starting server SHALL leave a link that already points at its executable as it is and replace any other by a rename, so a pane of another server MUST never find `tilr`, `xclip` or `wl-paste` missing while it starts.

**Verified by**: `files::a_second_server_starting_leaves_the_first_ones_panes_their_tools` and unit test `clipboard::tests::the_shims_are_never_missing_while_another_server_installs_them` (crates/tilrd/src/clipboard.rs).

#### Scenario: A second server starts three times
- **WHEN** a pane of one server checks `[ -e "$TILR_SHIMS/tilr" ]` in a loop while a second server of the same user starts and stops three times
- **THEN** the check never fails, and `tilr edit` in the pane still reaches its client

#### Scenario: Links replaced over and over
- **WHEN** the links are installed 2000 times, for two executables in turn, while another thread looks at `bin/tilr`
- **THEN** the link is never missing, and `bin/` holds only the links

### Requirement: A probe of the host socket is not a refused connection
A starting server SHALL tell a live server at `<server_id>.sock` by connecting to it, and the live server SHALL close a connection whose peer has gone before its uid check without logging `host socket: refused a connection`.

**Verified by**: `files::a_second_server_starting_leaves_the_first_ones_panes_their_tools`.

#### Scenario: A second server's probe
- **WHEN** a second server of the same user starts, finds the first one's `<server_id>.sock` live and takes `<server_id>-<pid>.sock`
- **THEN** the first server's log has no `refused a connection` line

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

## Notes

- Requirements without a test: none. Untested details inside requirements: the 1 MiB per-connection input cap of the host socket, the `/tmp/tilr-<uid>` fallback for long socket paths, the server running on without host events when the socket cannot be bound, `tilr: cannot reach the tilr server`, and handing over to a real `xclip`/`wl-paste` (the tests have none on `PATH`).
- No itests case cited here is currently red at HEAD per itests/USE_CASES.md (UC-035, UC-075, UC-076 pass).
- Unit tests in `crates/tilrd/src/*.rs` are cited as `module::tests::name` with their file.
