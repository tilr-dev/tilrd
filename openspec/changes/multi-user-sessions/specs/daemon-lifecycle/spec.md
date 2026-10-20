## ADDED Requirements

### Requirement: System mode needs root
`tilrd --system` SHALL check, before it binds or creates anything, that it runs with euid 0; otherwise it MUST exit 1 with `--system needs root: tilrd runs as uid N; run it as root (a system service), or without --system for a server of your own`. Without `--system` a server run as root SHALL behave as today: a per-user server of root.

**Verified by**: not tested yet (a test is named in tasks.md).

#### Scenario: Not root
- **WHEN** a user with uid 1000 runs `tilrd --system`
- **THEN** it exits 1 with `--system needs root: tilrd runs as uid 1000`, and nothing is made under `/run/tilr` or `/var/lib/tilr`

#### Scenario: Root without --system
- **WHEN** no system tilrd runs and root runs `tilr new -d`
- **THEN** a per-user server of root starts on root's runtime directory, as without this change

### Requirement: System mode needs CAP_SETUID and CAP_SETGID
On Linux, `tilrd --system` run with euid 0 SHALL also check, before it binds or creates anything, that it holds `CAP_SETUID` and `CAP_SETGID`; otherwise it MUST exit 1 with `--system needs CAP_SETUID and CAP_SETGID: tilrd runs as root without <missing>`.

**Verified by**: not tested yet (a test is named in tasks.md).

#### Scenario: Root without capabilities
- **WHEN** `tilrd --system` runs as uid 0 in a container without `CAP_SETUID`
- **THEN** it exits 1 with `--system needs CAP_SETUID and CAP_SETGID: tilrd runs as root without CAP_SETUID`

### Requirement: Paths and modes of a system server
A server started with `--system` SHALL ignore `TILR_RUNTIME_DIR`, `XDG_RUNTIME_DIR`, `TILR_STATE_DIR`, `XDG_STATE_HOME` and `TILR_CONFIG_DIR` for its defaults and use: socket `/run/tilr/tilrd.sock` (`root:tilr`, 0660; `--socket` still overrides), its lock `/run/tilr/tilrd.sock.lock` (0600), `/run/tilr` 0755 owned by root, state `/var/lib/tilr` (0700 root), configuration `/etc/tilr/server.toml`. It MUST refuse `/run/tilr` or `/var/lib/tilr` owned by another uid than 0.

**Verified by**: not tested yet (a test is named in tasks.md).

#### Scenario: Started as a service
- **WHEN** root runs `tilrd --system` with `XDG_RUNTIME_DIR` set
- **THEN** `/run/tilr/tilrd.sock` is `root:tilr` 0660, `/var/lib/tilr` is 0700 root, and nothing is made in `XDG_RUNTIME_DIR`

#### Scenario: State directory of another owner
- **WHEN** `/var/lib/tilr` belongs to uid 1000 and root runs `tilrd --system`
- **THEN** it exits 1 with `/var/lib/tilr belongs to another user`

### Requirement: System directories are writable by root only
A server started with `--system` MUST refuse to start when `/run/tilr`, `/run/tilr/users` (0755) or `/var/lib/tilr` is not a directory owned by root and writable by root only, and SHALL check, before using them, that an existing `/run/tilr/users/<uid>` and its `agent/`, `agent/<pid>/` and `shims/` are directories (not links) owned by root, else fail the pane and log the path.

**Verified by**: not tested yet (a test is named in tasks.md).

#### Scenario: A world-writable runtime directory
- **WHEN** `/run/tilr` is 0777 owned by root and root runs `tilrd --system`
- **THEN** it exits 1 naming `/run/tilr`, and nothing is bound

#### Scenario: A link in place of a user's directory
- **WHEN** `/run/tilr/users/<alice's uid>` is a symbolic link to `/etc` before her first pane
- **THEN** the pane fails, nothing in `/etc` changes, and the log names the path

