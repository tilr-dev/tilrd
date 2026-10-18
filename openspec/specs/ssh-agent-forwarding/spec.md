# ssh-agent-forwarding Specification

## Purpose

Covers forwarding the SSH agent of the user's computer (the client's side) to programs in panes, as `ssh -A` does: the per-pane agent socket and `SSH_AUTH_SOCK`, which client serves a connection, and the generic byte streams (`StreamOpen` / `StreamData` / `StreamClose`) that carry the agent protocol, with their limits and back pressure. The client queue's two lanes and the 1 MiB in-flight window are owned by the `protocol` spec; reload hand-over by the `reload` spec.

## Requirements

### Requirement: Forwarding is on unless turned off
The server SHALL forward the agent unless it runs with `--no-agent` or its configuration file says `agent = false` (unset means on; the flag wins over `agent = true`). While on, it MUST list `"agent"` in `HelloAck.features`. While off, `HelloAck.features` MUST NOT contain `"agent"`, no pane gets `SSH_AUTH_SOCK`, no runtime `agent/` directory is made, and stream messages from clients are ignored.

**Verified by**: `agent::the_agent_is_forwarded_unless_turned_off`.

#### Scenario: Default server
- **WHEN** a client says hello to a server started without flags
- **THEN** `HelloAck.features` contains `"agent"`

#### Scenario: Turned off by flag or file
- **WHEN** the server runs with `--no-agent`, or with `agent = false` in `server.toml`
- **THEN** `HelloAck.features` lacks `"agent"`
- **AND** `printenv SSH_AUTH_SOCK` in a new pane prints nothing and `<runtime>/agent` does not exist

### Requirement: Every pane has its own agent socket
While forwarding is on, every pane SHALL get a Unix socket of its own at `agent/<pid>/<pane id>.sock` in the runtime directory (`$XDG_RUNTIME_DIR/tilr`, on macOS `~/Library/Application Support/tilr`, `/tmp/tilr-<uid>` where that path is too long for a socket), `<pid>` being the server's, and `SSH_AUTH_SOCK` in the pane's environment MUST point at it. The socket MUST have mode 0600 and the `agent/<pid>/` and `agent/` directories (and the runtime directory above them) mode 0700.

**Verified by**: `agent::a_pane_has_ssh_auth_sock_pointing_at_its_own_socket`.

