# panes Specification

## Purpose

Covers the life of a pane as clients see it: creating, listing, watching, resizing and closing panes (`CreatePane`, `ListPanes`, `Subscribe`, `Unsubscribe`, `Resize`, `ClosePane`, `PaneCreated`, `PaneList`, `PaneExited`), pane and server ids, the program a pane runs and its environment, the pane process per pane, and `tilrd ps`. What is inside a frame and how input is encoded lives in the `screen-frames` spec; the handshake in `protocol`; viewers' `client_id` and dropping dead clients in `client-liveness`; carrying panes across a reload in `reload`; `tilr ps` / `tilr ls` / `tilr attach` in `tilr-cli`.

## Requirements

### Requirement: Creating a pane
On `CreatePane` the server SHALL start a pane of `cols` x `rows` cells (each clamped to 1..65535) running `command` and, once the program runs, SHALL answer the asking client with `PaneCreated` carrying the pane's `PaneInfo` and the request's `request_id`, then push a `PaneList` to every client.

**Verified by**: `panes::a_pane_runs_commands_and_reports_how_its_shell_exited` and `panes::a_pane_without_a_command_runs_the_users_login_shell`.

#### Scenario: A pane is created and answered by request id
- **WHEN** a client sends `CreatePane{cols: 100, rows: 30, command: [], request_id: 6}`
- **THEN** it receives `PaneCreated` with `request_id` 6 and a `PaneInfo` whose `pane_id` is a UUID
- **AND** `ListPanes` then lists that pane with `cols` 100 and `rows` 30

#### Scenario: Commands run in the pane
- **WHEN** a client types `echo "sum is $((6 * 7))"` and Enter into a new pane it subscribed to
- **THEN** its frames show `sum is 42`

### Requirement: A pane that cannot start is reported to its creator
When a pane cannot be started (its process cannot be spawned, its program cannot be run, or its process dies during startup) the asking client SHALL get an `Error` whose message starts with `create pane: `, and no pane SHALL be listed for the request.

#### Scenario: A program that does not exist
- **WHEN** a client sends `CreatePane` with `command` `["/no/such/program"]`
- **THEN** it gets an `Error` starting with `create pane: ` and no `PaneCreated`

### Requirement: Pane ids are UUIDs that name one pane only
Every pane SHALL be named by a random UUID (v4) made when it is created, never reused, the same in `PaneInfo.pane_id`, `Frame.pane_id`, `PaneExited.pane_id`, in the pane's environment as `TILR_PANE` and in the pane process's command line. A name that does not parse as the UUID of a running pane SHALL name no pane.

**Verified by**: `panes::a_pane_is_told_apart_by_its_id_in_its_shell_and_in_ps`.

#### Scenario: Two panes get different ids
- **WHEN** a client creates two panes
- **THEN** their `pane_id`s differ

#### Scenario: The shell knows its pane
- **WHEN** `test "$TILR_PANE" = <pane_id>` runs in the pane
- **THEN** it exits 0

### Requirement: Naming a pane that does not exist is an error, not a disconnect
`Subscribe`, `Resize`, `ClosePane`, `KeyInput`, `MouseInput`, `Paste`, `RawInput` and `Scroll` naming no running pane SHALL be answered with `Error{message: "no such pane: <name as sent>"}`, the connection staying open; `Unsubscribe` of an unknown pane SHALL be ignored.

**Verified by**: `panes::asking_for_a_pane_that_does_not_exist_gets_an_error`.

#### Scenario: Unknown UUID
- **WHEN** a client subscribes to a random UUID no pane has
- **THEN** it gets `Error` `no such pane: <that uuid>`
- **AND** typing into that id gets the same error

#### Scenario: A name that is not a UUID
- **WHEN** a client sends `Subscribe{pane_id: "1"}`
- **THEN** it gets `Error` `no such pane: 1`

### Requirement: A pane without a command runs the user's login shell
A pane whose `CreatePane.command` is empty SHALL run the user's shell from the password database, else `$SHELL` (if not empty), else `/bin/sh`, as a login shell: `argv[0]` is the shell's file name with a leading `-`.

**Verified by**: `panes::a_pane_without_a_command_runs_the_users_login_shell` and unit tests `pty::tests::a_login_shell_is_started_under_its_name_with_a_dash`, `pty::tests::the_login_shell_comes_from_the_password_database_first` (crates/tilrd/src/pty.rs).

