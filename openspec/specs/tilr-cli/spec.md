# tilr-cli Specification

## Purpose

Defines the terminal client `tilr` (crate `crates/tilr`): its subcommands, which server it talks to, the attached terminal session (keys, detach, reconnect), the server commands it runs in tilrd, `tilr service install|uninstall`, its output lines and exit status. How the local tilrd is started, locked and stopped lives in `daemon-lifecycle`; `tilr edit` / `tilr open` in `clipboard-host-events`; token and peer-uid rules in `access-control`; what `update` does in the server in `update`.

## Requirements

### Requirement: Command set
`tilr` SHALL offer the subcommands `new`, `attach` (alias `a`), `ls` (alias `list`), `kill`, `kill-server`, `ps`, `version`, `token`, `cloud login|status|logout`, `update`, `deploy`, `service install|uninstall`, `edit` and `open`, and the global options `--socket PATH`, `--connect URL` and `--token TOKEN` (also `TILR_SOCKET`, `TILR_CONNECT`, `TILR_TOKEN`). With no subcommand it SHALL behave as `attach` with no pane. `edit` and `open` SHALL make no server connection (see `clipboard-host-events`).

**Verified by**: `client::the_first_tilr_starts_a_detached_server_and_the_next_one_uses_it`, `client::tilr_edit_in_a_pane_asks_the_server_of_the_pane`.

#### Scenario: No subcommand
- **WHEN** the user runs `tilr` in a terminal
- **THEN** it attaches to the pane used last, or makes one and attaches to it

#### Scenario: Version flag
- **WHEN** the user runs `tilr --version`
- **THEN** it prints `tilr <version> (<platform>)`

#### Scenario: edit needs no server connection
- **WHEN** `tilr edit FILE` runs in a pane
- **THEN** the request goes over `$TILR_SOCK` as the `clipboard-host-events` spec says, and no WebSocket connection to tilrd is made

