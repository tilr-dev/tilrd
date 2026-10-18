# Proposal: Each of one user's servers keeps its agent sockets in a directory of its own

## Why

TILR-118. `agent.rs` tests flaked at c54b66a when `--test agent` ran alone (2 of 8 runs on macOS): `the_sockets_a_killed_server_left_are_removed_when_the_next_starts` ("SIGKILL leaves the socket"), `a_pane_has_ssh_auth_sock_pointing_at_its_own_socket` (the socket in `SSH_AUTH_SOCK` not found) and `the_panes_socket_works_after_a_reload` (`SSH_AUTH_SOCK` not `agent/<pane id>.sock`). Re-checked at 71c3176:

- Every server of one user puts its panes' sockets flat in one directory, `agent/<pane id>.sock` in the runtime directory (`agent.rs`, `socket_path`); on macOS the tests' runtime directory is too long for a socket, so all of them share `/tmp/tilr-<uid>/agent`, as two real servers of one user share the runtime directory.
- A server starting fresh sweeps that directory (`agent::sweep`, from 13f9522, not in a release yet) and decides that a socket's server is gone when a connection to it is refused, then removes the file. A live server's socket refuses connections too between `bind` and `listen` (a bound socket that does not listen yet: `ECONNREFUSED`), and the probe and the removal are not one step: the sweep removes a socket a live server has just made. That server's pane is left with an `SSH_AUTH_SOCK` that names nothing, or, when the file goes before the server's `chmod`, with no agent at all (`no agent in the pane`). The first test's failure is only the test: another test's server swept the killed server's socket first, rightly.
- The probe also connects to every live socket of the other servers: on Linux the live server opens an agent stream to its client for a program nobody ran (`StreamOpen`, then `StreamClose`); on macOS the peer check of the closed connection fails and the live server logs `agent socket: refused a connection` (reproduced by `agent::a_server_starting_leaves_the_agent_sockets_of_a_live_one_alone`, red at 71c3176).

So it is a product bug, not a test-isolation one: with two servers of one user (two Unix sockets, one runtime directory; see `host-socket-per-process`, ddcc453), a server starting can take the agent away from a pane of the other and asks the other's agent client for nothing.

## Decision (2026-10-18, technical)

- Each server keeps its panes' sockets in `agent/<pid>/` (as `sshd` keeps a forwarded agent socket in a directory of the session's, `/tmp/ssh-XXXX/agent.<pid>`, and as the host socket of a second server is `<server_id>-<pid>.sock`). The sweep removes the `agent/<pid>/` directories of pids that are not running (`kill(pid, 0)`, as the host socket sweep does) and connects to nothing. The pid survives a reload in place, so the successor finds the same paths.
- Rejected: keeping the flat directory and making the probe safe (a lock held around the sweep and around every bind): it still connects to live sockets, and a reload's gap between the old listener closing and the new one listening would still see sockets removed and made again.

## What Changes

- A pane's socket is `agent/<pid>/<pane id>.sock` in the runtime directory (`<pid>` the server's), mode 0600, in directories of mode 0700; `SSH_AUTH_SOCK` points at it.
- A server starting fresh removes the `agent/<pid>/` directories of pids that are not running, and logs `removed N agent socket(s) a server that is gone left`. It does not connect to any socket.
- A server stopping for good removes its sockets and then its `agent/<pid>/` directory.
- After a reload a pane's socket is made again at the path its shell has: `agent/<pid>/<pane id>.sock`, or, for a pane a server from before this change started, `agent/<pane id>.sock` while that socket file is there.

## Capabilities

### Modified Capabilities
- `ssh-agent-forwarding`: see `specs/ssh-agent-forwarding/spec.md`.
- `access-control`: the agent socket's path in "The host-events and agent sockets check the peer's uid"; see `specs/access-control/spec.md`.

## Non-goals

- Sweeping flat `agent/<pane id>.sock` sockets that killed servers from before this change left (whose server is gone cannot be told without a probe); they stay until removed by hand.
- A pid reused by another process keeps a dead server's directory until that process is gone.
- Changing where the host socket or the unix socket of the server goes.

## Impact

`crates/tilrd/src/agent.rs` (path, sweep, removal of the directory, the legacy path on resume), `crates/tilrd/src/host.rs` (`running` shared), `crates/tilrd/src/supervisor.rs` (the resume path), tests in `crates/tilrd/tests/agent.rs`, itests UC-005, UC-035, UC-036, UC-069 (paths), README "SSH agent forwarding", USE_CASES.md, itests/USE_CASES.md. Survives a reload: same pid, same directory; no state file change. Update from the previous release: 0.1.1 (`tilr-server`) cannot update into this one (the loose names are no longer published since 2a8b425; its users reinstall), and a pane started by a server from before this change keeps the flat path after a reload into it. No protocol change; size S.
