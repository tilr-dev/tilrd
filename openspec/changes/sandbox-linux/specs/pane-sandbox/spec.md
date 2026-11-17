## ADDED Requirements

### Requirement: A sandboxed profile starts a sandboxed pane
On a server that lists `sandbox` in `server.info.features`, `pane.create` with a profile that has `[sandbox]` SHALL start the pane with its program and every process it starts restricted to that table's grants, and `PaneInfo.sandboxed` SHALL be true. A pane without such a profile SHALL have `sandboxed` false and SHALL be unrestricted as before.

**Verified by**: not tested yet; the bus-level tests come with the pane methods (tasks 2.2).

#### Scenario: Sandboxed and not
- **WHEN** the owner creates one pane with a sandboxed profile and one without a profile
- **THEN** `pane.list` shows `sandboxed` true for the first and false for the second

### Requirement: The sandbox feature is offered where it can be enforced
`sandbox` SHALL be listed in `server.info.features` only on Linux on x86_64 or aarch64 with Landlock ABI 4 or later and seccomp, or on macOS when `/usr/bin/sandbox-exec` applies a probe profile to `/usr/bin/true` within 2 s. On Landlock ABI 4 or 5 a sandboxed pane SHALL start without signal scoping, the one thing weaker there, and the server SHALL log a warning naming the pane and the ABI.

**Verified by**: unit tests `sandbox::tests::a_kernel_without_signal_scoping_is_named_in_a_warning`, `sandbox::tests::linux::signals_follow_the_kernels_scoping`; over the bus not tested yet (tasks 2.1).

#### Scenario: No sandbox on this host
- **WHEN** the server runs with the failpoint `sandbox-unavailable`
- **THEN** `sandbox` is missing from `server.info.features` and a sandboxed profile is refused as the `pane-profiles` spec says

#### Scenario: Kernel without signal scoping
- **WHEN** a sandboxed pane is started for Landlock ABI 4
- **THEN** it starts, can signal a process of the user outside it, and the log has a line `sandbox: kernel lacks signal scoping (Landlock ABI 4 < 6)` naming the pane

### Requirement: Files outside the grants cannot be read or written
In a sandboxed pane and every process it starts, a path under `read_write` SHALL be readable, writable and executable; under `read_only` readable and executable, not writable. Any other file's contents MUST be neither readable nor writable and any other directory MUST NOT be listed, except the system directories, `/etc`, `/proc`, `/sys`, the pane's terminal devices and, with `client` or `server`, the file `/etc/resolv.conf` links to.

**Verified by**: unit tests `sandbox::tests::linux::a_sandboxed_program_writes_only_where_it_may`, `sandbox::tests::linux::a_child_of_the_sandboxed_program_is_restricted_too`; over the bus not tested yet (tasks 2.2).

#### Scenario: Write outside
- **WHEN** a profile grants `read_write = ["<w>"]`, `read_only = ["<r>"]`, and the pane runs `touch <w>/a <r>/b <o>/c` (another directory `<o>` of the user)
- **THEN** only `<w>/a` exists, and the pane shows permission errors for the other two

#### Scenario: A grandchild
- **WHEN** the pane runs `sh -c 'sh -c "touch <o>/d"'`
- **THEN** `<o>/d` does not exist

### Requirement: Names and metadata outside the grants are not hidden
A profile SHALL NOT be relied on to hide names or metadata (documented limits; contents stay closed): Landlock has no right for metadata, so a sandboxed pane can read (`stat`) the metadata of any path and change it (mode, group, times, extended attributes) on any file the user owns, the server's state directory and token included; and an inotify watch tells it the names made in a directory it cannot list.

**Verified by**: unit tests `sandbox::tests::linux::metadata_outside_the_grants_can_be_changed`, `sandbox::tests::linux::names_in_a_directory_not_granted_can_be_watched`.

#### Scenario: The token's mode
- **WHEN** the pane runs `chmod 0644 <state dir>/token` and `cat <state dir>/token`
- **THEN** the mode changes and `cat` fails with a permission error

#### Scenario: A watch on a directory not granted
- **WHEN** the pane cannot list `<o>`, sets an inotify watch on it, and a file `<o>/a-name` is made
- **THEN** the pane reads the event with the name `a-name`