#### Scenario: Login shell name
- **WHEN** a pane is created with no command and `case "$0" in "-${SHELL##*/}") true;; *) false;; esac` runs in it
- **THEN** it exits 0

#### Scenario: Shell from the password database wins
- **WHEN** the password database names `/bin/zsh` and `$SHELL` is something else
- **THEN** the pane runs `/bin/zsh` as `-zsh`

#### Scenario: Names of other shells
- **WHEN** the login shell is `/opt/homebrew/bin/fish` or `sh`
- **THEN** it is started as `-fish` or `-sh`

### Requirement: A pane with a command runs it directly
A pane whose `CreatePane.command` is not empty SHALL run `command[0]` with the rest as its arguments, without a shell. Whatever it runs, the program SHALL be a session leader with the pane's pty as its controlling terminal and as its stdin, stdout and stderr.

**Verified by**: `panes::a_pane_is_told_apart_by_its_id_in_its_shell_and_in_ps` and `panes::a_pane_runs_commands_and_reports_how_its_shell_exited`.

#### Scenario: /bin/sh as the command
- **WHEN** a client creates a pane with `command` `["/bin/sh"]`
- **THEN** `/bin/sh` runs as the pane process's child and reads what the client types

### Requirement: Terminal variables of a pane
The program in a pane SHALL get the server's environment with `TERM=xterm-256color`, `COLORTERM=truecolor`, `SHELL` set to the login shell's path (also in a pane with a command) and `TILR_PANE` set to the pane id.

**Verified by**: `panes::a_pane_without_a_command_runs_the_users_login_shell` and `panes::a_pane_is_told_apart_by_its_id_in_its_shell_and_in_ps`.

#### Scenario: The shell is named
- **WHEN** `test -x "$SHELL"` runs in a login-shell pane
- **THEN** it exits 0

### Requirement: User variables of a pane
`HOME`, `USER` and `LOGNAME` SHALL come from the password database when the server's own value is unset or empty, and pass through otherwise; `XPC_SERVICE_NAME`, `XPC_FLAGS` and the service manager's `NOTIFY_SOCKET`, `LISTEN_FDS`, `LISTEN_FDNAMES`, `LISTEN_PID`, `WATCHDOG_USEC` and `WATCHDOG_PID` SHALL be removed. When the server has tools for the panes it SHALL put their directory first on `PATH` and name it in `TILR_SHIMS` (see `clipboard-host-events`).

**Verified by**: `panes::a_pane_without_a_command_runs_the_users_login_shell` and `panes::a_pane_does_not_get_the_service_managers_variables`.

#### Scenario: Who and where
- **WHEN** `test "$HOME" = <home> && test -n "$USER" && test "$LOGNAME" = "$USER"` runs in a login-shell pane
- **THEN** it exits 0

#### Scenario: The tools survive the login profile
- **WHEN** the login profile has run in a new pane
- **THEN** `$TILR_SHIMS` is not empty and is one of the directories on `$PATH`

#### Scenario: Server under a notify unit
- **WHEN** the server runs with `NOTIFY_SOCKET`, `LISTEN_FDS` and `WATCHDOG_USEC` set and a pane runs `env`
- **THEN** none of them is in the output

### Requirement: The directory a pane starts in
A pane SHALL start in `CreatePane.cwd` when it names an existing directory on the server's machine, and in the user's home directory when `cwd` is empty or is not a directory there; `PaneInfo.cwd` SHALL be that directory.

**Verified by**: `panes::a_pane_starts_in_the_home_directory_unless_given_one_that_exists`.

#### Scenario: A directory from another machine
- **WHEN** a client creates a pane with `cwd` `/no/such/directory`
- **THEN** the shell runs in the home directory and `PaneInfo.cwd` is the home directory

#### Scenario: An existing directory
- **WHEN** a client creates a pane with `cwd` naming an existing directory
- **THEN** the shell runs there and `PaneInfo.cwd` is that directory

### Requirement: The shell reports its directory and title
When the program reports a directory with OSC 7 (`file://host/path`, percent-decoded) it SHALL replace `PaneInfo.cwd`; when it sets the title (OSC 0/2) it SHALL become `PaneInfo.title`. Either change SHALL push a `PaneList`.