### Requirement: Target selection
The client SHALL pick one server: `--socket` / `TILR_SOCKET` (the local tilrd at that socket), else `--connect` / `TILR_CONNECT` (a tilrd's TCP port), else the configuration file, else the default socket of the `daemon-lifecycle` spec. Giving both `--socket` and `--connect` SHALL fail with `--socket and --connect name two servers: give one`.

**Verified by**: unit tests `target::tests::the_command_line_beats_the_file`, `target::tests::both_a_socket_and_an_address_is_an_error` (crates/tilr/src/target.rs).

#### Scenario: Command line beats the file
- **WHEN** `client.toml` says `connect = "ws://far:7890"` and the user passes `--socket /s`
- **THEN** the target is the local tilrd at `/s`

#### Scenario: Both given
- **WHEN** the user passes `--socket /s --connect ws://h:1`
- **THEN** the command fails with `--socket and --connect name two servers: give one`

### Requirement: Client configuration file
The configuration file SHALL be `$TILR_CONFIG_DIR/client.toml`, else `$XDG_CONFIG_HOME/tilr/client.toml`, else `~/.config/tilr/client.toml`, with only the keys `socket`, `connect` and `token`. A missing file SHALL say nothing; a file with another key or bad TOML SHALL fail the command, naming the file. In the file, `socket` SHALL win over `connect`.

**Verified by**: unit tests `target::tests::the_file_names_a_server_and_its_token`, `target::tests::both_a_socket_and_an_address_is_an_error`.

#### Scenario: File names a remote server
- **WHEN** `client.toml` says `connect = "ws://far:7890"` and `token = "t1"` and nothing is given on the command line
- **THEN** the target is `ws://far:7890` with token `t1`

#### Scenario: Unknown key
- **WHEN** `client.toml` contains `sockett = "/s"`
- **THEN** every command that connects fails, naming the file

### Requirement: Token for a remote target
For a `connect` target the token SHALL be `--token` / `TILR_TOKEN`, else the file's `token`; when the target's host is on the loopback (`localhost`, an address in `127.0.0.0/8`, `::1`), else the `token` file in the state directory (`$TILR_STATE_DIR`, else `$XDG_STATE_HOME/tilr`, else `~/.local/state/tilr`), trimmed; else empty. The token from the state directory MUST NOT be sent to any other host, a name that resolves to the loopback included.

**Verified by**: unit tests `target::tests::the_command_line_beats_the_file`, `target::tests::the_stored_token_goes_to_loopback_targets_only` (crates/tilr/src/target.rs).

#### Scenario: Own machine's port without a token
- **WHEN** the user runs `tilr --connect ws://127.0.0.1:7890 ls` with no token given anywhere
- **THEN** the `Hello.token` is the contents of `~/.local/state/tilr/token`

#### Scenario: Remote target without its own token
- **WHEN** `tilr --connect ws://example.net:7890 ls` runs with a token in the state directory and none given for that target
- **THEN** the `Hello` sent carries an empty token

#### Scenario: Remote target with its own token
- **WHEN** `tilr --connect ws://example.net:7890 --token T ls` runs, or `client.toml` gives `token = "T"`
- **THEN** the `Hello` sent carries `T`

### Requirement: Which commands start the local server
For a local target, `new`, `attach` with no pane (and bare `tilr`), `token`, `cloud …` and `update` SHALL start tilrd when none serves the socket, as the `daemon-lifecycle` spec says. `ls`, `ps`, `kill`, `attach PANE`, `kill-server` and `version` SHALL start nothing. A `--connect` target SHALL never be started.

**Verified by**: `client::the_first_tilr_starts_a_detached_server_and_the_next_one_uses_it`, `client::a_socket_left_by_a_killed_server_is_replaced_by_the_next_tilr`, itests `UC-014` (`uc_014.py::after_a_sigkill_the_next_tilr_replaces_the_socket_and_starts_a_new_daemon`), `UC-017` (`uc_017.py::kill_server_leaves_no_process_and_no_socket`).

#### Scenario: ls on an empty machine
- **WHEN** no tilrd runs and the user runs `tilr ls`
- **THEN** it exits non-zero with `no tilrd runs at <socket> (<error>)` on stderr and no tilrd is started

#### Scenario: new on an empty machine
- **WHEN** no tilrd runs and the user runs `tilr new -d`
- **THEN** a tilrd is started, a pane is made and its id is printed

#### Scenario: Nothing at a remote address
- **WHEN** nothing listens at the `--connect` URL
- **THEN** the command fails with `no tilrd answers at <url> (<error>)`

### Requirement: Connecting and saying hello
Every connection SHALL be a WebSocket (to `ws://localhost/` over the Unix socket, or to the URL over TCP with `TCP_NODELAY`) that begins with `Hello` carrying `protocol_version` = `PROTOCOL_VERSION`, `client_name` `tilr`, the token (empty on the socket) and the `client_id`. Connecting and the `HelloAck` SHALL take at most 5 s (`<target> did not answer`), and each request (`ListPanes`, `CreatePane`) at most 10 s (`the server did not answer`).

**Verified by**: itests `UC-015` (`uc_015.py::another_programs_listener_at_the_socket_is_left_alone`) for the foreign listener; the timeouts and the version mismatch are untested.

#### Scenario: Other protocol version
- **WHEN** the `HelloAck.protocol_version` differs from the client's
- **THEN** the command fails with `the server speaks protocol N, tilr M`; an `Error` instead of `HelloAck` fails with its message

#### Scenario: Foreign listener at the socket
- **WHEN** another program listens on the local socket path, its lock is free, and the user runs `tilr`
- **THEN** `tilr` fails with `something that is not tilrd listens on <socket> (nothing was removed): …` and the socket is left as it was

### Requirement: One client id per run
The client SHALL make one `client_id` (a UUID v4) when it starts and SHALL send the same `client_id` in the `Hello` of every connection it makes while it runs, reconnects included. What the server does with it is in the `client-liveness` spec.

**Verified by**: `client::reconnects_say_the_same_client_id`.

#### Scenario: Reconnect keeps the id
- **WHEN** an attached `tilr` loses its connection and connects again
- **THEN** the new `Hello.client_id` equals the one of the first connection

### Requirement: Default pane
The pane used last SHALL be the one with the greatest `max(PaneInfo.used_at_ms, created_at_ms)` (ties by `created_at_ms`), leaving out the pane `tilr` runs in (`$TILR_PANE`). `attach` with no pane SHALL attach to it, or, with no such pane, SHALL make one (`CreatePane` with the terminal size and the current directory) and attach to it. `attach` of the pane `tilr` runs in SHALL fail with `<id> is the pane this runs in: it cannot show itself`.

**Verified by**: unit test `tests::the_pane_used_last_is_attached_to` (crates/tilr/src/main.rs), itests `UC-011` (`uc_011.py::a_second_tilr_attaches_to_the_same_daemon_and_pane`).

#### Scenario: Second tilr
- **WHEN** one `tilr` is attached and a second `tilr` is started with no arguments
- **THEN** the second attaches to the same pane and input typed in one is seen in the other

#### Scenario: Own pane left out
- **WHEN** `tilr` runs inside pane A and pane A was used last
- **THEN** `tilr` attaches to the pane used last among the others

### Requirement: Pane names
A pane SHALL be named by its full id or by a prefix that exactly one pane id starts with. An unknown or empty name SHALL fail with `no pane NAME (tilr ls lists them)`; a prefix several ids start with SHALL fail with `NAME names N panes: give more of the id`.

**Verified by**: unit test `tests::a_pane_is_named_by_the_start_of_its_id` (crates/tilr/src/main.rs).

#### Scenario: Prefix names two panes
- **WHEN** panes `ab12` and `ab34` exist and the user runs `tilr attach ab`
- **THEN** it fails with `ab names 2 panes: give more of the id`

#### Scenario: Unique prefix
- **WHEN** panes `ab12` and `cd56` exist and the user runs `tilr attach cd`
- **THEN** it attaches to `cd56`

### Requirement: new
`tilr new [-d] [--cwd DIR] [-- CMD…]` SHALL send `CreatePane` with the terminal's size (80x24 without one), `cwd` = `--cwd` made absolute or the current directory, and `command` = the words after `--` (empty: the login shell), then attach to the new pane. With `-d` it SHALL print the pane's full id on one line, exit 0 and not attach.

**Verified by**: `client::the_first_tilr_starts_a_detached_server_and_the_next_one_uses_it`, `client::kill_server_ends_the_shells_and_removes_the_socket`.

#### Scenario: Detached new
- **WHEN** the user runs `tilr new -d -- sleep 1000`
- **THEN** stdout is the pane id and a newline, and `tilr ps` lists that pane with command `sleep 1000`

### Requirement: ls and ps output
`tilr ls` SHALL print a header and one line per pane, oldest first: `*` for the pane used last (else a space), the first 8 characters of the id, `COLSxROWS`, the viewer count, the directory and the command (`(shell)` when empty). `tilr ls -q` SHALL print only the full ids, one per line. `tilr ps` SHALL print a header `PANE  PID  SIZE  VIEWERS  CWD  COMMAND` (padded) and per pane its full id, `PaneInfo.pid`, size, viewers, directory and command. Both SHALL exit 0.

**Verified by**: `client::the_first_tilr_starts_a_detached_server_and_the_next_one_uses_it`.

#### Scenario: Quiet listing
- **WHEN** two panes were made with `tilr new -d` and the user runs `tilr ls -q`
- **THEN** the output is exactly the two printed ids, oldest first

#### Scenario: ps
- **WHEN** a pane exists and the user runs `tilr ps`
- **THEN** the output has a `PID` header and a line starting with the pane's full id

### Requirement: kill
`tilr kill [PANE]` SHALL close the named pane, else the pane it runs in (`$TILR_PANE`), else the pane used last, by sending `ClosePane` and polling `ListPanes` every 50 ms until the pane is gone. It SHALL fail with `pane <id> is still there` after 10 s and with `no panes` when there is none to choose.

**Verified by**: `client::kill_closes_the_named_pane_and_the_current_one`.

#### Scenario: Kill with no panes
- **WHEN** the server has no panes and the user runs `tilr kill`
- **THEN** it exits non-zero with `no panes`

### Requirement: kill-server
`tilr kill-server` SHALL work on the local tilrd only. It SHALL take the server's pid from the socket's peer credentials and the pane processes' pids from `ListPanes`, send the server SIGTERM, and wait until the socket's lock is free or gone, the server is gone and every pane process is gone, failing with `tilrd (pid N) did not stop within 10s` after 10 s.

**Verified by**: `client::kill_server_ends_the_shells_and_removes_the_socket`, itests `UC-017` (`uc_017.py::kill_server_leaves_no_process_and_no_socket`).

#### Scenario: Stop the server
- **WHEN** a pane runs `sleep 1000` and the user runs `tilr kill-server`
- **THEN** it exits 0 once the server and the pane process are gone and the socket and lock file are removed
- **AND** a second `tilr kill-server` fails with `no tilrd runs at <socket>`

#### Scenario: Remote target
- **WHEN** the user runs `tilr --connect ws://h:7890 kill-server`
- **THEN** it fails with `kill-server stops the local tilrd; a server at --connect is stopped where it runs`

### Requirement: version
`tilr version` SHALL print `tilr <version> (<platform>)` and then `tilrd <HelloAck.server_version> at <target>`, or `tilrd is not running at <target>` when nothing listens there, and exit 0 in both cases; it SHALL start nothing.

**Verified by**: `client::the_first_tilr_starts_a_detached_server_and_the_next_one_uses_it`.

#### Scenario: Versions
- **WHEN** a tilrd runs and the user runs `tilr version`
- **THEN** the first line starts with `tilr ` and the second with `tilrd `

### Requirement: Server commands
`tilr token`, `tilr cloud login [--url URL] [--name NAME]|status|logout` and `tilr update [--cdn URL]` SHALL run in the local tilrd (started if needed) as `ExecStart` with `server_command = true` and `argv` = the subcommand and its options. Output SHALL be shown as it arrives (`ExecOutput.stream` 2 on stderr, others on stdout); Ctrl+C SHALL send `ExecCancel` once. With a `--connect` target they SHALL fail with `server commands run in the local tilrd; on another machine, run tilr there`.

**Verified by**: `client::server_commands_run_in_the_server` (token, cloud status, the `--connect` refusal).

#### Scenario: Token
- **WHEN** the user runs `tilr token`
- **THEN** stdout is exactly the contents of the state directory's `token` file

#### Scenario: Remote target refused
- **WHEN** the user runs `tilr --connect ws://127.0.0.1:9 token`
- **THEN** it exits non-zero

#### Scenario: Refused by the server
- **WHEN** `ExecStarted.error` is not empty
- **THEN** the command fails with that text

### Requirement: Server command exit status
On `ExecExit` the exit status SHALL be `code` when `signal` is 0 and `code` fits 0..255 (else 1), and 1 when `signal` is set. A non-empty `reason` SHALL fail with `the server ended the command: REASON`, except for `update`. What the commands do in the server is in the `access-control`, `cloud-tunnels` and `update` specs; which connections may run them in `exec`.

**Verified by**: `client::server_commands_run_in_the_server` (status 0 only).

#### Scenario: Update reloads the server
- **WHEN** `tilr update` ends with `ExecExit.reason` `reload`, or with code 0
- **THEN** the client reconnects every 100 ms for up to 15 s and prints `tilrd <version> is running`, or fails with `the server did not come back after the update: …`

#### Scenario: Ended by the server
- **WHEN** `tilr token` ends with `ExecExit.reason` `shutdown`
- **THEN** it fails with `the server ended the command: shutdown`

### Requirement: Attached terminal
Attaching SHALL need stdin to be a terminal, else fail with ``attaching needs a terminal (`tilr ls`, `tilr new -d` do not)``. Attached, the terminal SHALL be in raw mode on the alternate screen with line wrap off and bracketed paste on, restored on every way out. On each (re)connection the client SHALL send `Resize` with the terminal's size and then `Subscribe` for the pane, draw every `Frame` of that pane, and send `Resize` when the terminal is resized.

**Verified by**: `client::attached_in_a_terminal_it_types_reconnects_after_a_reload_and_detaches`, itests `UC-010` (`uc_010.py::tilr_without_a_running_daemon_starts_tilrd_detached_and_connects`).

#### Scenario: Shell in a pane
- **WHEN** no tilrd runs and the user starts `tilr` in a terminal
- **THEN** the terminal shows the shell prompt of a new pane, and typed input reaches it

### Requirement: Signals while attached
When an attached `tilr` gets SIGTERM, SIGHUP or SIGINT it SHALL restore the terminal (cooked mode, cursor shown, main screen, mouse capture and bracketed paste off) and then end by that signal, printing nothing; the pane goes on.

**Verified by**: `client::a_terminated_attach_restores_the_terminal`.

#### Scenario: SIGTERM while attached
- **WHEN** `kill -TERM` is sent to an attached `tilr`
- **THEN** `tilr` ends by SIGTERM, `stty -a` on its terminal shows `icanon` and `echo`, the cursor is shown, and the pane is still listed by `tilr ls`

### Requirement: How an attach ends
An attach SHALL end with one line on stdout and exit status 0: `[detached from pane XXXXXXXX]` (the first 8 characters of the id) on Ctrl+] d or end of input, `[pane XXXXXXXX exited with code N]` on `PaneExited`, `[pane XXXXXXXX is gone]` when a `PaneList` lacks the pane or it is gone after a reconnect, `[tilrd stopped]` when the local server stopped, and `[superseded by another connection of this client]` when the server closed the connection as superseded (4000).

