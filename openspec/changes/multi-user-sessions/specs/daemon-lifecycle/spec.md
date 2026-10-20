## ADDED Requirements

### Requirement: System mode needs root
`tilrd --system` SHALL check, before it binds or creates anything, that it runs with euid 0 and (on Linux) holds `CAP_SETUID` and `CAP_SETGID`; otherwise it MUST exit 1 with `--system needs root: tilrd runs as uid N; run it as root (a system service), or without --system for a server of your own` (naming the missing capability when euid is 0). Without `--system` a server run as root SHALL behave as today: a per-user server of root.

#### Scenario: Not root
- **WHEN** a user with uid 1000 runs `tilrd --system`
- **THEN** it exits 1 with `--system needs root: tilrd runs as uid 1000`, and nothing is made under `/run/tilr` or `/var/lib/tilr`

#### Scenario: Root without capabilities
- **WHEN** `tilrd --system` runs as uid 0 in a container without `CAP_SETUID`
- **THEN** it exits 1 and the error names `CAP_SETUID`

#### Scenario: Root without --system
- **WHEN** root runs `tilr new -d`
- **THEN** a per-user server of root starts on root's runtime directory, as without this change

### Requirement: Paths and modes of a system server
A server started with `--system` SHALL ignore `TILR_RUNTIME_DIR`, `XDG_RUNTIME_DIR`, `TILR_STATE_DIR`, `XDG_STATE_HOME` and `TILR_CONFIG_DIR` for its defaults and use: socket `/run/tilr/tilrd.sock` (`root:tilr`, 0660; `--socket` still overrides), its lock `/run/tilr/tilrd.sock.lock` (0600), `/run/tilr` 0755 owned by root, state `/var/lib/tilr` (0700 root), configuration `/etc/tilr/server.toml`. It MUST refuse `/run/tilr` or `/var/lib/tilr` owned by another uid than 0.

#### Scenario: Started as a service
- **WHEN** root runs `tilrd --system` with `XDG_RUNTIME_DIR` set
- **THEN** `/run/tilr/tilrd.sock` is `root:tilr` 0660, `/var/lib/tilr` is 0700 root, and nothing is made in `XDG_RUNTIME_DIR`

#### Scenario: State directory of another owner
- **WHEN** `/var/lib/tilr` belongs to uid 1000 and root runs `tilrd --system`
- **THEN** it exits 1 with `/var/lib/tilr belongs to another user`

### Requirement: Per-user directories of a system server
A server started with `--system` SHALL make, for each user with a pane, `/run/tilr/users/<uid>/` (0700, owned by that user) holding that user's host-events socket, agent sockets and shims, and MUST make and open it without following links.

#### Scenario: After alice's first pane
- **WHEN** a system tilrd runs and alice creates her first pane
- **THEN** `/run/tilr/users/<alice's uid>/` is 0700 owned by alice, and her pane's `TILR_SOCK` and `SSH_AUTH_SOCK` are in it

#### Scenario: A link planted in users
- **WHEN** `/run/tilr/users/<alice's uid>` is a symbolic link to `/etc` before her first pane
- **THEN** the pane fails, nothing in `/etc` changes, and the log names the path

### Requirement: A system server always asks for a token
`tilrd --system --no-auth` MUST exit 1 before binding anything with `--no-auth cannot be used with --system`.

#### Scenario: No authentication asked for
- **WHEN** root runs `tilrd --system --no-auth`
- **THEN** it exits 1 with `--no-auth cannot be used with --system` and nothing listens

### Requirement: tilr finds a system server and does not start one
When no server answers on the user's own socket and nothing names a socket, `tilr` SHALL use `/run/tilr/tilrd.sock` if a tilrd answers there, accepting that its server runs as uid 0. `tilr` MUST NOT start a system server; with neither socket answering it starts a per-user server as today.

#### Scenario: Only a system server
- **WHEN** a system tilrd runs, alice has no server of her own and runs `tilr new -d`
- **THEN** the pane is made on the system server as alice and no tilrd runs as alice

#### Scenario: Both
- **WHEN** alice also runs a server of her own
- **THEN** `tilr ls` lists the panes of her own server
