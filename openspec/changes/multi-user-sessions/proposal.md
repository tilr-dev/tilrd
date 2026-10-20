Блокер: the security model of a root tilrd (how a client names the user, how it proves it may, who may ask for another user, which sockets and directories become per-user) is the owner's decision; design.md lays out the options (Q1-Q8) with a recommendation for each. The spec deltas here are written for the recommended options and are rewritten to match the owner's answers before any task is cut from tasks.md.

# Proposal: A system tilrd that runs each session as its user

## Why

Today one tilrd serves one user: it runs as that user, its sockets and directories are that user's (`tilr-<euid>`, `~/.local/state/tilr`, 0700), and every socket refuses a peer of another uid (`access-control`: "The Unix socket checks the peer's uid", "The host-events and agent sockets check the peer's uid"). The owner asked for a mode in which one daemon per machine, run as root, serves the sessions of several users, starting each as its user (TILR-138; owner's decision 2026-10-20 in the task: design first, discuss the options). The per-user model stays the default (decision 2026-10-16, Basic Memory `main/projects/tilr/decisions/2026-10-16-zapusk-tilrd-po-modeli-tmux-klient-podnimaet-demon-bez-root-sluzhba-systemd-user-s-folbekom-crontab`, which already names `/etc/tilr`, `/var/lib/tilr`, `/run/tilr` for a system mode).

## What Changes

- A new opt-in **system mode**, `tilrd --system`, that needs root (euid 0) and refuses to start otherwise with a clear error (design Q1, Q8). Without `--system` nothing changes, also for root.
- System mode has its own paths: socket `/run/tilr/tilrd.sock`, state `/var/lib/tilr`, configuration `/etc/tilr/server.toml`, and per-user directories `/run/tilr/users/<uid>/` (host-events, agent and shim sockets, owned by the user, 0700) (Q5).
- Each connection has a **user**: on the Unix socket the peer's uid (SO_PEERCRED / getpeereid), on TCP the user whose token the `Hello` carries (per-user tokens, `tilr token` prints the caller's) (Q2, Q3).
- Panes, execs, file requests and server commands of a connection run as its user: the pane process drops to the user (initgroups, setgid, setuid) before it opens the pty, the shell gets the user's passwd entry (HOME, USER, LOGNAME, SHELL, home as start directory) and a clean environment, not root's (Q6).
- A connection sees and drives only its user's panes; `PaneInfo.user` names the user. Asking for a pane as another user (`CreatePane.run_as`) is refused unless the connection's user is root on the Unix socket (Q4).
- The per-user peer-uid checks become, in system mode, "the peer is the socket's user or root" for the per-user sockets and "identify, do not refuse" for the shared Unix socket, which is 0660 `root:tilr` (only members of the group `tilr` reach it) (Q5, Q7).
- `tilr` finds the system socket when no per-user server answers and never starts a system tilrd itself.
- Protocol: not breaking. `CreatePane.run_as`, `PaneInfo.user` and `HelloAck.user` are additions an old peer can ignore, announced by the feature flag `multi-user` in `HelloAck.features`; no PROTOCOL_VERSION bump. An old client on a system server works as its own user and does not see user names.
- **BREAKING** for the reload state files: the supervisor's and panes' state files move to `/run/tilr/` (root) and must carry each pane's uid; a reload re-execs a pane process as its user, so `tilrd state-formats` gains a version. Migration: a successor that does not read the new version is not reloaded into (the server restarts its panes instead, as for any unknown state version); a per-user server keeps today's format until it is updated. Update from the previous release: a per-user server is untouched; system mode is new, so there is nothing to migrate.

## Capabilities

### Modified Capabilities
- `access-control`: per-user tokens; the Unix socket identifies the user in system mode; the per-user sockets accept their user (and root); who may ask for another user's pane.
- `daemon-lifecycle`: `--system`, its root check, its paths and modes; the per-user runtime directories.
- `panes`: a pane runs as its connection's user with that user's environment; a connection lists only its user's panes.

### Touched later (deltas written once the options are decided)
- Requirements of today's specs that the recommended options contradict and that become MODIFIED (a "on a server started with `--system` …" clause) once the owner has answered (design "Requirements to modify after the answers"): `access-control` "The access token is made once and kept for the user only", "Showing the token", "The Unix socket is the user's alone and asks no token", "A connection gone before its uid check is not refused"; `panes` "The directory a pane starts in", "Listing panes", "Pane changes are pushed".
- `files`, `exec`: requests run as the connection's user (Q6).
- `clipboard-host-events`, `ssh-agent-forwarding`: sockets in the user's directory; events go only to clients of the pane's user (Q5).
- `cloud-tunnels`, `tilr-cli`, `reload`, `protocol`.

## Non-goals

- PAM sessions (`pam_open_session`: limits, logind sessions, `XDG_RUNTIME_DIR` of the user), password or Kerberos login: out of scope for the first step (Q3, Q6).
- SSH-key authentication of TCP clients (Q3 option d) — a later change if chosen.
- A sudoers-like allow list ("alice may run as deploy") — later (Q4 option b).
- Panes as root over TCP; sandboxing (change `sandboxed-pane`); several system daemons per machine.
- Windows; macOS audit sessions / launchd bootstrap namespaces beyond what `setuid` gives (keychain access in a pane may differ from a login; noted in design).
- Implementation: this is a Research task; implementation tasks are cut from tasks.md after the owner's decision.