**Verified by**: `client::attached_in_a_terminal_it_types_reconnects_after_a_reload_and_detaches`, itests `UC-016` (`uc_016.py::tilr_attaches_to_a_tilrd_running_in_the_foreground`), `UC-017` (`uc_017.py::kill_server_leaves_no_process_and_no_socket`), `client::an_attach_superseded_by_its_own_client_id_ends`.

#### Scenario: Detach
- **WHEN** the user presses Ctrl+] then d
- **THEN** `tilr` prints `[detached from pane …]`, exits 0 and the pane goes on

#### Scenario: Server stopped while attached
- **WHEN** the tilrd an attached `tilr` uses stops on SIGTERM
- **THEN** `tilr` prints `[tilrd stopped]` and exits 0

### Requirement: Prefix key
Ctrl+] (also read as the control code 0x1D) SHALL be the prefix: prefix then `d` (Shift allowed) SHALL detach, prefix twice SHALL send Ctrl+] itself, and prefix then any other key SHALL send that key to the pane.

**Verified by**: unit test `tests::ctrl_bracket_d_detaches_and_twice_sends_it` (crates/tilr/src/attach.rs).

#### Scenario: Prefix twice
- **WHEN** the user presses Ctrl+] Ctrl+]
- **THEN** one `KeyInput` for Ctrl+] goes to the pane and the client stays attached

