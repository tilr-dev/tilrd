"""UC-069: fifty reloads in a row under load leave no orphans and leak no descriptors.

The load: a counter every 50 ms, a pane printing without pause (`seq` in a loop), a
python REPL and a shell, watched by a client that reconnects on its own and types into
the shell between reloads. Each round sends SIGUSR1 and waits for the server to resume
and the client to be back. Measured: each reload (the log's `reloading` to the last pane
resumed), each client return, the descriptors of the supervisor and of every pane process
before the first reload and after the last (exactly as many: a child is watched through a
pidfd whether the process spawned it or got it from a reload), the memory from the fifth
reload (warmed up) to the last. Afterwards: the same pids
everywhere, no process outside the server's tree, no state file left, as many agent
sockets as panes, and every program still answers.
"""

import re
import signal
import statistics
import time
from pathlib import Path

from tt import T, Counter, case, descendants, proc
from tt.live import reload_times, state_files, wait_reloads

RELOADS = 50
WARM = 5


@case(fixture="server_state", timeout=600)
def fifty_reloads_under_load_leave_no_orphans_and_no_descriptor_leaks(t: T):
    """UC-069: 50 SIGUSR1 reloads with output, a counter and typing; pids kept, fds and memory flat, no orphans."""
    server = t.server(state=t.root / "state")
    d = t.driver()
    d.connect(server, features=["ping", "agent"])
    d.cmd("keepalive", interval_ms=500, timeout_ms=3000, auto_reconnect=True, reconnect_timeout_ms=30_000)
    shell = d.shell_pane()
    d.line(shell, "export MARK=kept; sleep 9999 &")
    shell_value = d.value(shell, "$MARK $$ $(jobs -r | grep -c sleep)")
    counter_pane = d.shell_pane()
    counter = Counter(t, d, counter_pane, period=0.05)
    noisy = d.shell_pane()
    d.line(noisy, "while :; do seq 1 500; done")
    python = d.shell_pane()
    d.line(python, "python3 -q")
    d.wait_for(python, r"(?m)^>>> ?$")
    d.line(python, "x = 69")
    d.wait_for(counter_pane, r"(?m)^tick 20$")

    pane_pids = sorted(p.pid for p in server.panes())
    tree_before = {p.pid: p.cmdline for p in descendants(server.pid)
                   if re.search(r"tilrd.* pane --id |^(\S*/)?-?bash|^python3|^sleep 9999", p.cmdline)}
    runtime = Path(server.env["TILR_RUNTIME_DIR"])
    samples, returns, typed = {}, [], 0
    # Before any reload, as each one after it must be: exactly as many descriptors.
    samples[0] = {pid: (proc(pid).fds, proc(pid).rss_kb) for pid in [server.pid, *pane_pids] if proc(pid)}
    for n in range(1, RELOADS + 1):
        since = d.mark()
        sent = time.monotonic()
        server.signal(signal.SIGUSR1)
        wait_reloads(t, server, n, timeout=30)
        back = d.wait_event("reconnected", since=since, timeout=30)
        returns.append(round((back["_at"] - sent) * 1000))
        d.input(shell, f"echo typed-{n}\r")
        typed += 1
        if n in (WARM, RELOADS):
            # Settled: the pane processes have resumed and the client resubscribed.
            d.wait_for(shell, rf"(?m)^typed-{n}$", timeout=10)
            samples[n] = {pid: (proc(pid).fds, proc(pid).rss_kb) for pid in [server.pid, *pane_pids] if proc(pid)}
            print(f"  after reload {n}: " + ", ".join(f"{pid}: {fds} fds {rss} kB"
                                                      for pid, (fds, rss) in samples[n].items()))
    times = reload_times(server)
    t.metric("reloads", RELOADS)
    t.metric("reload_ms_median", statistics.median(times) if times else None)
    t.metric("reload_ms_max", max(times) if times else None)
    t.metric("client_back_ms_median", statistics.median(returns))
    t.metric("client_back_ms_max", max(returns))
    t.eq(server.logged("resumed after reload with 4 pane"), RELOADS, "every reload resumed with the four panes")

    def descriptors_flat():
        changed = {pid: (samples[0][pid][0], samples[RELOADS][pid][0]) for pid in samples[0]
                   if samples[RELOADS].get(pid, (0, 0))[0] != samples[0][pid][0]}
        t.metric("supervisor_fds", [samples[0][server.pid][0], samples[WARM][server.pid][0],
                                    samples[RELOADS][server.pid][0]])
        t.eq(changed, {}, f"descriptors that changed between before the first reload and after the "
                          f"{RELOADS}th (pid: before, after)")
    t.check("no descriptor leaks in the supervisor or the pane processes", descriptors_flat)

    def memory_flat():
        start, end = samples[WARM][server.pid][1], samples[RELOADS][server.pid][1]
        t.metric("supervisor_rss_kb", [start, end])
        t.ok(end - start <= 8 * 1024, f"the supervisor's memory grows by at most 8 MB (+{end - start} kB)")
    t.check("the supervisor's memory stays flat", memory_flat)

    t.check("the server and the pane processes keep their pids",
            lambda: (t.eq(t.listener_pid(server.port), server.pid, "listener"),
                     t.eq(sorted(p.pid for p in server.panes()), pane_pids, "pane pids")))

    def same_tree():
        # The candidates first, the tree after: a `seq` of the noisy pane started in between
        # is in the tree, one that ended in between is gone by then (not an orphan).
        candidates = t.procs(r"tilrd.* pane --id |^python3 -q|^seq |^sleep 9999")
        now = {p.pid: p.cmdline for p in descendants(server.pid)}
        gone = {pid: c for pid, c in tree_before.items() if pid not in now}
        t.eq(gone, {}, "long-lived processes gone")
        mine = set(now)
        strays = [f"{p.pid} {p.cmdline}" for p in candidates
                  if p.pid not in mine and Path(f"/proc/{p.pid}").exists()]
        t.eq(strays, [], "processes of the server's outside its tree (orphans)")
    t.check("no orphans: the same processes, all under the server", same_tree)
    t.check("no reload state is left behind", lambda: t.eq(state_files(server), [], "state files"))
    t.check("as many agent sockets as panes",
            lambda: t.eq(len(list((runtime / "agent").glob("*.sock"))), 4, "agent sockets"))

    t.check("the shell kept its variable and job",
            lambda: (t.eq(d.value(shell, "$MARK $$ $(jobs -r | grep -c sleep)"), shell_value, "shell")))
    t.check("the python REPL kept its state", lambda: (d.line(python, "print('x', x)"),
                                                       d.wait_for(python, r"(?m)^x 69$")))
    t.check("the counter goes on", lambda: counter.check_screen(d))
    t.check("the counter skipped nothing", lambda: counter.check_log(max_gap=2.0))
    d.interrupt(noisy)
    t.check("the noisy pane stops on Ctrl+C", lambda: t.eq(d.run(noisy, "echo quiet"), 0, "status"))
    t.check("nothing went wrong in the log", lambda: t.eq(
        [l for l in server.log_text().splitlines() if re.search(r"pane lost|panicked|not reloading|reload failed", l)],
        [], "bad log lines"))