### Requirement: Per-user directories of a system server
A server started with `--system` SHALL make, for each user with a pane, `/run/tilr/users/<uid>/` and its subdirectories `agent/`, `agent/<pid>/` and `shims/` owned by root (directories 0711, shims 0755), and SHALL bind that user's host-events and agent sockets there itself, each 0600 root until it is made, then owned by the user. Its only user-owned entry SHALL be `panes/` (0700, made empty by root).

**Verified by**: not tested yet (a test is named in tasks.md).

#### Scenario: After alice's first pane
- **WHEN** a system tilrd runs and alice creates her first pane
- **THEN** `/run/tilr/users/<alice's uid>/` is 0711 owned by root, her pane's `TILR_SOCK` and `SSH_AUTH_SOCK` are in it, 0600 owned by alice, and `panes/` is 0700 owned by alice

### Requirement: Root makes nothing inside a user's directory
A server started with `--system` (uid 0) MUST NOT make, open or remove any entry inside a directory owned by a user. `panes/` SHALL be written only by that user's pane processes after their drop, and root SHALL remove it only when it is empty; a new pane process of that user, after its drop, SHALL remove the state files of panes that no longer run.

**Verified by**: not tested yet (a test is named in tasks.md).

#### Scenario: Alice cannot add entries to her directory
- **WHEN** alice tries to create `shims` or `agent` as a symbolic link to `/etc` in `/run/tilr/users/<alice's uid>/` and then creates panes
- **THEN** her link is refused with `Permission denied`, and nothing in `/etc` changes

#### Scenario: A link in panes
- **WHEN** alice makes `panes/<id>.state` a symbolic link to `/etc/passwd` before her pane is reloaded
- **THEN** the reload writes nothing through it as root (her pane process, as alice, cannot write `/etc/passwd`), and `/etc/passwd` is unchanged

#### Scenario: A crashed pane leaves no state behind
- **WHEN** one of alice's pane processes is killed after it wrote `panes/<id>.state` and alice then creates a pane
- **THEN** that state file is gone and was removed by a process running as alice

### Requirement: A system server always asks for a token
`tilrd --system --no-auth` MUST exit 1 before binding anything with `--no-auth cannot be used with --system`.

**Verified by**: not tested yet (a test is named in tasks.md).

#### Scenario: No authentication asked for
- **WHEN** root runs `tilrd --system --no-auth`
- **THEN** it exits 1 with `--no-auth cannot be used with --system` and nothing listens

### Requirement: tilr finds a system server and does not start one
When no server answers on the user's own socket and nothing names a socket, `tilr` SHALL use `/run/tilr/tilrd.sock` if a tilrd answers there, accepting that its server runs as uid 0. `tilr` MUST NOT start a system server; with neither socket answering it starts a per-user server as today. A system socket that refuses the connection with permission denied counts as a server that is there (see "tilr outside the group tilr is told so").

**Verified by**: not tested yet (a test is named in tasks.md).

#### Scenario: Only a system server
- **WHEN** a system tilrd runs, alice has no server of her own and runs `tilr new -d`
- **THEN** the pane is made on the system server as alice and no tilrd runs as alice

#### Scenario: Both
- **WHEN** alice also runs a server of her own
- **THEN** `tilr ls` lists the panes of her own server

### Requirement: tilr outside the group tilr is told so
When `tilr` would use `/run/tilr/tilrd.sock` and connecting there fails with permission denied, `tilr` MUST exit 1 with `/run/tilr/tilrd.sock: permission denied: this is the socket of a system tilrd and you are not in the group tilr` and MUST NOT start a server.

**Verified by**: not tested yet (a test is named in tasks.md).

#### Scenario: Not in the group
- **WHEN** a system tilrd runs and carol, not in the group `tilr` and with no server of her own, runs `tilr ls` or `tilr new -d`
- **THEN** it exits 1 with `/run/tilr/tilrd.sock: permission denied: this is the socket of a system tilrd and you are not in the group tilr`, and no tilrd runs as carol

#### Scenario: A stopped system server
- **WHEN** the system tilrd has stopped and left `/run/tilr/tilrd.sock` behind, and carol runs `tilr new -d`
- **THEN** it exits 1 with the same message and no tilrd runs as carol