### Requirement: Keys, paste and mouse
Keys SHALL go as `KeyInput` (press or repeat; releases dropped). Enter, Tab, Backspace, Escape, the arrows, Home, End, PageUp, PageDown, Insert, Delete, Space and F1…F12 SHALL carry `named` and `code` (`Enter`, `ArrowUp`, `F1`, …). A character key SHALL carry `codepoint` (lowercase for letters, Shift added for uppercase) and `text`, `code` empty, `text` empty with Ctrl. A paste SHALL go as `Paste`. Mouse input SHALL go as `MouseInput` only while the latest `Frame.mouse_tracking` is true.

**Verified by**: unit test `tests::named_keys_carry_their_physical_key` (crates/tilr/src/attach.rs).

#### Scenario: Russian layout Ctrl+C
- **WHEN** the terminal reports Ctrl+С on a Russian layout as ^C
- **THEN** the `KeyInput` carries Ctrl and codepoint `c`, with `code` and `text` empty

#### Scenario: Control codes read as digits
- **WHEN** the terminal sends 0x1C..0x1F, read as Ctrl+4..7
- **THEN** they go as Ctrl+\, Ctrl+], Ctrl+^ and Ctrl+_

#### Scenario: Mouse capture follows the program
- **WHEN** a `Frame` with `mouse_tracking` true arrives, and later one with it false
- **THEN** the client turns the terminal's mouse capture on, then off again