**Verified by**: `panes::the_first_thing_a_client_hears_is_the_answer_to_its_hello` and unit test `ghostty::tests::status_reports_title_and_cwd` (crates/tilr-screen/src/ghostty.rs).

#### Scenario: A new directory
- **WHEN** the program prints `ESC ] 7 ; file://host/tmp/a%20b BEL`
- **THEN** the pane's `PaneInfo.cwd` becomes `/tmp/a b`

#### Scenario: A new title
- **WHEN** the program prints `ESC ] 2 ; new title BEL`
- **THEN** greeted clients get a pushed `PaneList` where that pane's `title` is `new title`

### Requirement: Listing panes
`ListPanes` SHALL be answered with `PaneList` carrying the request's `request_id` and every running pane's `PaneInfo`, oldest first (`created_at_ms`, then `pane_id`): `pane_id`, `cols`, `rows`, `command` as created, `title`, `cwd`, `viewers`, `viewed_by`, `created_at_ms`, `used_at_ms` and `pid`.

**Verified by**: `panes::a_pane_runs_commands_and_reports_how_its_shell_exited`.

#### Scenario: An empty server
- **WHEN** a client sends `ListPanes` to a server with no panes
- **THEN** the `PaneList` it gets has no panes

#### Scenario: One pane
- **WHEN** a client created a 100 x 30 pane and subscribed to it
- **THEN** `ListPanes` lists it with its `pane_id`, `viewers` 1, `cols` 100 and `rows` 30

### Requirement: Pane changes are pushed
The server SHALL push a `PaneList` with `request_id` 0 to every client that has completed its hello whenever a pane is created or goes away, a client subscribes, unsubscribes or leaves, a pane is resized, or a pane's title or cwd changes; a client that has not sent `Hello` yet SHALL get none (see `protocol`).

**Verified by**: `panes::a_client_that_leaves_does_not_take_the_pane_with_it` and `panes::the_first_thing_a_client_hears_is_the_answer_to_its_hello`.

#### Scenario: A late client
- **WHEN** panes change before a connected client says hello
- **THEN** its first message is `HelloAck`, and after it a pane's exit pushes it an empty `PaneList`

### Requirement: When a pane was used last
`PaneInfo.used_at_ms` SHALL be the Unix time in milliseconds when a client last subscribed to the pane or sent it key, mouse, paste, raw or scroll input, and `created_at_ms` until then. A change of `used_at_ms` alone SHALL NOT push a `PaneList`.

#### Scenario: Input marks the pane used
- **WHEN** a client sends `KeyInput` to a pane and then sends `ListPanes`
- **THEN** that pane's `used_at_ms` is not earlier than the time the key was sent

### Requirement: Watching a pane
`Subscribe` SHALL add the connection to the pane's viewers and be followed by a full frame (see `screen-frames`); a viewer SHALL get the pane's frames until it unsubscribes, the pane exits or the connection ends. `PaneInfo.viewers` SHALL count the subscribed connections and `viewed_by` list one `Viewer` per connection (see `client-liveness`). The first frame delivered after each `Subscribe` SHALL be full.

**Verified by**: `reconnect::subscribing_again_sends_the_whole_screen_again`, `reconnect::a_client_back_on_a_new_connection_supersedes_its_old_one_at_once` and `supervisor::tests::a_subscriber_gets_full_before_delta_and_keeps_its_own_input_echo`.

#### Scenario: Subscribe while delta is in flight
- **GIVEN** a pane with an existing viewer and an in-flight delta
- **WHEN** another connection subscribes before that delta reaches the supervisor
- **THEN** the existing viewer receives the delta, the new viewer receives no delta before a full frame, and both receive subsequent frames normally with existing backpressure

#### Scenario: Own input echoed before the initial full frame
- **WHEN** delta skipped after Subscribe acknowledges the subscriber's own input, followed by a full frame acknowledging another client's input
- **THEN** the subscriber receives the full frame with its latest skipped own input sequence

#### Scenario: Subscribe again while recovering
- **WHEN** a connection subscribes again before receiving the full frame that ends its recovery
- **THEN** its recorded own input acknowledgement is preserved until the full frame arrives

#### Scenario: Subscribing again
- **WHEN** a client that watches a pane subscribes to it again
- **THEN** it gets a full frame of the pane