#### Scenario: Two panes
- **WHEN** a client opens two panes
- **THEN** each pane's `SSH_AUTH_SOCK` ends in `agent/<the server's pid>/<its pane id>.sock`, a socket of mode 0600 in a directory of mode 0700, and the two paths differ

### Requirement: The socket goes with its pane
A pane's agent socket SHALL be removed when the pane goes and when the server stops for good; streams already open through it go on after the pane is gone. If the socket cannot be made, the pane MUST still start, without `SSH_AUTH_SOCK`.

**Verified by**: `agent::a_pane_has_ssh_auth_sock_pointing_at_its_own_socket`.

#### Scenario: The pane exits
- **WHEN** the shell of the first pane exits
- **THEN** its socket is removed and the second pane's socket stays

### Requirement: Only the user's own programs may use the socket
The server SHALL check the uid of every program that connects to an agent socket and MUST refuse (close) a connection from another uid, even where the socket's and directory's permissions were opened by hand, logging `agent socket: refused a connection`.

**Verified by**: itests `UC-035` (`uc_035.py::another_user_cannot_send_tilr_edit_to_the_pane`, which opens the agent socket's permissions and connects as uid 65534).

#### Scenario: Permissions opened by hand
- **WHEN** the pane's agent socket is chmod 0666 and its directory 0755, and a program of uid 65534 connects
- **THEN** the server refuses the connection and logs `agent socket: refused a connection`

### Requirement: Sockets a killed server left are swept at start
A server that starts fresh (not resuming after a reload) SHALL remove the `agent/<pid>/` directories in the runtime directory whose pid is not running, logging `removed N agent socket(s) a server that is gone left`, and MUST leave the directory of every running server of the same user alone: it MUST NOT connect to their sockets (no stream is opened to their clients, nothing is logged by them) nor remove one, even while it is being made and does not accept yet.

**Verified by**: `agent::the_sockets_a_killed_server_left_are_removed_when_the_next_starts`, `agent::a_server_starting_leaves_the_agent_sockets_of_a_live_one_alone` and itests `UC-036` (`uc_036.py::agent_sockets_left_by_a_killed_server_are_removed_at_the_next_start`).

#### Scenario: SIGKILL, then a new server
- **WHEN** a server with a pane is killed with SIGKILL while another server of the user runs with a pane of its own, and a new server starts
- **THEN** the killed server's `agent/<pid>/<pane>.sock` and its `agent/<pid>/` directory are gone and the live server's socket still exists

#### Scenario: A second server starts next to a live one
- **WHEN** a server has a pane watched by a client with `"agent"`, and a second server of the user starts in the same runtime directory
- **THEN** the first server's client receives no `StreamOpen`, the first server does not log `agent socket: refused a connection`, and the pane's socket is the same file and still serves the agent

### Requirement: The client that subscribed last serves the agent
A client SHALL say it can serve the agent by listing `"agent"` in `Hello.features`. When a program connects to a pane's socket, the server MUST open the stream to the client that subscribed to that pane last among the clients currently subscribed to it with `"agent"`; clients without the feature do not count, and when that client unsubscribes the next most recent one serves.

**Verified by**: `agent::the_client_that_subscribed_last_serves_the_agent`.

#### Scenario: Two agent clients and a plain one
- **WHEN** agent client A made the pane, agent client B subscribed after it, then a plain client subscribed, and a program connects
- **THEN** only B gets `StreamOpen`; A and the plain client get no stream message

#### Scenario: The last one leaves
- **WHEN** B unsubscribes and a program connects again
- **THEN** A gets `StreamOpen` with a new `stream_id`

### Requirement: With no agent client the program finds no agent
When no client with `"agent"` is subscribed to the pane (none attached, only clients without the feature, or the agent client does not watch the pane or has unsubscribed), the server SHALL close the program's connection at once and send no client anything, so that `ssh` takes it for an agent without keys.

**Verified by**: `agent::with_no_agent_client_attached_the_program_finds_no_agent` and itests `UC-078` (`uc_078.py::without_the_clients_agent_the_clone_is_refused`).

#### Scenario: Watched by a plain client only
- **WHEN** a program connects to the socket of a pane watched only by a client without `"agent"`
- **THEN** the program reads end of file at once and the client receives no `StreamOpen`

### Requirement: A connection opens a stream to the client
For every accepted connection that has a serving client, the server SHALL send that client `StreamOpen` with a `stream_id` chosen by the server and never reused while it runs, `kind` `"ssh-agent"` and `pane_id` the pane whose socket was connected to. Each new connection MUST be a new stream. If the client's queue cannot take the `StreamOpen`, the program's connection is closed.

**Verified by**: `agent::a_program_talks_to_the_agent_of_the_client_through_the_panes_socket`.

#### Scenario: A program connects
- **WHEN** a program connects to the pane's `SSH_AUTH_SOCK` while an agent client watches the pane
- **THEN** the client gets `StreamOpen` with `kind` `"ssh-agent"` and `pane_id` the pane's id
- **AND** a second connection gets a `StreamOpen` with a different `stream_id`

### Requirement: Bytes go both ways unchanged and in order
The server SHALL carry a stream's bytes unread and unchanged: what the program writes reaches the client in order as `StreamData` of at most 64 KiB each; the client's `StreamData` (at most 64 KiB each) is written to the program in order.

**Verified by**: `agent::a_program_talks_to_the_agent_of_the_client_through_the_panes_socket` and itests `UC-073` (`uc_073.py::a_stream_echoed_by_the_client_comes_back_whole_in_order`, `uc_073.py::a_big_stream_to_the_client_arrives_whole_and_in_order`).

#### Scenario: An agent request and its answer
- **WHEN** the program writes `\0\0\0\x01\x0b` and the client answers with `StreamData` `\0\0\0\x05\x0c\0\0\0\0`
- **THEN** the client receives exactly the 5 bytes and the program reads exactly the 9 bytes

#### Scenario: More than one message's worth
- **WHEN** 200 KiB go each way
- **THEN** they arrive whole and in order, split into several `StreamData`

### Requirement: Messages for another stream are ignored
`StreamData` or `StreamClose` from a client for a stream that is closed, was never opened, or belongs to another client SHALL be ignored, with no answer.

**Verified by**: `agent::a_program_talks_to_the_agent_of_the_client_through_the_panes_socket`.

#### Scenario: Data for a dead stream
- **WHEN** the client sends `StreamData` for a closed stream and for stream 999
- **THEN** nothing happens and no stream message comes back

### Requirement: Either side ends a stream with StreamClose
When the program closes its connection, the server SHALL send the client `StreamClose` after the stream's last `StreamData`. When the client sends `StreamClose`, the program MUST read what was sent before it and then end of file. `StreamClose` is not answered.

**Verified by**: `agent::a_program_talks_to_the_agent_of_the_client_through_the_panes_socket`.

#### Scenario: The program hangs up
- **WHEN** the program closes its connection
- **THEN** the client gets `StreamClose` with that `stream_id`

#### Scenario: The client closes
- **WHEN** the client sends `StreamData` "last words" and then `StreamClose`
- **THEN** the program reads "last words" and then end of file

### Requirement: A client that goes takes its streams with it
When a client disconnects or its connection is cut (reset, or silent past the client timeout), its streams SHALL end (the program reads end of file); the socket stays for the next client.

**Verified by**: `agent::the_program_reads_the_end_when_the_agent_client_disconnects` and itests `UC-047` (`uc_047.py::an_agent_stream_cut_by_a_reset_ends_for_the_program_and_the_next_one_works`, `uc_047.py::an_agent_stream_on_a_silent_connection_ends_for_the_program_and_the_next_one_works`).

#### Scenario: Disconnect mid-stream
- **WHEN** the agent client disconnects while a program's stream is open
- **THEN** the program reads end of file
- **AND** a new agent client that watches the pane gets `StreamOpen` for the next connection

### Requirement: At most 64 streams per client
A client SHALL have at most 64 streams open at once; a program that connects while the serving client has 64 open MUST have its connection closed at once, and no `StreamOpen` is sent.

#### Scenario: The 65th connection
- **WHEN** a client has 64 agent streams open and a program connects to a socket it would serve
- **THEN** the program's connection is closed and the client gets no `StreamOpen`

### Requirement: A slow client holds the program back
Stream data to the client and the `StreamClose` after it SHALL go by the client's bulk lane (see the `protocol` spec), at most 4 MiB per client for all its streams and file reads; the server MUST read from the program only as that lane has room, so a slow client holds the program up, the server's memory stays flat, and the client is not dropped.

**Verified by**: itests `UC-073` (`uc_073.py::a_slow_client_holds_the_program_back_and_is_neither_dropped_nor_buffered_for`).

#### Scenario: A client reading at 1 MB/s
- **WHEN** a program sends 64 MiB to a client limited to 1 MB/s down
- **THEN** after 10 s the program has got at most 24 MiB out, the server's RSS has grown by at most 16 MB and the client is still connected
- **AND** once the client is fast again all 64 MiB arrive in order

### Requirement: A client running ahead of the program loses the stream
The server SHALL queue at most 64 `StreamData` messages (about 4 MiB) from the client for one program; when the client sends more while the queue is full, the server MUST end the stream: it sends the client `StreamClose`, and the program reads end of file. The client stays connected.

**Verified by**: itests `UC-073` (`uc_073.py::a_client_running_ahead_of_a_slow_program_is_held_to_the_queue_and_then_loses_the_stream`).

#### Scenario: 2 MiB to a slow reader
- **WHEN** the client sends 2 MiB to a program reading 512 KB/s and then `StreamClose`
- **THEN** the program reads all 2 MiB intact

#### Scenario: 64 MiB at once
- **WHEN** the client sends 64 MiB at once to a program reading 100 KB/s
- **THEN** the client gets `StreamClose`, the program reads end of file having got at most 8 MiB, and the server's RSS grows by at most 16 MB

### Requirement: A reload ends open streams and keeps the socket path
A reload or update SHALL end every open agent stream (the program reads end of file) and make each pane's socket again at the same path, so the shell's `SSH_AUTH_SOCK` stays valid and the next connection reaches a client that reconnects and watches the pane. The path is the one the pane got: `agent/<pid>/<pane id>.sock` (the pid survives the exec), or `agent/<pane id>.sock` for a pane a server from before the per-server directories started, while that socket file is there.

**Verified by**: `agent::the_panes_socket_works_after_a_reload` and itests `UC-064` (`uc_064.py::an_agent_stream_open_during_an_update_ends_for_the_program_and_the_next_one_works`).

#### Scenario: SIGUSR1 with a stream open
- **WHEN** a program has a stream open and the server reloads
- **THEN** the program reads end of file
- **AND** after the client reconnects and watches the pane, `SSH_AUTH_SOCK` is the same path and a new connection gets `StreamOpen` and carries bytes both ways

### Requirement: Real ssh and git use the forwarded agent
The forwarding SHALL serve the SSH agent protocol unchanged, so that `ssh-add -l` in a pane lists the keys of the client's agent, `git clone` over ssh in a pane authenticates with them, and `ssh -A` from the pane forwards the same agent one hop further; the private key never leaves the client's machine.

**Verified by**: itests `UC-078` (`uc_078.py::ssh_add_in_a_pane_lists_the_keys_of_the_clients_agent`, `uc_078.py::git_clone_over_ssh_uses_the_forwarded_agent_and_ssh_a_forwards_it_further`).

#### Scenario: git clone through the client's agent
- **WHEN** the client relays its streams to an ssh-agent holding a key that the git host accepts, and the pane runs `git clone` over ssh
- **THEN** the clone succeeds
- **AND** `ssh-add -l` in the pane lists the same fingerprint as on the client's machine

## Notes

- Requirements without a test: "At most 64 streams per client". Parts without a test: a pane started without `SSH_AUTH_SOCK` when its socket cannot be made, streams surviving their pane's exit, sockets removed on a normal stop, a dropped `StreamOpen` when the client's queue is full, a pane from a server before the per-server directories keeping `agent/<pane id>.sock` after a reload (no older binary to reload from in the tests).
- No itests case for these is currently red. `uc_073.py` was red until 2026-10-17 (packet-counted queue); it is green at HEAD per itests/USE_CASES.md.
- The streams are generic in the proto (`StreamOpen.kind`), but `"ssh-agent"` is the only kind at HEAD; the SOCKS proxy does not use them (see the `socks-proxy` spec).
- The itests case for real `ssh -A` / `git clone` is UC-078, not UC-075 (UC-075 is the clipboard).