### Requirement: Reconnect while attached
An attached client SHALL detect lost connections and reconnect with the same `client_id`, retaining admitted input until acknowledged. It SHALL reconcile `HelloAck` counters and replay unacknowledged input in order. The delivery queue SHALL hold at most 4 MiB or 4096 messages; overflow SHALL visibly reject new input without discarding admitted input or advancing its sequence.

**Verified by**: `client::attach_replays_a_key_when_the_connection_ends_without_an_ack`, `client::attached_in_a_terminal_it_types_reconnects_after_a_reload_and_detaches`, itests `UC-014` (`uc_014.py::after_a_sigkill_the_next_tilr_replaces_the_socket_and_starts_a_new_daemon`).

#### Scenario: Connection loss and retry
- **WHEN** no message (including pongs) arrives for 15 s, a send fails, or a read fails
- **THEN** the client shows `tilr: disconnected (<why>), reconnecting… (Ctrl+] d detaches)` in reverse video on the bottom row and retries after 0.1 s, doubling waits up to 2 s
- **AND** it retains keys typed while disconnected except local prefix commands; a 4000 close ends the attach
- **AND** while connected it pings every 5 s

#### Scenario: Delivery queue overflow
- **WHEN** a key would exceed the delivery queue limit
- **THEN** the client displays `input buffer full`, preserves all admitted input, and does not advance the rejected input's sequence

#### Scenario: Reload of the server
- **WHEN** the server reloads while `tilr` is attached
- **THEN** `tilr` reconnects, checks `ListPanes` for its pane, subscribes again, and admitted input typed before, during and after the disconnection reaches the pane without loss and in order; replay may duplicate input applied but not acknowledged before reload

#### Scenario: SIGKILLed server
- **WHEN** the local tilrd is killed with SIGKILL, so its socket refuses and its lock is free
- **THEN** the attached `tilr` prints `[tilrd stopped]` and exits 0

#### Scenario: Failed attempt
- **WHEN** a reconnect attempt fails for another reason
- **THEN** the status line shows the error and the attempt number, and the next attempt waits twice as long, at most 2 s

### Requirement: Reconnect attempts
A reconnect attempt that connects but then fails to list the panes or to subscribe SHALL count as a failed attempt (status line, backoff), not end the attach. Keys SHALL be read while an attempt runs, so Ctrl+] d MUST detach at once also then.

**Verified by**: `client::an_error_right_after_a_reconnect_is_retried`, `client::detach_works_while_a_reconnect_attempt_hangs`.

#### Scenario: Error right after a reconnect
- **WHEN** a reconnect attempt connects and the connection is lost before `ListPanes` is answered
- **THEN** `tilr` stays attached, shows the error on the status line and connects again, and keys typed once it is back reach the pane

#### Scenario: Detach while an attempt hangs
- **WHEN** a reconnect attempt is waiting on a socket that accepts but never answers and the user types Ctrl+] d
- **THEN** `tilr` prints `[detached from pane …]` and exits 0 at once, not after the 5 s connect timeout (the test allows 1 s)

### Requirement: Errors and exit status
Every failure SHALL print one message `tilr: <error with its causes>` on stderr and exit with status 1, except server commands, whose status comes from `ExecExit`, and usage errors, which are clap's (status 2). Successful commands SHALL exit 0.

**Verified by**: `client::a_file_in_the_way_is_left_alone_with_a_clear_error`, `client::a_socket_path_too_long_is_a_clear_error`.

#### Scenario: A file in the way
- **WHEN** a regular file sits at the socket path and the user runs `tilr new -d`
- **THEN** it exits non-zero, stderr says `not a socket (nothing was removed)` and the file is unchanged

### Requirement: Service install with systemd
On Linux, when `systemctl --user show-environment` succeeds and lingering is on (`loginctl show-user USER --property=Linger --value` says `yes`) or `loginctl enable-linger USER` succeeds, `tilr service install` SHALL write the user unit `tilr.service` in `$XDG_CONFIG_HOME/systemd/user` (else `~/.config/systemd/user`), run `systemctl --user daemon-reload` and `systemctl --user enable --now tilr.service` (without `--now` when a tilrd already serves the socket), say which way was chosen and exit 0.

**Verified by**: unit test `service::tests::the_unit_runs_tilrd_from_a_login_shell_and_tells_systemd_when_ready` (crates/tilr/src/service.rs), itests `UC-020` (`uc_020.py::service_install_writes_a_notify_user_unit_and_enables_it`, `uc_020.py::with_lingering_off_install_turns_it_on_and_notes_it`, `uc_020.py::a_running_tilrd_is_left_alone`).

