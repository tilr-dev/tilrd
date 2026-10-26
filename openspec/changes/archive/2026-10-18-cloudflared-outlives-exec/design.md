# Design

## Context

At 1163abd the tunnel task owns `cloudflared` (`crates/tilrd/src/cloud.rs:553-663`): it is spawned with `kill_on_drop(true)` (`cloud.rs:539-549`), stopped by SIGTERM, a 3 s wait and SIGKILL (`stop`, `cloud.rs:686-699`), and held before a reload's exec (`Tunnel::hold`). Nothing covers a server that is SIGKILLed (no drop runs) or a signal to the server's process group.

## Decisions

### D1. Own process group; parent-death signal on Linux
`Command::process_group(0)` for `cloudflared`. On Linux a `pre_exec` sets `PR_SET_PDEATHSIG` to SIGTERM and then compares `getppid()` with the pid the server had before the fork; if they differ the parent is already gone and the child exits before `execve`. The parent-death signal fires when the thread that forked exits, not the process: the spawn runs on a tokio worker thread, which lives as long as the runtime, so this is the server's death in practice (the same assumption `exec.rs:433-445` makes for execs). SIGTERM rather than SIGKILL: `cloudflared` then signs off at Cloudflare as on `tunnel_close`. A parent-death signal does not fire on the server's own exec (same process), and the tunnel is stopped before the exec anyway.

### D2. Pid file and sweep (both platforms)
After a successful spawn the task writes the child's pid to `<runtime dir>/cloudflared/pid` (0600, written whole as the other tunnel files) and removes it once the child is reaped. At start (`spawn`, before the first `cloudflared`) the server reads that file; if the pid is alive and its command line names `<runtime dir>/cloudflared/config.yml` (Linux `/proc/<pid>/cmdline`; macOS `sysctl KERN_PROCARGS2`), it sends SIGTERM, waits `TUNNEL_STOP`, then SIGKILL, and logs `stopped a cloudflared left by an earlier server (pid N)`. A pid that is gone or runs something else is ignored and the file removed. This is the only cover on macOS (the leftover runs until the next start of a server on that runtime directory; the Tilr app restarts its server, a service manager does too).
- Rejected: a watchdog wrapper process on macOS (a second executable or mode, more moving parts for a platform where Cloud tunnels run under the app).
- Rejected: killing by name (`pkill cloudflared`) — would hit another user's or another server's tunnel.

### D3. Stuck cloudflared at a reload
No code change: `stop` already SIGKILLs after `TUNNEL_STOP`. A cargo test uses a stand-in `cloudflared` that traps SIGTERM and checks the reload completes and the old pid is gone.

## Reload and reconnect

The old supervisor stops and reaps its `cloudflared` before the exec (unchanged) and removes the pid file; the successor starts its own on Cloud's next `tunnel_open` and writes the file again. The check child of a reload (`TILR_RELOAD_CHECK`, archived `reload-rollback`) does not run the tunnel task, so it never sweeps. Clients are not involved.

## Backpressure

Not applicable: no client data.

## As implemented

- The sweep runs in the tunnel task before its loop (`stop_leftover`), so it precedes any `cloudflared` this server starts; a server without a TCP port runs no tunnel task and sweeps nothing. The pid file is written with mode 0600 when the child has started and removed when it is reaped (stopped, held for a reload, or exited); a tunnel closed removes the whole directory as before.
- The command line is matched argument by argument against `<runtime dir>/cloudflared/config.yml` (Linux `/proc/<pid>/cmdline`, macOS `KERN_PROCARGS2`); a leftover is polled until gone (a process that is not this server's child cannot be waited for), SIGKILLed after `TUNNEL_STOP`.
- The cargo test of the sweep uses a stand-in that ignores SIGTERM, so on Linux the parent-death signal leaves it running and the sweep is exercised on both platforms; the itests case UC-088 checks the parent-death signal itself.