### Requirement: Several clients share a pane
Any number of clients SHALL be able to watch and type into the same pane at once, each seeing the output of everyone's input.

**Verified by**: `panes::two_clients_watch_and_type_into_the_same_pane`.

#### Scenario: Two clients type
- **WHEN** two clients subscribe to one pane and each types an `echo` line
- **THEN** both see both outputs and `PaneInfo.viewers` is 2

### Requirement: Unsubscribing stops the frames
After `Unsubscribe` the client SHALL get no more frames of that pane and SHALL no longer count as its viewer.

**Verified by**: `panes::a_client_that_stops_watching_gets_no_more_frames`.

#### Scenario: Output after unsubscribing
- **WHEN** a client unsubscribes and output is then printed in the pane
- **THEN** that client gets no frame of the pane within 500 ms and `viewers` is 0

### Requirement: Panes outlive their clients
A pane SHALL keep running, with its screen and history, when its viewers disconnect or are dropped; a client that leaves SHALL stop counting as a viewer, the others being told by a pushed `PaneList`. A client that subscribes later SHALL see what was printed meanwhile.

**Verified by**: `panes::a_client_that_leaves_does_not_take_the_pane_with_it`, `reconnect::output_printed_while_nobody_watched_is_there_on_reconnect` and `reconnect::many_reconnects_leave_no_viewers_behind`.

#### Scenario: One of two viewers leaves
- **WHEN** one of two subscribed clients closes its connection
- **THEN** the other gets a pushed `PaneList` with `viewers` 1 and the shell still runs commands

#### Scenario: Output while nobody watched
- **WHEN** a command prints while no client is connected and a client then subscribes
- **THEN** its first frame shows that output

### Requirement: Resizing a pane
`Resize{pane_id, cols, rows, ...}` SHALL set the pane's size (`cols`, `rows` each clamped to 1..65535) on the pty, so the program sees it (`SIGWINCH`, `stty size`), update `PaneInfo.cols`/`rows` and push a `PaneList`; when the size or the cell changed, the next frame SHALL be full at the new size. The cell fields are in `screen-frames`.

**Verified by**: `panes::resizing_a_pane_resizes_the_terminal_the_shell_sees`.

#### Scenario: The shell sees the new size
- **WHEN** a client resizes a pane to 50 x 10
- **THEN** it gets a full frame with `cols` 50 and `rows` 10, `stty size` in the pane prints `10 50`, and `PaneInfo` says 50 x 10

### Requirement: Closing a pane ends its program
`ClosePane` SHALL send SIGHUP to the process group of the pane's program and to the pty's foreground process group and, if they have not emptied 2 s later, SIGKILL both; the pane then exits as any pane does.

**Verified by**: `panes::closing_a_pane_ends_its_shell`, `panes::closing_a_pane_ends_a_foreground_program_ignoring_sighup`.

#### Scenario: Close
- **WHEN** a client sends `ClosePane` for a pane running a shell
- **THEN** it gets `PaneExited` for that pane and `ListPanes` no longer lists it

#### Scenario: Close with a foreground job ignoring SIGHUP
- **WHEN** a pane's shell runs `sh -c 'trap "" HUP; sleep 1000'` in the foreground and a client sends `ClosePane`
- **THEN** within 3 s the client gets `PaneExited` and the `sleep` is no longer running

### Requirement: Reporting how a pane's program exited
When the pane's program ends, the pane SHALL send a last frame, then the server SHALL send `PaneExited{pane_id, exit_code}` to every connected client, remove the pane and push a `PaneList`. `exit_code` SHALL be the exit status, or -1 when the program was killed by a signal, had not exited 2 s after its pty closed (it is then killed), or the pane process died.

**Verified by**: `panes::a_pane_runs_commands_and_reports_how_its_shell_exited`.

#### Scenario: exit 3
- **WHEN** `exit 3` is typed into a pane's shell
- **THEN** the client gets `PaneExited` with that `pane_id` and `exit_code` 3
- **AND** `ListPanes` returns no panes