#### Scenario: Unit content
- **WHEN** install writes the unit for tilrd at `/home/u/.local/bin/tilrd`
- **THEN** the unit starts with `# Written by `tilr service install`; `tilr service uninstall` removes it.` and has `Type=notify`, `ExecStart=/bin/sh -lc 'exec "/home/u/.local/bin/tilrd"'`, `Restart=on-failure`, `RestartSec=2` and `WantedBy=default.target`
- **AND** it is written aside and renamed into place, mode 0644

#### Scenario: Install with lingering off
- **WHEN** lingering is off and `loginctl enable-linger` succeeds
- **THEN** the unit ends with `# tilr service: lingering turned on by install` and `systemctl --user enable --now tilr.service` is run
- **AND** a later install keeps that note

#### Scenario: A tilrd already runs
- **WHEN** a tilrd started by `tilr` serves the socket
- **THEN** the unit is enabled without `--now` and the running tilrd keeps its pid and panes

### Requirement: Service paths
`tilr service install` SHALL use the tilrd found as the `daemon-lifecycle` spec says, resolved to its real path, and the socket `--socket` / `TILR_SOCKET`, else the default one. A tilrd or socket path containing `'`, `"`, `$`, `` ` ``, `\`, `%` or a newline, or not UTF-8, SHALL be refused, with nothing written.

**Verified by**: unit test `service::tests::paths_a_command_line_cannot_carry_are_refused`.

#### Scenario: Quote in the path
- **WHEN** tilrd is at `/home/o'neil/tilrd`
- **THEN** install fails saying the path has `'` in it, which a service's command line cannot carry

### Requirement: Service install with cron
On Linux without a systemd user manager, or when lingering can be neither found on nor turned on, `tilr service install` SHALL say why and put into the user's crontab, keeping every other line and replacing an earlier line of its own, `@reboot /bin/sh -lc 'mkdir -p -m 700 "<dir>" && exec "<tilrd>" --socket "<socket>" >>"<dir>/tilrd.log" 2>&1' # tilr service` (`<dir>` the socket's), print it, and start tilrd now when none runs (`tilrd started`).

**Verified by**: unit test `service::tests::the_cron_line_is_recognized_as_ours_and_others_are_not`, itests `UC-021` (`uc_021.py::without_lingering_a_crontab_reboot_line_is_written`, `uc_021.py::without_systemd_a_crontab_reboot_line_is_written_next_to_the_users_own`, `uc_021.py::with_neither_systemd_nor_crontab_install_fails_and_writes_nothing`).

#### Scenario: Next to the user's own line
- **WHEN** the user's crontab has a line of their own and there is no systemd, and install runs twice
- **THEN** the crontab holds the user's line and exactly one `# tilr service` line

#### Scenario: Socket under XDG_RUNTIME_DIR
- **WHEN** the socket is under `$XDG_RUNTIME_DIR`
- **THEN** install also warns that this directory is made at login and gone at boot, and to set `TILR_RUNTIME_DIR` and install again

#### Scenario: Neither systemd nor crontab
- **WHEN** neither `systemctl` nor `crontab` works
- **THEN** install exits non-zero with `no way to start tilrd at boot here: no systemd user service with lingering, and no crontab (…)`, writes nothing in the home and starts nothing

### Requirement: Service install on macOS
On macOS, `tilr service install` SHALL write `~/Library/LaunchAgents/sh.tilr.tilrd.plist` and, unless a tilrd already serves the socket, run `launchctl bootout gui/<uid>/sh.tilr.tilrd` and then `launchctl bootstrap gui/<uid> <plist>`, failing on a bootstrap error.

**Verified by**: unit test `service::tests::the_plist_names_the_socket_and_keeps_tilrd_alive_on_failure`.

#### Scenario: Plist content
- **WHEN** install runs with socket `/tmp/s & t/tilrd.sock`
- **THEN** the plist has label `sh.tilr.tilrd`, `ProgramArguments` `/bin/sh -lc 'exec "<tilrd>" --socket "/tmp/s &amp; t/tilrd.sock"'`, `RunAtLoad` true, `KeepAlive` with `SuccessfulExit` false, and stdout and stderr in `~/Library/Logs/tilrd.log`

### Requirement: Service uninstall removes only its own
`tilr service uninstall` SHALL remove the unit (after `systemctl --user disable tilr.service`, then `daemon-reload`) or, on macOS, the plist, only when the file contains `tilr service`, else leave it with a message. It SHALL remove its `# tilr service` `@reboot` crontab line and keep the others, or the whole crontab (`crontab -r`) when nothing else is left. It SHALL stop nothing and exit 0.

**Verified by**: itests `UC-022` (`uc_022.py::uninstall_removes_the_unit_it_wrote_and_nothing_else`, `uc_022.py::uninstall_leaves_lingering_it_did_not_turn_on`, `uc_022.py::uninstall_removes_its_crontab_line_and_keeps_the_users`, `uc_022.py::uninstall_removes_the_crontab_it_made`, `uc_022.py::uninstall_leaves_what_it_did_not_write`).

