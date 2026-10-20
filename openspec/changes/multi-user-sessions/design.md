# Design: A system tilrd that runs each session as its user

Status: options for the owner. Every question below lists the options with their pros and cons and a recommendation (**Rec.**). Nothing is decided; the spec deltas follow the recommendations and change with the answers.

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

- (a) **The connection is the user**: Unix socket → the peer's uid; TCP → the user the token belongs to (Q3). No name on the wire; `HelloAck.user` tells the client who it is. + Nothing to forge; + an old client works unchanged. − To act as another user one needs another connection (or Q4's `run_as`).
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

- (a) **Nobody but root on the Unix socket**: `CreatePane.run_as` from a connection whose user is not root is `Error { "only root may run a pane as another user" }`; over TCP it is always refused, root token or not. + Mirrors `su`/`sudo -u` needing root; − an admin who is not root must `sudo tilr …`.
- (b) An allow list in `/etc/tilr/server.toml` (`[run_as] alice = ["deploy"]`). + Delegation without root. − A sudoers clone to get right (wildcards, groups, root).
- (c) Members of a group (`tilr-admin`) may run as any non-root user. + Simple delegation. − Coarse: any member is every user.
- Panes **as root**: only for a connection whose user is root (root logged in locally); never from a TCP token. A root token does not exist (root has no per-user token in system mode).
- **Rec. (a)**; (b) later if a use is named.

## Q5. Which sockets and directories become per-user

| thing | today | system mode (Rec.) | alternative |
|---|---|---|---|
| main Unix socket | `tilr-<euid>/tilrd.sock` 0600 in 0700 | one `/run/tilr/tilrd.sock`, `root:tilr` 0660, dir `/run/tilr` 0755 root; the peer uid identifies the user | (i) 0666, any local user (no group to manage, every account on the box can get a shell — it already has one); (ii) a socket per user in `/run/tilr/users/<uid>/` — keeps the check as it is, but the daemon must know users in advance |
| lock, log | next to the socket | `/run/tilr/tilrd.sock.lock` 0600 root; log to stderr (journald / launchd) | — |
| host-events socket `$TILR_SOCK` | `<runtime>/<server_id>.sock` | `/run/tilr/users/<uid>/<server_id>.sock`, dir 0700 owned by the user | per pane (`…/<pane>.sock`): finer, more sockets |
| agent sockets `SSH_AUTH_SOCK` | `<runtime>/agent/<pid>/<pane>.sock` | `/run/tilr/users/<uid>/agent/<pid>/<pane>.sock`, owned by the user | — |
| shims, shell wrappers | `<runtime>/shims`, wrapper dir | `/run/tilr/users/<uid>/shims`, owned by the user (the user's shell reads them) | one shared root-owned read-only copy |
| reload state | `<runtime>/server-<pid>.state`, `panes/<id>.state` | `/run/tilr/` 0700 root; pane state written by the pane process into `/run/tilr/users/<uid>/panes/` | — |
| state (server-id, token, cloud.json) | `~/.local/state/tilr` | `/var/lib/tilr` 0700 root; per-user tokens `/var/lib/tilr/users/<uid>/token` | the user's own `~/.local/state/tilr` — rejected: root writing in a user's home follows the user's links |
| configuration | `~/.config/tilr/server.toml` | `/etc/tilr/server.toml` | — |
| clipboard / host events | to every client of the server | only to clients whose user is the pane's user | — |

The directories under `/run/tilr/users/<uid>/` are made by root, `chown`ed to the user and opened with `O_NOFOLLOW`/`openat` from the root-owned `/run/tilr/users` so a user cannot point them elsewhere. **Rec. the table's middle column**, main socket with the group `tilr`.

## Q6. Where the drop to the user happens

- (a) **The pane process drops**: the supervisor (root) forks `tilrd pane`, which right away does `initgroups(user, gid)`, `setgid`, `setuid` (and checks it cannot regain root), then opens the pty, runs the screen model and the shell. + The VT parser that reads the user's program output never runs as root; + a pane process compromised is that user only. − Reload re-execs the pane process as the user: the executable must be readable by the user (it is, in `/usr/local/bin`); − the supervisor talks to it over the pipe it already has.
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
- daemon-lifecycle: `--system` paths; `tilr` never starts a system tilrd (a service manager does: `tilr service install --system` is a later task); the runtime/state directory owner checks expect root for `/run/tilr`, `/var/lib/tilr` and the user for `/run/tilr/users/<uid>`.
- Client: `tilr` today refuses a server of another uid (`check_peer`); for the system socket it must accept a server whose uid is 0.

## Q8. Without root

- `tilrd --system` with euid ≠ 0: exit 1 before binding anything, `--system needs root: tilrd runs as uid N; run it as root (a system service), or without --system for a server of your own`.
- `CreatePane.run_as` on a per-user server: `Error { "this server runs panes as its own user only" }`, no pane made.
- A system tilrd that loses root (setuid by someone else, a container without `CAP_SETUID`): the first pane fails with the `setuid` error in `PaneCreated.error`; the start check also tries `setgroups` capability — `--system` checks `CAP_SETUID`/`CAP_SETGID` on Linux, not just euid 0, and refuses with the same message naming the missing capability.

## Reload and reconnect

- Reload: the supervisor re-execs as root and keeps the listeners and the lock; each pane process re-execs itself (as its user) and hands its state over through `/run/tilr/users/<uid>/panes/<id>.state`; the supervisor's state records each pane's uid, so a successor that does not know the field (older state version) is not reloaded into (`tilrd state-formats`).
- Reconnect: a client reconnecting on the Unix socket gets the same user from its uid; on TCP from its token; it sees its user's panes as before.
- No new data in quantity: the bulk lane and its backpressure are unchanged.

## Risks

- Every bug in the user check is a privilege escalation; the tests must run as root with at least two users (itests group D already has uid stands, UC-034/UC-035).
- Symlink and TOCTOU attacks on `/run/tilr/users/<uid>/` (Q5: create with `openat` from a root-owned parent).
- Token storage: a leaked per-user token is that user's shell, as today.
