## ADDED Requirements

### Requirement: A sandboxed profile starts a sandboxed pane
On a server that lists `sandbox` in `server.info.features`, `pane.create` with a profile that has `[sandbox]` SHALL start the pane with its program and every process it starts restricted to that table's grants, and `PaneInfo.sandboxed` SHALL be true. A pane without such a profile SHALL have `sandboxed` false and SHALL be unrestricted as before.

**Verified by**: `sandbox::a_sandboxed_profile_starts_a_sandboxed_pane`.

#### Scenario: Sandboxed and not
- **WHEN** the owner creates one pane with a sandboxed profile and one without a profile
- **THEN** `pane.list` shows `sandboxed` true for the first and false for the second

### Requirement: The sandbox feature is offered where it can be enforced
`sandbox` SHALL be listed in `server.info.features` only on Linux with Landlock ABI 4 or later and seccomp, or on macOS when `/usr/bin/sandbox-exec` applies a probe profile to `/usr/bin/true` within 2 s. On a Linux kernel with Landlock ABI 4 or 5, every sandboxed pane SHALL be started without signal scoping and the server SHALL log a warning naming the pane and the ABI.

**Verified by**: `sandbox::sandbox_is_offered_where_it_can_be_enforced` (failpoints `sandbox-unavailable`, `sandbox-abi-4`).

#### Scenario: No sandbox on this host
- **WHEN** the server runs with the failpoint `sandbox-unavailable`
- **THEN** `sandbox` is missing from `server.info.features` and a sandboxed profile is refused as the `pane-profiles` spec says

#### Scenario: Kernel without signal scoping
- **WHEN** the failpoint `sandbox-abi-4` makes the server see Landlock ABI 4 and a sandboxed pane is created
- **THEN** the pane starts and the log has a line `sandbox: kernel lacks signal scoping (Landlock ABI 4 < 6)` naming the pane

### Requirement: Files outside the grants cannot be read or written
In a sandboxed pane, a path under `read_write` SHALL be readable, writable and executable; a path under `read_only` SHALL be readable and executable and MUST NOT be writable; any other path MUST be neither readable nor writable, except the system program and library directories, `/etc` and the terminal devices, which SHALL be readable. This SHALL hold for the shell and every process it starts.

**Verified by**: `sandbox::a_sandboxed_pane_writes_only_where_it_may`, `sandbox::a_child_of_the_shell_is_restricted_too`.

#### Scenario: Write outside
- **WHEN** a profile grants `read_write = ["<w>"]`, `read_only = ["<r>"]`, and the pane runs `touch <w>/a <r>/b <o>/c` (another directory `<o>` of the user)
- **THEN** only `<w>/a` exists, and the pane shows permission errors for the other two

#### Scenario: A grandchild
- **WHEN** the pane runs `sh -c 'sh -c "touch <o>/d"'`
- **THEN** `<o>/d` does not exist