#### Scenario: Lingering it turned on
- **WHEN** the removed unit carries the lingering note
- **THEN** uninstall runs `loginctl disable-linger USER`, or on failure says to run it with sudo; without the note lingering is left as it is

#### Scenario: A unit it did not write
- **WHEN** `~/.config/systemd/user/tilr.service` was written by hand, not by `tilr service install`
- **THEN** uninstall leaves it as it is

#### Scenario: Server goes on
- **WHEN** the service was installed and a tilrd runs with panes
- **THEN** after uninstall the same tilrd and its panes are alive, and it says the tilrd goes on until `tilr kill-server`; with nothing of its own found it says `no service was installed: nothing to remove`

### Requirement: Nothing starts tilrd at boot unless asked
No `tilr` command other than `tilr service install` SHALL write a systemd unit, crontab line, launchd agent, autostart entry, shell profile change or anything under `/etc`.

**Verified by**: itests `UC-023` (`uc_023.py::nothing_starts_tilrd_at_boot_unless_asked`).

#### Scenario: Ordinary use
- **WHEN** the user runs `tilr`, `tilr new -d`, `tilr ls`, `tilr kill-server` and the other commands
- **THEN** no unit, crontab or autostart entry exists afterwards and the profiles are unchanged

### Requirement: tilr checks who serves the socket
Before it connects to the default socket or starts a server for it, `tilr` SHALL refuse a runtime directory that is a symbolic link, is not a directory, is not owned by its effective uid or has permission bits for group or others, and connect to and start nothing. For any socket, before sending a byte, `tilr` MUST compare the peer's uid with its effective uid and, on a mismatch, close and fail with `socket <path> is served by another user (uid <N>)`.

**Verified by**: unit test `local::tests::a_runtime_directory_of_this_user_alone_is_trusted` (crates/tilr-proto/src/local.rs), `client::a_runtime_directory_open_to_others_or_a_link_is_refused_and_nothing_started`, itests `UC-03A` (`uc_03a.py::a_runtime_directory_another_user_made_in_tmp_is_not_entered`, `uc_03a.py::a_socket_another_user_serves_at_a_named_path_is_refused_before_a_byte`), `UC-034` (`uc_034.py::another_user_cannot_use_the_socket`).

#### Scenario: Another user's runtime directory
- **WHEN** `XDG_RUNTIME_DIR` is unset and another user made `/tmp/tilr-<uid>` and serves a socket in it
- **THEN** `tilr ls`, `tilr attach` and `tilr new -d` exit non-zero naming the directory and its owner's uid, nothing connects to that socket and no tilrd is started

#### Scenario: A runtime directory open to others or a link
- **WHEN** the runtime directory is mode 0755, or a symbolic link to a private directory
- **THEN** `tilr` refuses with `open to other users (mode 0755)` or `is a symbolic link` and starts no server