### Requirement: The pane process shows which pane it is
Each pane SHALL be its own process, a child of the supervisor, started as `tilrd pane --id <pane_id>` followed by `--cols N --rows N` only when the size is not 80 x 24, `--cwd <dir>` when there is one, and `-- <command...>` when a command was given. It SHALL keep these start arguments for its whole life, reloads included: a reload SHALL exec the successor with them, whatever its process title shows, and its visible command line (`ps -o args`, `/proc/<pid>/cmdline`) SHALL start with them.

**Verified by**: `panes::a_pane_is_told_apart_by_its_id_in_its_shell_and_in_ps`.

#### Scenario: Default size is left out
- **WHEN** a client creates an 80 x 24 pane and a 100 x 30 pane running `/bin/sh` in the home directory
- **THEN** their pane processes' command lines after `pane` start with `--id <id1> --cwd <home> -- /bin/sh` and `--id <id2> --cols 100 --rows 30 --cwd <home> -- /bin/sh`

#### Scenario: Start arguments survive a reload
- **WHEN** a pane started with `--cols 100 --rows 30` has shown a title and the server reloads
- **THEN** the new pane process image's command line still starts with `--id <id> --cols 100 --rows 30`, and the pane keeps its shell

### Requirement: The pane process owns the pty
The pane process SHALL run in a session of its own, own the pty and the screen model, and have the pane's program as its child; `PaneInfo.pid` SHALL be its pid. The supervisor SHALL route packets between clients and pane processes; when its pipe to a pane process closes, the pane process SHALL exit, hanging up its program.

**Verified by**: `reconnect::a_server_started_from_a_terminal_outlives_the_terminal_hanging_up`.

#### Scenario: The server's terminal hangs up
- **WHEN** the terminal a server was started from hangs up and its process group gets SIGHUP
- **THEN** the pane processes and their shells live on, and a client reaches the same pane

### Requirement: tilrd ps lists the panes of a running server
`tilrd ps` (hidden; users run `tilr ps`, see `tilr-cli`) SHALL ask the server on its Unix socket, or at `--url ws://HOST:PORT` with the token from the state directory, and print the header `PANE PID SIZE VIEWERS CWD  COMMAND` and one line per pane, oldest first: id, pane process pid, `<cols>x<rows>`, viewers, cwd, the command joined by spaces or `(shell)`; it SHALL exit 0.

**Verified by**: `panes::ps_lists_the_panes_of_a_running_server`.

#### Scenario: One pane
- **WHEN** one 100 x 30 pane runs `/bin/sh` and `tilrd ps --url <server>` runs
- **THEN** it prints two lines, the second starting with the pane id, the pane process pid and `100x30` and ending with `/bin/sh`

### Requirement: tilrd ps can tell the server's version
`tilrd ps --server-version` SHALL print the server's `HelloAck.server_version` instead of the table. A server that does not answer within 5 s, or answers with an `Error`, SHALL make `tilrd ps` fail.

**Verified by**: `panes::ps_lists_the_panes_of_a_running_server`.

#### Scenario: Server version
- **WHEN** `tilrd ps --url <server> --server-version` runs
- **THEN** it prints the server's version and exits 0

### Requirement: The server has a lasting id
The server SHALL send its id in `HelloAck.server_id`: a UUID made when it first runs and kept in `server-id` (mode 0600) in the state directory (`$TILR_STATE_DIR`, else `$XDG_STATE_HOME/tilr`, else `~/.local/state/tilr`, made 0700), the same after a restart; another state directory SHALL have another id. If the file cannot be kept, the server SHALL warn and use a fresh id.

**Verified by**: `panes::a_server_keeps_its_id_when_it_restarts`.

#### Scenario: Restart
- **WHEN** the server is killed and started again with the same state directory
- **THEN** `HelloAck.server_id` is the same UUID as before

#### Scenario: Another machine
- **WHEN** a server runs with another state directory
- **THEN** its `server_id` differs

### Requirement: The tools stay first on PATH after the login profile
When a pane runs the user's login shell, that shell is zsh, bash or fish, and the server has tools for the panes, the first command at the shell's first prompt SHALL see `$TILR_SHIMS` as the first directory of `PATH` and nowhere else on it, even when the startup files set `PATH` from scratch; the shell SHALL still read the user's startup files a login shell of its kind reads and SHALL still run as `-<name>`.

**Verified by**: itests `UC-07B` (`uc_07b.py::the_tools_stay_first_on_path_when_the_profile_resets_it`, `uc_07b.py::the_users_startup_files_still_run`).