### Requirement: Only granted programs run
A sandboxed pane SHALL be able to run the programs under its grants, the `executables` (names looked up in the pane's `PATH` when it is created) and the program the pane was started with, and MUST NOT be able to run another program of the user outside those grants and the system directories.

**Verified by**: `sandbox::only_granted_programs_run`.

#### Scenario: A program in the user's directory
- **WHEN** the user has an executable `<o>/tool` outside the grants and the pane runs `<o>/tool`
- **THEN** it fails with a permission error

### Requirement: Programs made inside run inside
A script or binary created in a `read_write` directory of a sandboxed pane SHALL be executable inside that pane without any step outside it.

**Verified by**: `sandbox::a_script_and_a_binary_made_inside_run_inside`.

#### Scenario: Script and compiled binary
- **WHEN** the pane writes `#!/bin/sh` + `echo ok` to `<w>/t.sh`, runs `chmod +x <w>/t.sh && <w>/t.sh`, and copies `/bin/echo` to `<w>/e` and runs `<w>/e hi`
- **THEN** it prints `ok` and `hi`

### Requirement: Network follows the profile
With `network = "none"` a sandboxed pane MUST NOT open TCP connections nor send UDP; with `"client"` it SHALL connect out over TCP and MUST NOT accept TCP connections, and UDP SHALL NOT be restricted (a documented limit on Linux); with `"server"` it SHALL do both.

**Verified by**: `sandbox::network_follows_the_profile`.

#### Scenario: None
- **WHEN** a test listener runs on 127.0.0.1 and a `none` pane tries to connect to it
- **THEN** the connection fails and the listener sees nothing

#### Scenario: Client
- **WHEN** a `client` pane connects to that listener and then tries to listen on a TCP port
- **THEN** the connection succeeds and listening fails

#### Scenario: Server
- **WHEN** a `server` pane listens on a TCP port and the test connects to it
- **THEN** the test's connection is accepted

### Requirement: A sandboxed pane is a normal terminal
A sandboxed pane SHALL behave as any pane for the terminal: raw and canonical mode (`stty`), full-screen programs, job control (`Ctrl+Z`, `bg`, `fg`, `jobs`), `Ctrl+C`, window size changes, and `pane.command_finished`.

**Verified by**: `sandbox::raw_mode_and_job_control_work_in_a_sandboxed_pane`, itests `UC-0S1` (`uc_0s1.py::vim_less_and_claude_run_in_a_sandboxed_pane`).

#### Scenario: Raw mode
- **WHEN** the pane runs `stty raw -echo; dd bs=1 count=1 2>/dev/null | od -c; stty sane` and the client inputs `x`
- **THEN** `od` prints `x` without Enter having been sent, and `stty` reported no error

#### Scenario: Job control
- **WHEN** the shell runs `sleep 100`, the client sends Ctrl+Z, then `jobs`, then `fg` and Ctrl+C
- **THEN** `jobs` lists the stopped `sleep`, and after Ctrl+C the prompt is back with no error about the terminal's process group

#### Scenario: A full-screen program
- **WHEN** the pane runs `vi <w>/f`, the client types `ihello`, Escape, `:wq` and Enter
- **THEN** `<w>/f` contains `hello`

### Requirement: The variables of a sandboxed pane
A sandboxed pane's environment SHALL hold only `HOME`, `USER`, `LOGNAME`, `SHELL`, `TERM`, `COLORTERM`, `TILR_PANE`, `TMPDIR`, `PATH`, the variables named in `[sandbox] env` with the server's values, and the profile's `[env]`. `PATH`, unless set, SHALL be the `executables`' directories followed by `/usr/bin:/bin:/usr/sbin:/sbin`.

**Verified by**: `sandbox::a_sandboxed_pane_has_only_its_variables`.

#### Scenario: Allowlist
- **WHEN** the server runs with `LANG=C.UTF-8` and `SECRET=x`, and the profile says `env = ["LANG"]`
- **THEN** `env` in the pane shows `LANG=C.UTF-8` and no `SECRET`, `TILR_SHIMS`, `TILR_BUS_FD` or `SSH_AUTH_SOCK`

### Requirement: A sandboxed pane cannot reach the server
From a sandboxed pane it MUST NOT be possible to create a Unix-domain socket (`socket(AF_UNIX, …)` fails on Linux; no Unix socket connect is allowed on macOS), so neither the server's socket, the host-events and agent sockets nor any other Unix socket can be reached; nor to read the server's state directory. A profile whose grants cover the state directory, the runtime directory or the configuration directory SHALL be refused with `bad_request` starting `create pane: sandbox: `.

**Verified by**: `sandbox::a_sandboxed_pane_cannot_reach_the_server`, `sandbox::a_grant_over_the_state_directory_is_refused`.

#### Scenario: Connect to the server socket
- **WHEN** a sandboxed pane with `network = "server"` runs a granted program that connects to the server's Unix socket path, given on its command line
- **THEN** the connection fails and the server logs no new connection

#### Scenario: Home granted
- **WHEN** a profile says `read_only = ["~"]` and the state directory is under home
- **THEN** `pane.create` is answered `bad_request` starting `create pane: sandbox: ` naming the state directory, and no pane starts

#### Scenario: Configuration directory read-only
- **WHEN** a profile says `read_only = ["<config dir>"]`
- **THEN** `pane.create` is refused naming the configuration directory

### Requirement: A sandbox that cannot be applied starts nothing
If the restriction cannot be installed for a pane, its program MUST NOT run and the caller SHALL get `bad_request` starting `create pane: sandbox: `; no pane SHALL be listed for the request.

**Verified by**: `sandbox::a_sandbox_that_cannot_be_applied_starts_nothing` (failpoints `sandbox-apply`, `sandbox-exec-missing`).

#### Scenario: Install fails
- **WHEN** the failpoint makes installing the restriction fail and the profile's command is `touch <marker>`
- **THEN** the caller gets that error and `<marker>` does not exist

#### Scenario: No sandbox-exec on macOS
- **WHEN** on macOS the failpoint `sandbox-exec-missing` makes `/usr/bin/sandbox-exec` unavailable and a sandboxed profile is used
- **THEN** `sandbox` is not in `server.info.features` and the pane is refused as the `pane-profiles` spec says; nothing runs

### Requirement: The macOS profile carries paths as parameters only
On macOS the server SHALL generate the Seatbelt profile from fixed rules and parameter names only, passing every path as a `-D NAME=VALUE` parameter, and SHALL keep it as `<runtime dir>/sandbox/<pane id>/profile.sb` (mode 0600) for the pane's life.

**Verified by**: unit tests `sandbox::macos::tests::paths_are_passed_as_parameters_only`, `sandbox::macos::tests::the_necessary_rules_are_always_present`, `sandbox::macos::tests::no_unix_socket_is_allowed_but_dns` (crates/tilrd/src/sandbox/macos.rs).

#### Scenario: A path with quotes
- **WHEN** a profile grants `read_write = ["~/a \"b\")"]` on macOS
- **THEN** `profile.sb` does not contain that path, and the pane can write in that directory

### Requirement: The sandbox survives a reload
After a reload, a sandboxed pane SHALL still be sandboxed: `PaneInfo.sandboxed` stays true and programs started in it after the reload are restricted as before.

**Verified by**: `reload::a_sandboxed_pane_stays_sandboxed_across_a_reload`.

#### Scenario: Write after reload
- **WHEN** a sandboxed pane exists, the server reloads, the client reconnects, and the pane runs `touch <o>/after`
- **THEN** `<o>/after` does not exist and `pane.list` shows `sandboxed` true

### Requirement: Sandbox leftovers are removed
The private temporary directory of a sandboxed pane (`TMPDIR`, `<runtime dir>/sandbox/<pane id>/`, mode 0700) and any other per-pane artifact the server makes for the sandbox SHALL be removed when the pane ends, and those of panes that no longer run SHALL be removed when the server starts.

**Verified by**: `sandbox::a_sandboxed_panes_leftovers_are_removed`.

#### Scenario: Pane ends
- **WHEN** a sandboxed pane writes `$TMPDIR/x` and is closed
- **THEN** its directory under `<runtime dir>/sandbox/` is gone

#### Scenario: Crash
- **WHEN** the server is killed with SIGKILL while a sandboxed pane runs and is started again
- **THEN** after start no directory under `<runtime dir>/sandbox/` names a pane that does not run

### Requirement: The sandbox is tested on every supported macOS
CI SHALL run every `sandbox::*` test and `reload::a_sandboxed_pane_stays_sandboxed_across_a_reload` on macOS 14, 15 and 26 in the merge queue, on release branches and in the nightly run on `main`.

**Verified by**: CI job `sandbox-macos` (matrix `macos-14`, `macos-15`, `macos-26`) in `.github/workflows/ci.yml`.

#### Scenario: Merge queue
- **WHEN** a pull request enters the merge queue
- **THEN** the `sandbox-macos` job runs on all three runners and the merge waits for them
