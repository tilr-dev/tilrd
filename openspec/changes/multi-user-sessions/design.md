# Design: A system tilrd that runs each session as its user

Status: options for the owner. Every question below (Q1-Q9, Q4b, Q5a, Q7b, Q7c) lists the options with their pros and cons and a recommendation (**Rec.**). Nothing is decided; the spec deltas follow the recommendations and change with the answers.

## Where the code assumes one user (dev at 7b320bc)

- Paths: `runtime_dir()` = `$TILR_RUNTIME_DIR` / `$XDG_RUNTIME_DIR/tilr` / `<tmp>/tilr-<euid>` (crates/tilr-proto/src/local.rs); `state_dir()` = `$TILR_STATE_DIR` / `$XDG_STATE_HOME/tilr` / `~/.local/state/tilr` (crates/tilrd/src/identity.rs); every private-directory helper checks owner == euid (local.rs `claim`, clipboard.rs `private_dir`, identity.rs `create_private_dir`).
- Peer checks: one helper `local::peer` (uid must equal `geteuid()`), used by the Unix socket (supervisor.rs), the host socket (host.rs) and the agent sockets (agent.rs). The client (`tilr-proto` `check_peer`) also refuses a server of another uid.
- Panes: the supervisor runs `tilrd pane …` with its own uid and environment (supervisor.rs `start_pane_process`); the shell gets HOME/USER/LOGNAME from the inherited environment or `getpwuid(getuid())` (pty.rs). Exec (exec.rs) and files (files.rs, roots at `home_dir()`) run as the daemon.
- Auth: one token per server (`<state>/token`); Unix-socket clients are `local` and skip it.
- No session or owner field: `Supervisor.panes` is flat; `Pane` has no user; `Hello` and `CreatePane` carry no user. Nothing checks for euid 0.

## Q1. How system mode is turned on

- (a) **Explicit `tilrd --system`** (and `system = true` is not a config key: the flag only). Needs euid 0, else exit 1. Paths switch to `/run/tilr`, `/var/lib/tilr`, `/etc/tilr`; the `TILR_*`/XDG variables are ignored for the defaults (a root service's environment is not a user's). + No surprise for root users who run today's per-user tilrd; + one switch to test and document. − One more flag.
- (b) Automatic when euid is 0. + Nothing to type. − Silently changes what `sudo tilr` or a root login does today (a root per-user server); − root's own panes would follow multi-user rules by accident.
- **Rec. (a).**

## Q2. How a client names the user of its session

