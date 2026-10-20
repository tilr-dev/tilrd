## ADDED Requirements

### Requirement: A system server runs a pane as its connection's user
On a server started with `--system`, the pane process SHALL take the connection's user (see `access-control`) before it opens the pty: supplementary groups from the group database, then the user's gid, then the uid, and it MUST fail the pane when it could still regain uid 0.

#### Scenario: Two users, one server
- **WHEN** a system tilrd runs as root and users alice and bob each run `tilr new -d` and then `id -un; echo $HOME` in their pane
- **THEN** alice's pane prints `alice` and her home, bob's prints `bob` and his home, and both pane processes run with the user's uid, not 0

### Requirement: A pane on a system server gets its user's environment
On a server started with `--system`, the shell SHALL get an environment built from the user's password entry (`HOME`, `USER`, `LOGNAME`, `SHELL`, a default `PATH`) and the pane variables, not the server's, and SHALL start in the user's home unless `CreatePane.cwd` names a directory that user can enter.

#### Scenario: Root's environment does not leak
- **WHEN** the system tilrd was started with `FOO=root-only` in its environment and alice runs `echo "[$FOO]"` in her pane
- **THEN** it prints `[]`

#### Scenario: A directory the user cannot enter
- **WHEN** alice creates a pane with `cwd` `/root`
- **THEN** the pane starts in alice's home

### Requirement: A connection sees and drives only its user's panes
On a server started with `--system`, `PaneList` SHALL list only the panes of the connection's user, each `PaneInfo` with its `user` (the user name); `Subscribe`, input, `Resize`, `Scroll` and `ClosePane` naming a pane of another user MUST be answered as for a pane that does not exist. A connection whose user is root on the Unix socket SHALL see and drive every pane. `HelloAck.features` SHALL list `multi-user` and `HelloAck.user` SHALL name the connection's user.

#### Scenario: Bob lists
- **WHEN** alice has two panes and bob one, and bob runs `tilr ls`
- **THEN** he sees his one pane only

#### Scenario: Bob names alice's pane
- **WHEN** bob sends `Subscribe` with the id of one of alice's panes
- **THEN** he gets the error of a pane that does not exist and no frame of it

#### Scenario: Root lists
- **WHEN** root runs `tilr ls` on the system socket
- **THEN** all three panes are listed, with `alice` or `bob` as their user

### Requirement: Only root may ask for a pane as another user
`CreatePane.run_as` (a user name) SHALL be honoured only on a server started with `--system` and only for a connection whose user is root on the Unix socket; the pane then runs as that user as above. Any other connection asking for `run_as` MUST get `PaneCreated.error` "only root may run a pane as another user", and a server without `--system` "this server runs panes as its own user only"; no pane is made. A pane as root MUST NOT be made for a TCP connection.

#### Scenario: Root asks for alice
- **WHEN** root on the system socket sends `CreatePane { run_as: "alice" }`
- **THEN** the pane runs as alice and alice's `tilr ls` lists it

#### Scenario: Bob asks for alice
- **WHEN** bob sends `CreatePane { run_as: "alice" }`
- **THEN** `PaneCreated.error` is "only root may run a pane as another user" and no pane is made

#### Scenario: Per-user server
- **WHEN** a client of a server started without `--system` sends `CreatePane { run_as: "alice" }`
- **THEN** `PaneCreated.error` is "this server runs panes as its own user only"