#### Scenario: bash with a profile that resets PATH
- **WHEN** the login shell is bash and `~/.bash_profile` says `PATH=/usr/bin:/bin`
- **THEN** `echo "$PATH"` in a new pane prints `$TILR_SHIMS:/usr/bin:/bin`

#### Scenario: zsh and fish
- **WHEN** the login shell is zsh with `path=(/usr/bin /bin)` in `~/.zprofile`, or fish with `set -gx PATH /usr/bin /bin` in `config.fish`
- **THEN** `$TILR_SHIMS` is the first directory of `PATH` in a new pane

#### Scenario: The user's files still run
- **WHEN** `~/.zshrc`, `~/.bash_profile` or `config.fish` sets a variable `X=1`, and zsh's `ZDOTDIR` is the user's own
- **THEN** `echo $X` prints `1`, `$0` is `-zsh`, `-bash` or `-fish`, and in zsh `$ZDOTDIR` is the user's value or unset as it was

#### Scenario: Another shell
- **WHEN** the login shell is `/bin/sh` (dash) and `~/.profile` resets `PATH`
- **THEN** the pane starts as before and `$TILR_SHIMS` is still set for the user's own `PATH="$TILR_SHIMS:$PATH"`

### Requirement: The shell wrappers survive another server starting
The wrappers in `<runtime dir>/shell/` are shared by all servers of the user. A starting server SHALL write a wrapper only when its content differs, whole (through a temporary file of its own, renamed over it), and two servers starting at once MUST both succeed, so neither removes the wrappers the other's panes start through.

**Verified by**: unit test `shell::tests::servers_starting_at_once_each_install_the_wrappers` (crates/tilrd/src/shell.rs).

#### Scenario: Servers starting at once
- **WHEN** four installs of the wrappers run at once, 100 times each, with an older copy put back before each
- **THEN** every install succeeds and the directory holds the current wrapper and nothing beside it