### Requirement: A sandboxed pane reaches only its own terminal
By default a sandboxed pane SHALL be able to read, write and use its own terminal, as `/dev/pts/N` and as `/dev/tty`, and to open `/dev/ptmx`; it MUST NOT be able to open another terminal device of the user, so it can neither read what is typed there nor write to it. The slave of a terminal made inside the pane cannot be opened either, so `script` and tmux fail there (a documented limit).

**Verified by**: unit tests `sandbox::tests::linux::a_sandboxed_pane_cannot_open_another_terminal`, `sandbox::tests::linux::a_terminal_made_inside_a_sandbox_cannot_be_opened`.

#### Scenario: Another terminal of the user
- **WHEN** the user has another terminal `/dev/pts/M` open and the pane runs `: < /dev/pts/M` and `: > /dev/pts/M`
- **THEN** both fail with a permission error, and `echo x > $(tty)` and `echo x > /dev/tty` succeed

#### Scenario: A terminal made inside
- **WHEN** a program in the pane opens `/dev/ptmx` and then the slave of that terminal
- **THEN** `/dev/ptmx` opens and the slave fails with a permission error

### Requirement: A profile that grants the terminals is warned about
A profile MAY grant `/dev/pts` (or a path covering it) in `read_write`; its panes SHALL then make terminals inside (tmux, `script`) and SHALL also open every terminal of the same user, reading what is typed there and writing to it, so the sandbox no longer isolates terminals. For each such pane the server SHALL log a warning naming the grant and the pane.

**Verified by**: unit tests `sandbox::tests::linux::a_profile_granting_dev_pts_can_make_terminals`, `sandbox::tests::a_grant_of_the_users_terminals_is_warned_about`.

#### Scenario: `/dev/pts` granted
- **WHEN** a profile says `read_write = ["/dev/pts"]` and the pane runs `openpty` and writes to another terminal of the user
- **THEN** both succeed, the other terminal shows what was written, and the log has `sandbox: the grant /dev/pts covers /dev/pts` naming the pane