- (a) **The connection is the user**: Unix socket → the peer's uid; TCP → the user the token belongs to (Q3). No name on the wire; `HelloAck.user` tells the client who it is. + Nothing to forge; + an old TCP client (the Tilr app) works unchanged (today's `tilr` must be updated anyway, Q7). − To act as another user one needs another connection (or Q4's `run_as`).
- (b) `Hello.user` (a name) for the whole connection, checked against the credentials. + Explicit. − A field that must always equal what the credentials already say, or be an "act as" request — two meanings in one field.
- (c) Per pane: `CreatePane.run_as` names a user or uid. + Mixed panes on one connection (an admin console). − Every other message (input, files, exec) then needs "as whom" too, or follows the pane.
- **Rec. (a) for the connection, plus (c) only as the explicit "another user" request of Q4.** Exec and files follow the connection's user; input follows the pane's owner check.

## Q3. How a TCP client proves who it is

- (a) **Per-user tokens**: `/var/lib/tilr/users/<uid>/token` (root-owned 0600, made on first ask); `tilr token` over the Unix socket prints the caller's own; a `Hello.token` maps to its uid. + Fits today's `Hello` and the Tilr app/Cloud flow unchanged; + revoking one user (`tilr token --new`) touches no one else. − A token is a shell as that user: as strong as its storage; − root must look tokens up in constant time over all users (a map keyed by a hash of the token).
- (b) TCP off in system mode (`--no-listen` forced) — Unix socket only. + Smallest attack surface; + no token at all. − No Tilr app on another machine, no Cloud tunnel: system mode only for local terminals and SSH logins.
- (c) Username + password in `Hello`, checked with PAM. + Familiar. − Passwords on the wire and in clients; PAM differs per OS; brute force on the port.
- (d) SSH-key challenge against the user's `~/.ssh/authorized_keys`. + Reuses existing trust; no new secret. − Protocol change (challenge/response before `HelloAck`), key storage in every client; a later change.
- **Rec. (a), with (b) available as `--no-listen`** (already there). (d) is a candidate follow-up.

## Q4. Who may ask for a session as another user

- (a) **Nobody but root on the Unix socket**: `CreatePane.run_as` from a connection whose user is not root gets `PaneCreated.error` "only root may run a pane as another user"; over TCP it is always refused, root token or not. + Mirrors `su`/`sudo -u` needing root; − an admin who is not root must `sudo tilr …`.
- (b) An allow list in `/etc/tilr/server.toml` (`[run_as] alice = ["deploy"]`). + Delegation without root. − A sudoers clone to get right (wildcards, groups, root).
- (c) Members of a group (`tilr-admin`) may run as any non-root user. + Simple delegation. − Coarse: any member is every user.
- Panes **as root**: only for a connection whose user is root (root logged in locally); never from a TCP token. A root token does not exist (root has no per-user token in system mode).
- **Rec. (a)**; (b) later if a use is named.

### Q4b. What root sees on a system server

- (a) **Root sees and drives every pane** (`tilr ls` lists all with their user, root can attach to alice's pane), but host events, clipboard writes (OSC 52) and agent streams of a pane go only to connections of the pane's user, not to root. + An admin can find and close a stuck pane; + alice's clipboard and SSH agent never reach root's client. − Root watching alice's pane without her knowing (root can do this anyway with `ptrace`; `PaneInfo.viewed_by` shows root's connection).
- (b) Root sees only its own panes, like any user; other users' panes only through `run_as`. + Same rule for all. − No admin view; a stuck pane needs `kill`.
- (c) Root sees every pane in `tilr ls` but may not attach or type (read the list, close only). + Admin view without reading other users' screens. − A third access level to specify and test.
- **Rec. (a)**: the deltas (panes "A connection sees and drives only its user's panes", access-control host-events/clipboard scenario) follow it.

## Q5. Which sockets and directories become per-user

| thing | today | system mode (Rec.) | alternative |
|---|---|---|---|
| main Unix socket | `tilr-<euid>/tilrd.sock` 0600 in 0700 | one `/run/tilr/tilrd.sock`, `root:tilr` 0660, dir `/run/tilr` 0755 root; the peer uid identifies the user | (i) 0666, any local user (no group to manage, every account on the box can get a shell — it already has one); (ii) a socket per user in `/run/tilr/users/<uid>/` — keeps the check as it is, but the daemon must know users in advance |
| lock, log | next to the socket | `/run/tilr/tilrd.sock.lock` 0600 root; log to stderr (journald / launchd) | — |
| host-events socket `$TILR_SOCK` | `<runtime>/<server_id>.sock` | `/run/tilr/users/<uid>/<server_id>.sock`, socket 0600 owned by the user in a root-owned directory (Q5a) | per pane (`…/<pane>.sock`): finer, more sockets |
| agent sockets `SSH_AUTH_SOCK` | `<runtime>/agent/<pid>/<pane>.sock` | `/run/tilr/users/<uid>/agent/<pid>/<pane>.sock`, socket 0600 owned by the user, directories root-owned (Q5a) | — |
| shims, shell wrappers | `<runtime>/shims`, wrapper dir | `/run/tilr/users/<uid>/shims`, root-owned 0755, read-only for the user (the user's shell reads them) (Q5a) | one shared root-owned read-only copy |
| reload state | `<runtime>/server-<pid>.state`, `panes/<id>.state` | `/run/tilr/server-<pid>.state` 0600 root (in the 0755 `/run/tilr`); pane state written by the pane process, as the user, into `/run/tilr/users/<uid>/panes/` (the one user-owned directory, Q5a) | — |
| state (server-id, token, cloud.json) | `~/.local/state/tilr` | `/var/lib/tilr` 0700 root; per-user tokens `/var/lib/tilr/users/<uid>/token` | the user's own `~/.local/state/tilr` — rejected: root writing in a user's home follows the user's links |
| configuration | `~/.config/tilr/server.toml` | `/etc/tilr/server.toml` | — |
| clipboard / host events | to every client of the server | only to clients whose user is the pane's user | — |

**Rec. the table's middle column**, main socket with the group `tilr`; who owns `/run/tilr/users/<uid>/` and who writes in it is Q5a.

### Q5a. Who owns a user's directory and who writes in it

Root (the supervisor) makes the host-events and agent sockets, their directories and the shims. If it does so inside a directory the user owns, the user can plant a link there first (`shims -> /etc/cron.d`, `agent -> /etc`) and root then writes where the link points: a privilege escalation. `openat`/`O_NOFOLLOW` on `/run/tilr/users/<uid>` itself does not help, because the links are one level down, and `bind()` of a Unix socket takes a path, not a directory descriptor. (The same reason rejects the user's `~/.local/state/tilr` for root's state in Q5.)

- (a) **Root never makes an entry in a directory the user owns.** `/run/tilr/users/<uid>/` and everything root makes under it (`agent/`, `agent/<pid>/`, `shims/`) are owned by root: the directories 0711 (the user can reach a socket by its path but not list or add entries), the shims 0755. Root binds each socket there itself, with a umask that leaves it 0600 root while it is being made, then `chown`s it to the user (the user only connects). The one directory the user writes in, `panes/` (reload state), is made by root as an empty 0700 directory owned by the user, and root never opens, makes or removes anything inside it; the pane process, after its drop, writes and removes its state there; root removes `panes/` only when it is empty (`unlinkat` with `AT_REMOVEDIR` in its root-owned parent). Stale files there (a pane process that crashed, files alice put there) are removed by a process running as the user: each new pane process of that user, after its drop, removes the state files of panes the supervisor tells it are gone. File requests and anything else that touches a user's own paths go through the per-user helper of Q6. + No path root writes to is under the user's control; + the sockets keep today's 0600 owner check. − The user cannot clean up or add to the tree; − 0711 instead of 0700: other users can reach a socket by name, but a 0600 socket of another owner refuses them (and the peer check refuses them again).
- (a′) As (a), with the root-owned directories `0750 root:<user's primary gid>` instead of 0711. + Others cannot even reach the path. − The primary group is often shared (`users`, gid 100), so it keeps out fewer people than it seems; a private group per user cannot be assumed.
- (b) The draft so far: the directory is the user's (0700), root makes the sockets, `agent/<pid>/` and the shims inside it. + Simplest. − The escalation above; rejected.
- (c) The user's directory stays the user's, and root never touches it: a per-user helper (dropped to the user, as in Q6) makes the sockets and directories and passes the listening sockets to the supervisor over `SCM_RIGHTS`. + Root never resolves a path in a user's tree at all. − The helper must run before the user's first pane and across reloads; fd passing for every socket; more to test.
- **Rec. (a)**; (c) if the helper of Q6 turns out to be needed early anyway.

## Q6. Where the drop to the user happens

- (a) **The pane process drops**: the supervisor (root) forks `tilrd pane`, which right away does `initgroups(user, gid)`, `setgid`, `setuid` (and checks it cannot regain root), then opens the pty, runs the screen model and the shell. + The VT parser that reads the user's program output never runs as root; + a pane process compromised is that user only — provided the supervisor treats the pipe as untrusted: after the drop alice can `ptrace` her pane process and write anything into the pipe, so the supervisor takes the pane's uid, id and the users its host events, clipboard and agent streams go to only from what it recorded when it made the pane, and checks every message from a pane as user input. − Reload re-execs the pane process as the user: the executable must be readable by the user (it is, in `/usr/local/bin`); − the supervisor talks to it over the pipe it already has.
- (b) The pane process stays root and drops only in the shell's `pre_exec`. + Smaller change. − libghostty parses untrusted bytes as root; a bug there is root.
- Exec and files: the same rule — an `ExecStart` runs in a child that drops before `exec`; file requests run in a per-user helper process (a `tilrd files` child per user, started on the first request) — the supervisor never opens a user's path as root. Option: `setfsuid`/`seteuid` per request in the supervisor thread — Linux-only, racy with threads; rejected.
- Environment of a pane: built from the passwd entry, not inherited from root: `HOME`, `USER`, `LOGNAME`, `SHELL`, `PATH` (a fixed default, then the login profile), `TERM`, `COLORTERM`, `TILR_*`, `SSH_AUTH_SOCK`; start directory the user's home; login shell as today. No `XDG_RUNTIME_DIR` without PAM (non-goal); a later change may add PAM sessions.
- macOS: `setuid` gives the uid but not the user's audit session or launchd bootstrap namespace (keychain, `pbcopy` to the GUI may not work); `launchctl asuser` is the documented way. Linux is the first target; macOS system mode may be refused at first.
- **Rec. (a)**, Linux first.

## Q7. What changes in access-control and daemon-lifecycle

- "The Unix socket checks the peer's uid": per-user mode unchanged; system mode reads the peer uid and makes it the connection's user, refusing only a peer that cannot be read or has no passwd entry. Access is limited by the socket's group (Q5).
- "The host-events and agent sockets check the peer's uid": in system mode they accept the socket's user and root and refuse the rest.
- "The access token …", "TCP clients present the token": per-user tokens (Q3). `--no-auth` with `--system` is refused (`--no-auth` would make every TCP client root's choice of user).
- "A server command … Unix socket only": stays; it runs as the connection's user (`tilr token` prints that user's token).
- daemon-lifecycle: `--system` paths; `tilr` never starts a system tilrd (a service manager does: `tilr service install --system` is a later task); the runtime/state directory owner checks expect root for `/run/tilr`, `/var/lib/tilr` and `/run/tilr/users/<uid>` (Q5a), and the user only for its `panes/`.
- Client: `tilr` today refuses a server of another uid (`check_peer`); for the system socket it must accept a server whose uid is 0.

### Q7b. What a user outside the group `tilr` gets from `tilr`

With the Q5 socket (`root:tilr` 0660), carol (not in `tilr`) gets `EACCES` when she connects to `/run/tilr/tilrd.sock`. `tilr` must tell that apart from "no server there".

- (a) **`EACCES` means "a server is there, you may not use it"**: `tilr` exits 1 with `/run/tilr/tilrd.sock: permission denied: a system tilrd runs here and you are not in the group tilr`, and starts nothing. A user who wants a server of her own names its socket (`--socket`) or runs `tilrd` herself. + The admin's setup is not silently bypassed; + no "my panes are not where I expected" puzzles. − A user who used per-user tilr before the admin installed the system server loses the automatic start until she is added to the group or names her socket.
- (b) `EACCES` counts as "no server": carol gets a per-user server of her own, as today. + Nothing changes for users outside the group. − Two servers per user possible without anyone noticing; the group no longer decides who uses the system server, only who may.
- (c) As (b) with a warning on stderr (`a system tilrd runs here but you are not in the group tilr; starting your own`). + Visible and still working. − A warning on every `tilr` run that starts a server.
- **Rec. (a)**: the deltas (access-control "Not in the group" as a direct connect, daemon-lifecycle "tilr outside the group tilr is told so") follow it.

### Q7c. Where `tilr` goes for a user with no server of her own

Once a system tilrd runs, `tilr new` of a user in the group who has no server of her own can land on the system server or start her own, as today.

- (a) **The system server**: `tilr` uses `/run/tilr/tilrd.sock` when the user's own socket does not answer. + One daemon per machine, the point of system mode; + nothing to configure per user. − What `tilr new` does changes for every member once an admin installs a system server; a user who wants her own must start `tilrd` herself or name its socket.
- (b) Her own server, as today; the system server only when named (`--socket /run/tilr/tilrd.sock`). + Nothing changes without the user asking. − Every user must opt in by hand; the per-user daemons the system server was meant to replace keep starting.
- (c) The system server only when `/etc/tilr/client.toml` says so (`prefer_system = true`), else as today. + The admin decides once for the machine. − One more file and key to specify and test.
- **Rec. (a)**: the delta daemon-lifecycle "tilr finds a system server and does not start one" follows it.

## Q8. Without root

- `tilrd --system` with euid ≠ 0: exit 1 before binding anything, `--system needs root: tilrd runs as uid N; run it as root (a system service), or without --system for a server of your own`.
- `CreatePane.run_as` on a per-user server: `PaneCreated.error` "this server runs panes as its own user only", no pane made.
- At start, `--system` checks not just euid 0 but (on Linux) `CAP_SETUID` and `CAP_SETGID`, and refuses with the message above naming the missing capability (a container that runs as root without them).
- If the drop still fails while running (a pane's `initgroups`/`setgid`/`setuid` returns an error), that pane fails with the error in `PaneCreated.error`; other panes are not touched.

## Q9. SOCKS proxy and metrics on a system server

The SOCKS proxy (`socks-proxy`) opens outgoing connections from the server process, which on a system server is root; `/metrics` shows every user's panes.

- (a) **SOCKS off with `--system`** (refused like `--socks` without a token today) and `/metrics` only on the Unix socket for root. + No outgoing connections as root on a user's behalf; + no data of one user to another. − No SOCKS for remote clients of a system server.
- (b) Any user's token opens `/socks`, connections leave from the root process. + Works as today. − Bypasses per-user firewall rules (`--uid-owner`), binds privileged source ports.
- (c) `/socks` relayed through a per-user helper process running as the token's user. + Connections as the user. − One more helper and its lifecycle.
- **Rec. (a)**; (c) if SOCKS on a system server is needed.

## Requirements to modify after the answers

The deltas here change only the two peer-uid requirements. With the recommended options these requirements of today's specs also stop holding on a system server and get a `--system` clause once the answers are in:

- `access-control` "The access token is made once and kept for the user only" (one `<state>/token`) and "Showing the token" (`tilrd token`): per-user tokens instead (Q3).
- `access-control` "The Unix socket is the user's alone and asks no token" (0600 socket, 0700 directory): `root:tilr` 0660 in a 0755 `/run/tilr` (Q5).
- `access-control` "A connection gone before its uid check is not refused" ("a connection from another uid MUST still be refused"): on the system socket another uid is a user, not refused (Q7).
- `panes` "The directory a pane starts in": a `cwd` the user cannot enter falls back to the home (Q6).
- `panes` "Listing panes" ("every running pane") and "Pane changes are pushed" (to every client): only the connection's user's panes (Q2, Q4, Q4b).
- `access-control` "TCP clients present the token in Hello" ("equals its token", one token), "HTTP requests on the port carry the token in a header or the query" (whose token admits `/socks`, `/metrics`): per-user tokens (Q3, Q9).
- `access-control` "Running without authentication": refused with `--system` (the ADDED daemon-lifecycle "A system server always asks for a token" says so; this requirement gets the clause).
- `daemon-lifecycle` "Unix socket location", "Runtime directory and socket modes", "State directory": the `--system` paths (Q5) instead of `tilr-<euid>`, `$XDG_RUNTIME_DIR/tilr`, `~/.local/state/tilr`.
- `daemon-lifecycle` "tilr starts tilrd detached" ("finds nothing accepting on the local socket" → starts one): only when the system socket does not answer either (the ADDED "tilr finds a system server and does not start one").
- `socks-proxy`, `metrics`: per Q9.

## Reload and reconnect

- Reload: the supervisor re-execs as root and keeps the listeners and the lock; each pane process re-execs itself (as its user) and hands its state over through `/run/tilr/users/<uid>/panes/<id>.state`; the supervisor's state records each pane's uid, so a successor that does not know the field (older state version) is not reloaded into (`tilrd state-formats`).
- Reconnect: a client reconnecting on the Unix socket gets the same user from its uid; on TCP from its token; it sees its user's panes as before.
- No new data in quantity: the bulk lane and its backpressure are unchanged.

## Risks

- Every bug in the user check is a privilege escalation; the tests must run as root with at least two users (itests group D already has uid stands, UC-034/UC-035).
- Symlink and TOCTOU attacks under `/run/tilr/users/<uid>/` (Q5a: root makes entries only in root-owned directories, the user-owned `panes/` is never opened by root).
- The pipe between a pane process and the supervisor crosses users (Q6): the supervisor trusts nothing a pane says about who it is or where its events go.
- Token storage: a leaked per-user token is that user's shell, as today.