### Requirement: The pane process title follows the pane
After its start arguments, a pane process SHALL show ` [shell <shell pid> in <cwd>]`, or ` [shell <shell pid> in <cwd>: <command>]` while another process group is in the foreground of its pty (`<cwd>` is `PaneInfo.cwd`, `<command>` the group leader's command line, its process name when the command line cannot be read, or `?` when the leader is gone; at most 80 bytes). It SHALL update the title within a second of either changing and SHALL set it again after a reload.

**Verified by**: `panes::the_pane_process_title_follows_the_directory_and_the_command`, `panes::the_pane_process_title_follows_exec_in_the_same_foreground_group` and `reload::the_pane_process_title_is_set_again_after_a_reload`.

#### Scenario: cd in a pane
- **WHEN** the shell in a pane runs `cd /tmp`
- **THEN** within a second `ps -o args= -p <pane process pid>` ends with `[shell <shell pid> in /tmp]`

#### Scenario: A command in the foreground
- **WHEN** the shell runs `sleep 30` in the foreground
- **THEN** within a second the title ends with `[shell <shell pid> in <cwd>: sleep 30]`
- **AND** after `sleep` is interrupted with Ctrl+C the title ends with `[shell <shell pid> in <cwd>]` again

#### Scenario: After a reload
- **WHEN** the shell is in `/tmp` and the server reloads
- **THEN** the new pane process image's title ends with `[shell <same shell pid> in /tmp]`

#### Scenario: The foreground leader execs another command
- **WHEN** a foreground wrapper waits on a test-controlled FIFO and then execs `sleep 30` without changing its pid or process group
- **THEN** the title first shows the wrapper's command line and within a second of exec shows `sleep 30`, even without pty output

### Requirement: A title that does not fit is cut
A title longer than the space the pane process has for it SHALL be cut at the end of that space; when no title can be set at all, the command line SHALL stay the start arguments and the server SHALL log `pane title: no room` once.

**Verified by**: unit test `title::tests::a_title_is_cut_to_the_area` (crates/tilrd/src/title.rs).

#### Scenario: A very long directory
- **WHEN** the shell enters a directory whose path is longer than the title space
- **THEN** the command line starts with the start arguments and ends where the space ends, and the pane works as before

### Requirement: Making room for a title preserves the environment
Moving the environment to make room for the title SHALL preserve every environment entry and the pane's original arguments.

**Verified by**: unit tests `title::tests::the_environment_is_unchanged_after_the_move` and `title::tests::a_moved_environment_entry_keeps_the_contiguous_prefix`.

#### Scenario: A title overwrites the original environment strings
- **WHEN** the title is written over the original argv and environment string area
- **THEN** the copied environment entries and runtime arguments retain their original values

#### Scenario: One environment entry has moved
- **WHEN** an environment entry points outside the original string area
- **THEN** the title area includes argv and the environment strings contiguous with it up to the first gap
- **AND** every environment entry and original argument is preserved, including entries after the gap

### Requirement: A bell in a pane is reported
When a pane's program rings the bell (BEL in its output, outside an escape sequence), the server SHALL send `PaneActivity { pane_id, kind: BELL, at_ms }` to every greeted client, whether or not it watches the pane, and at most one per pane per second.

**Verified by**: `panes::a_bell_in_an_unwatched_pane_is_reported`, `panes::bells_are_reported_once_a_second` and `panes::a_bel_ending_an_osc_is_not_reported`.

#### Scenario: Bell in a pane nobody watches
- **WHEN** a client is greeted but not subscribed to a pane, and `printf '\a'` runs in that pane
- **THEN** the client receives `PaneActivity` with that pane's id and `kind` `BELL`

#### Scenario: Many bells
- **WHEN** a pane rings the bell ten times within 200 ms
- **THEN** a client receives one `BELL` for that pane in that second

#### Scenario: BEL ending an OSC
- **WHEN** a program sets the title with `ESC ] 2 ; t BEL`
- **THEN** no `PaneActivity` is sent

### Requirement: A finished foreground command is reported
When the pty's foreground process group of a pane goes from another group back to the shell's, the server SHALL send `PaneActivity { pane_id, kind: COMMAND_FINISHED, at_ms }` to every greeted client, once per such return; this SHALL also hold for a command that was running across a reload. A pane whose program does not use job control (a command run directly, a shell without job control) MAY report none.

**Verified by**: `panes::a_finished_foreground_command_is_reported`, `panes::typing_at_the_prompt_reports_nothing` and `reload::a_command_running_across_a_reload_is_reported_when_it_ends`.

#### Scenario: A command ends
- **WHEN** `sleep 1` runs in the foreground of a pane's shell and ends
- **THEN** every greeted client receives `PaneActivity` with that pane's id and `kind` `COMMAND_FINISHED`

#### Scenario: Typing at the prompt
- **WHEN** a user types at the shell's prompt without running a command
- **THEN** no `COMMAND_FINISHED` is sent

#### Scenario: Across a reload
- **WHEN** `sleep 3` runs in the foreground, the server reloads, and the client reconnects
- **THEN** the client receives `COMMAND_FINISHED` for that pane when `sleep` ends

### Requirement: Applied resize dimensions precede delayed input acknowledgement
When a numbered Resize is applied while earlier numbered input is still waiting to be fully written to the PTY, the server SHALL update the pane's listed dimensions and push a PaneList without waiting for the cumulative input acknowledgement. It SHALL keep that acknowledgement behind the earlier PTY input.

**Verified by**: `input_delivery::a_numbered_resize_updates_the_pane_list_while_pty_input_waits`.

#### Scenario: Earlier PTY input is blocked
- **GIVEN** a pane program waiting on an external gate without reading its PTY
- **AND** numbered raw input larger than the PTY's capacity is pending
- **WHEN** a later numbered Resize is applied to 120 columns and 40 rows
- **THEN** a pushed PaneList and ListPanes report 120 columns and 40 rows before the gate opens
- **AND** reconnecting reports that neither the raw input nor the Resize has been cumulatively acknowledged
- **AND** after the program reads the pending bytes, the Resize is cumulatively acknowledged

## Notes

- Parts without a test: the -1 exit code cases, the `XPC_*` removal, `COLORTERM=truecolor`, the 1..65535 clamp, the `tilrd ps` 5 s timeout, the fallback to a fresh server id.
- Unit tests are cited as `module::tests::name` with their source file.
- UC-082 (a 65535 x 65535 `Resize` is clamped to 1..65535, the pane lives) is a planned itests case without a scenario. Stopping the server, and what it ends in a pane (UC-086), is in `daemon-lifecycle`.