#### Scenario: A socket served by another user
- **WHEN** the socket path (`TILR_SOCKET`, `--socket` or `client.toml`'s `socket`) is served by a process of another uid
- **THEN** `tilr` prints `socket <path> is served by another user (uid <N>)` and exits non-zero without sending a byte (no WebSocket handshake, no `Hello`)

### Requirement: Service install leaves a unit or plist it did not write
`tilr service install` MUST NOT replace a `tilr.service` (Linux, systemd) or `sh.tilr.tilrd.plist` (macOS) without the marker `tilr service`: it SHALL exit non-zero with `<path> was not written by `tilr service install`: move it away to install`, leave the file as it is and tell the service manager nothing (only `systemctl --user show-environment` runs, to find the manager).

**Verified by**: unit test `service::tests::a_file_without_the_mark_is_not_replaced` (crates/tilr/src/service.rs), itests `UC-020` (`uc_020.py::install_leaves_a_unit_it_did_not_write`).

#### Scenario: A unit it did not write
- **WHEN** `~/.config/systemd/user/tilr.service` exists without `tilr service` in it and the user runs `tilr service install`
- **THEN** install exits non-zero, stderr names the file and says it was not written by `tilr service install`, the file is unchanged, the only call made is `systemctl --user show-environment`, and no crontab is written

#### Scenario: A plist it did not write
- **WHEN** `~/Library/LaunchAgents/sh.tilr.tilrd.plist` exists without `tilr service` in it
- **THEN** install exits non-zero naming it and the file is unchanged

### Requirement: A missing crontab is recognized whatever the cron
`tilr service install` and `uninstall` SHALL run `crontab` with `LC_ALL=C` and take a `crontab -l` that fails saying `no crontab for` (cronie, Vixie cron) or `No such file or directory` (busybox) as an empty crontab; any other failure SHALL still stop install.

**Verified by**: unit test `service::tests::a_missing_crontab_is_recognized_from_cron_and_busybox` (crates/tilr/src/service.rs).

#### Scenario: busybox crontab without a crontab
- **WHEN** `crontab -l` exits 1 saying `crontab: can't open '/var/spool/cron/crontabs/u': No such file or directory`
- **THEN** install takes the crontab as empty and writes its line

### Requirement: A superseded attach ends
When the server closes an attached `tilr`'s connection with close code 4000 (`superseded`), `tilr` SHALL NOT reconnect: it SHALL restore the terminal, print `[superseded by another connection of this client]` on stdout and exit with status 0.

**Verified by**: `client::an_attach_superseded_by_its_own_client_id_ends`.

#### Scenario: Another connection with the same client id
- **WHEN** `tilr` is attached and a second connection says `Hello` with `tilr`'s `client_id` and the same token on the same transport
- **THEN** `tilr` prints `[superseded by another connection of this client]` and exits 0 without connecting again
- **AND** the pane goes on

### Requirement: tilr deploy installs and starts tilrd over SSH
`tilr deploy [--service] [--archive FILE] [--cdn URL] <dest>` SHALL use the user's `ssh` to find the remote platform, send it the matching release archive checked on the local machine, install `tilrd` and `tilr` into the remote `~/.local/bin` both or neither, run `tilr service install` there with `--service`, start tilrd there, and print an `ssh -t <dest> '~/.local/bin/tilr'` line, an `ssh -N -L` line and the remote token; any failure SHALL exit non-zero leaving the remote files as they were.

**Verified by**: itests `UC-07C` (`uc_07c.py::deploy_installs_and_starts_tilrd_on_a_host_reached_over_ssh`, `uc_07c.py::a_failed_deploy_leaves_the_remote_as_it_was`) and `client::deploy_names_a_platform_without_a_release`, `client::deploy_through_a_stub_ssh_installs_both_and_prints_how_to_connect`, `client::deploy_rolls_back_both_programs_when_remote_start_or_service_fails`, `client::deploy_gets_the_token_before_replacing_an_old_protocol_client`, `client::deploy_restores_service_configuration_when_the_manager_refuses_it`, `client::deploy_restores_the_crontab_after_a_service_failure`, `client::deploy_does_not_start_a_daemon_before_a_failing_service_install` (crates/tilr/tests/client.rs).

#### Scenario: A fresh host
- **WHEN** the user runs `tilr deploy user@host` and host runs Linux x86_64 without tilr
- **THEN** `~/.local/bin/tilrd` and `tilr` on host are the release's, a tilrd runs there, and the output ends with the attach line, the forward line and the token

#### Scenario: Local and remote home directories differ
- **WHEN** the user runs the printed terminal command in a local shell whose home differs from the remote home
- **THEN** the command preserves `~/.local/bin/tilr` for expansion by the remote shell

#### Scenario: A damaged archive
- **WHEN** the archive given with `--archive` does not match its `.sha256`
- **THEN** nothing is sent to host and the command exits non-zero with `checksum mismatch`

#### Scenario: No release for the platform
- **WHEN** host runs a platform the CDN has no `latest` for and no `--archive` is given
- **THEN** the command exits non-zero naming the platform and `--archive`, and host is unchanged

#### Scenario: A server already running there
- **WHEN** tilrd already runs on host with a pane, and a newer release is deployed
- **THEN** that tilrd reloads into it and the pane's shell is the same process

#### Scenario: An upgrade changes the wire protocol
- **WHEN** the installed client can talk to the running server and the new release uses a different protocol
- **THEN** deploy obtains the persistent token through the installed client before replacing it, and succeeds without a new client handshake with the old server

#### Scenario: Service installation fails
- **WHEN** `--service` fails after writing its configuration
- **THEN** deploy restores the previous service configuration and both programs, exits non-zero and prints no connection commands

#### Scenario: Service failure on a fresh host
- **WHEN** no tilrd runs on the host and its service manager refuses installation
- **THEN** deploy fails without starting a daemon or creating its token, and restores both programs and service configuration

### Requirement: Internal release fetching for deploy
`tilrd fetch-release --platform P --out DIR [--cdn URL] [--archive FILE]` SHALL validate a release's checksum and both regular program members using the updater's checks, write its archive into DIR, and exit non-zero on failure. It SHALL remain hidden from help and SHALL NOT execute the programs. With `--archive`, it SHALL validate the local archive and adjacent `.sha256` without HTTP.

**Verified by**: `update::fetch_release_writes_a_checked_archive`, `update::fetch_release_refuses_a_bad_checksum` and `client::deploy_refuses_a_damaged_archive_before_sending_it`.

#### Scenario: Fetch a foreign platform
- **WHEN** a user fetches a valid release for a platform other than the local one
- **THEN** the checked archive is written without running either program

#### Scenario: Bad checksum
- **WHEN** a downloaded or supplied archive does not match its checksum
- **THEN** the command exits non-zero without writing a release archive

## Notes

- Partly untested: the connect/request timeouts and the protocol-mismatch message (Connecting and saying hello), non-zero codes and reasons (Server command exit status). Several requirements rest on unit tests in `crates/tilr/src/*.rs` (`tests::…`, `target::tests::…`, `service::tests::…`) rather than on `crates/tilr/tests/client.rs`.
- The "Verified by" line is written as an OpenSpec metadata line (`**Verified by**: …`) so that it does not count toward the 500-character requirement-text limit of `openspec validate --strict`.