### Requirement: Only readable programs run
A sandboxed pane SHALL be able to run the programs under its grants, the `executables` (names looked up in the pane's `PATH` when it is created), the program the pane was started with and the system's programs, and MUST NOT be able to run a program it cannot read. A program it can read it can run another way (a copy in its `TMPDIR`, `memfd_create` and `execve`, `ld.so`): readable implies runnable.

**Verified by**: unit tests `sandbox::tests::linux::only_granted_programs_run_in_a_sandbox`, `sandbox::tests::linux::a_readable_program_runs_from_memory_too`.

#### Scenario: A program in the user's directory
- **WHEN** the user has an executable `<o>/tool` outside the grants and the pane runs `<o>/tool`
- **THEN** it fails with a permission error

#### Scenario: A readable program run from memory
- **WHEN** the pane copies a program under `read_only` into a `memfd_create` descriptor and execs that
- **THEN** the program runs

### Requirement: Programs made inside run inside
A script or binary created in a `read_write` directory of a sandboxed pane SHALL be executable inside that pane without any step outside it. The binary of a test may be a copy of a system program where the platform lets a copy run (Linux) or one the test builds inside the pane.

**Verified by**: unit test `sandbox::tests::linux::a_script_and_a_binary_made_inside_run_inside`; over the bus not tested yet (tasks 2.2).

#### Scenario: Script and compiled binary
- **WHEN** the pane writes `#!/bin/sh` + `echo ok` to `<w>/t.sh`, runs `chmod +x <w>/t.sh && <w>/t.sh`, and puts a binary printing its arguments at `<w>/e` (on Linux a copy of `/bin/echo`) and runs `<w>/e hi`
- **THEN** it prints `ok` and `hi`

### Requirement: Network follows the profile
With `network = "none"` a sandboxed pane MUST NOT open TCP connections nor send UDP, and on Linux every `socket()` but netlink is refused; with `"client"` it SHALL connect out over TCP and MUST NOT accept TCP connections nor `listen`, only TCP streams and datagrams of IPv4 and IPv6 are made, and UDP SHALL NOT be restricted (a documented limit on Linux); with `"server"` it SHALL do both.

**Verified by**: unit tests `sandbox::tests::linux::network_follows_the_request`, `sandbox::linux::tests::the_filter_refuses_what_the_sandbox_does_not_allow`; over the bus not tested yet (tasks 2.2).

#### Scenario: None
- **WHEN** a test listener runs on 127.0.0.1 and a `none` pane tries to connect to it
- **THEN** the connection fails and the listener sees nothing

#### Scenario: Client
- **WHEN** a `client` pane connects to that listener, then tries to listen on a TCP port and to open an SCTP stream
- **THEN** the connection succeeds, listening and the SCTP socket fail

#### Scenario: Server
- **WHEN** a `server` pane listens on a TCP port and the test connects to it
- **THEN** the test's connection is accepted

### Requirement: Only connected Unix socket pairs and routing netlink
On Linux a sandboxed pane MUST NOT make a Unix-domain socket other than a connected pair: `socket(AF_UNIX, …)` fails, and `socketpair` SHALL make only `AF_UNIX` streams and sequenced packets, never datagrams (an unbound datagram socket could send to any named or abstract one). Netlink sockets SHALL open only with `NETLINK_ROUTE`, which name lookups read.

**Verified by**: unit tests `sandbox::tests::linux::a_unix_datagram_socket_cannot_be_made_with_socketpair`, `sandbox::tests::linux::only_routing_netlink_opens_in_a_sandbox`, `sandbox::linux::tests::the_filter_refuses_what_the_sandbox_does_not_allow`.

#### Scenario: A datagram pair on Landlock ABI 4
- **WHEN** a pane built for Landlock ABI 4 asks `socketpair(AF_UNIX, SOCK_DGRAM)` and `socketpair(AF_UNIX, SOCK_STREAM)`
- **THEN** the first fails with `EACCES`, the second succeeds, and no named or abstract datagram socket of the user receives anything

#### Scenario: Netlink
- **WHEN** a `client` pane opens `NETLINK_ROUTE` and `NETLINK_SOCK_DIAG` sockets and calls `getaddrinfo` with `AI_ADDRCONFIG`
- **THEN** the route socket and the lookup succeed and the diag socket fails with `EACCES`

### Requirement: No namespaces in a sandboxed pane
On Linux `unshare` and `clone` with a namespace flag MUST fail with `EPERM` in a sandboxed pane, and `clone3` SHALL fail with `ENOSYS` (glibc then uses `clone`), as Docker and Flatpak do: a user namespace would give the pane capabilities of its own. Forks, threads and `subprocess` SHALL work; rootless podman, bubblewrap and Chrome's sandbox fail inside (a documented limit).

**Verified by**: unit tests `sandbox::tests::linux::no_namespace_can_be_made_in_a_sandbox`, `sandbox::linux::tests::the_filter_refuses_what_the_sandbox_does_not_allow`.

#### Scenario: A user namespace
- **WHEN** a program in the pane calls `unshare(CLONE_NEWUSER)`, then forks, starts a thread and runs `/bin/echo` with `subprocess`
- **THEN** `unshare` fails with `EPERM` and the rest succeeds

### Requirement: Kernel interfaces a pane does not need are refused
On Linux a sandboxed pane MUST NOT use io_uring (`EACCES`: its socket operations bypass the socket rules), the keyrings (`keyctl`, `add_key`, `request_key`), `bpf`, `perf_event_open`, `userfaultfd` or `setns` (`EPERM`). A syscall of another ABI SHALL fail: an x32 one with `ENOSYS`, a 32-bit one by killing the process.

**Verified by**: unit tests `sandbox::tests::linux::kernel_interfaces_a_pane_does_not_need_are_refused`, `sandbox::linux::tests::the_filter_refuses_what_the_sandbox_does_not_allow`, `sandbox::linux::tests::every_network_mode_has_a_filter_that_assembles`, `sandbox::linux::tests::the_filters_syscall_numbers_are_the_hosts`.

#### Scenario: io_uring and the keyrings
- **WHEN** a program in the pane calls `io_uring_setup` and `keyctl`
- **THEN** the first fails with `EACCES` and the second with `EPERM`

#### Scenario: Another ABI
- **WHEN** the filter of either architecture is run on a syscall of another `AUDIT_ARCH` and, for x86_64, on an x32 syscall number
- **THEN** it kills the process for the first and answers `ENOSYS` for the second

### Requirement: Nothing of the pane process reaches the program
A sandboxed pane's program SHALL inherit no descriptor of the pane process but its terminal on 0, 1 and 2: every descriptor above 2 MUST be closed at the exec, whether or not it was marked close-on-exec.

**Verified by**: unit test `sandbox::tests::linux::no_descriptor_of_the_pane_process_reaches_the_program`.

#### Scenario: A descriptor without close-on-exec
- **WHEN** the pane process holds descriptor 100 without close-on-exec and starts a sandboxed program
- **THEN** the program has no descriptor 100

### Requirement: A sandboxed pane is a normal terminal
A sandboxed pane SHALL behave as any pane for the terminal: raw and canonical mode (`stty`), full-screen programs, job control (`Ctrl+Z`, `bg`, `fg`, `jobs`), `Ctrl+C`, window size changes, and `pane.command_finished`.

**Verified by**: unit tests `sandbox::tests::linux::raw_mode_and_window_size_work_in_a_sandbox`, `sandbox::tests::linux::job_control_works_in_a_sandbox`, `sandbox::tests::linux::a_full_screen_editor_works_in_a_sandbox`; over the bus and `pane.command_finished` not tested yet (tasks 2.3).

#### Scenario: Raw mode
- **WHEN** the pane runs `stty raw -echo; dd bs=1 count=1 2>/dev/null | od -c; stty sane` and the client inputs `x`
- **THEN** `od` prints `x` without Enter having been sent, and `stty` reported no error

#### Scenario: Job control
- **WHEN** the shell runs a program, the client sends Ctrl+Z, then `jobs`, then `fg` and Ctrl+C
- **THEN** `jobs` lists the stopped program, and after Ctrl+C the prompt is back with no error about the terminal's process group

#### Scenario: A full-screen program
- **WHEN** the pane runs `vi <w>/f`, the client types `ihello`, Escape, `:wq` and Enter
- **THEN** `<w>/f` contains `hello`

### Requirement: The variables of a sandboxed pane
A sandboxed pane's environment SHALL hold only `HOME`, `USER`, `LOGNAME`, `SHELL`, `TERM`, `COLORTERM`, `TILR_PANE`, `TMPDIR`, `PATH`, the variables named in `[sandbox] env` with the server's values, and the profile's `[env]`, except that `TILR_*`, `SSH_AUTH_SOCK` and `SSH_AGENT_PID` are dropped from both. `PATH`, unless set, SHALL be the `executables`' directories followed by `/usr/bin:/bin:/usr/sbin:/sbin`.

**Verified by**: unit tests `sandbox::tests::a_sandboxed_pane_gets_only_its_variables`, `sandbox::tests::linux::a_sandboxed_program_has_only_its_variables`; over the bus not tested yet (tasks 2.2).

#### Scenario: Allowlist
- **WHEN** the server runs with `LANG=C.UTF-8` and `SECRET=x`, and the profile says `env = ["LANG", "TILR_SHIMS"]` and `[env] TILR_SOCKET = "/x"`
- **THEN** the pane's environment has `LANG=C.UTF-8` and no `SECRET`, `TILR_SHIMS`, `TILR_SOCKET` or `SSH_AUTH_SOCK`

### Requirement: The request reaches the pane process privately
The pane process of a sandboxed pane SHALL get its sandbox request and the server's protected directories in a file of mode 0600 in the runtime directory, `<runtime dir>/sandbox/<pane id>.request`, named by its command line (`--sandbox <file>`); no value of the profile's `[env]` SHALL be on its command line. A reload execs it with the same arguments and the file SHALL stay until the pane ends.

**Verified by**: unit test `sandbox::tests::a_request_reaches_the_pane_process_in_a_private_file`.

#### Scenario: A value in `[env]`
- **WHEN** a request whose `[env]` holds `API_TOKEN` is handed to a pane process
- **THEN** its command line is `--sandbox <file>` with neither the name nor the value, the file is 0600 and holds the request whole

### Requirement: A sandboxed pane cannot reach the server
From a sandboxed pane it MUST NOT be possible to create a Unix-domain socket that reaches another process (Linux: see the socket requirement; macOS: no Unix socket connect is allowed), so neither the server's socket, the host-events and agent sockets nor any other Unix socket can be reached; nor to read the server's state directory.

**Verified by**: unit test `sandbox::tests::linux::a_sandboxed_program_cannot_reach_the_server`; over the bus not tested yet (tasks 2.2).

#### Scenario: Connect to the server socket
- **WHEN** a sandboxed pane with `network = "server"` runs a granted program that connects to the server's Unix socket path, given on its command line
- **THEN** the connection fails and the server logs no new connection

### Requirement: A grant cannot expose the server's files
A grant that covers or lies inside the state directory, the runtime directory, the configuration directory (the `--config` file's and the profiles directory included) SHALL be refused with `bad_request` starting `create pane: sandbox: `, read-only included, and so SHALL a `read_write` grant covering the server's executable. The check SHALL be made on what is opened for the rule, so a path made a link later cannot slip past it.

**Verified by**: unit tests `sandbox::tests::a_grant_covering_a_server_directory_is_refused`, `sandbox::tests::the_servers_executable_and_configuration_are_protected`, `sandbox::tests::linux::a_grant_that_became_a_link_to_a_server_directory_is_refused`, `sandbox::tests::linux::a_sandboxed_program_cannot_reach_the_server`; over the bus not tested yet (tasks 2.2).

#### Scenario: Home granted
- **WHEN** a profile says `read_only = ["~"]` and the state directory is under home
- **THEN** `pane.create` is answered `bad_request` starting `create pane: sandbox: ` naming the state directory, and no pane starts

#### Scenario: The server's executable
- **WHEN** a profile grants the directory of the server's executable in `read_write`
- **THEN** the pane is refused naming the server's executable; in `read_only` it starts

#### Scenario: A grant that became a link
- **WHEN** a granted path is made a link to the state directory after the request was checked
- **THEN** building the restriction refuses it naming the state directory

### Requirement: A sandbox that cannot be applied starts nothing
If the restriction cannot be installed for a pane, its program MUST NOT run and the caller SHALL get `bad_request` starting `create pane: sandbox: `; no pane SHALL be listed for the request.

**Verified by**: unit test `sandbox::tests::linux::a_sandbox_that_cannot_be_applied_starts_nothing`; over the bus (failpoints `sandbox-apply`, `sandbox-exec-missing`) not tested yet (tasks 2.2, `sandbox-macos`).

#### Scenario: Install fails
- **WHEN** the failpoint makes installing the restriction fail and the profile's command is `touch <marker>`
- **THEN** the caller gets that error and `<marker>` does not exist

#### Scenario: No sandbox-exec on macOS
- **WHEN** on macOS the failpoint `sandbox-exec-missing` makes `/usr/bin/sandbox-exec` unavailable and a sandboxed profile is used
- **THEN** `sandbox` is not in `server.info.features` and the pane is refused as the `pane-profiles` spec says; nothing runs

### Requirement: Sandbox leftovers are removed
The private temporary directory of a sandboxed pane (`TMPDIR`, `<runtime dir>/sandbox/<pane id>/`, mode 0700), the file its request was handed over in and any other per-pane artifact the server makes for the sandbox SHALL be removed when the pane ends, and those of panes that no longer run SHALL be removed when the server starts.

**Verified by**: unit tests `sandbox::tests::a_sandboxed_panes_directory_is_private_and_removed`, `sandbox::tests::a_request_reaches_the_pane_process_in_a_private_file`, `sandbox::tests::linux::the_private_tmpdir_is_writable`; over the bus not tested yet (tasks 2.3).

#### Scenario: Pane ends
- **WHEN** a sandboxed pane writes `$TMPDIR/x` and is closed
- **THEN** its directory under `<runtime dir>/sandbox/` and its request file are gone

#### Scenario: Crash
- **WHEN** the server is killed with SIGKILL while a sandboxed pane runs and is started again
- **THEN** after start nothing under `<runtime dir>/sandbox/` names a pane that does not run
