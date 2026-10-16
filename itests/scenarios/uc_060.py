"""UC-060: `tilr-server update` from a fake CDN (a service container) while the panes run
live software: a shell with a variable and a background job, a counter, vim with an
unsaved buffer, htop, less on a big file, a foreground `sleep 1000`. The version changes,
the server keeps its pid, and every program is alive and answers.

The CDN serves the layout of crates/tilr-server/src/update.rs; its release is the same
source built with another version string (/opt/tt/tilr-server-next). The update runs
outside the panes, as `scripts/update-test.sh` runs it on a real host. Each outcome is a
separate check, so one broken program does not hide the others.
"""

import re
import time

from tt import T, case

RELOADED = "resumed after reload with"


@case(fixture="server_state", services=["cdn"], timeout=420)
def update_from_the_cdn_keeps_every_program_in_the_panes_alive(t: T):
    """UC-060: update with a shell, counter, vim, htop, less and sleep running; all survive."""
    cdn = t.service("cdn")
    manifest = t.manifest()
    exe = t.install()
    server = t.server(exe=exe, state=t.root / "state")
    t.eq(server.version(), manifest["version"], "the installed version before")
    d = t.driver()
    hello = d.connect(server)
    t.eq(hello["server_version"], manifest["version"], "the server's version before")

    shell = d.shell_pane()
    d.line(shell, "export MARK=survived; sleep 9999 &")
    shell_before = d.value(shell, "$MARK $$ $(jobs -r | grep -c sleep)")
    t.eq(shell_before.split()[::2], ["survived", "1"], "the shell has its variable and its job")

    counter = d.shell_pane()
    ticks = t.root / "ticks"
    d.line(counter, f"i=0; while :; do i=$((i+1)); echo \"tick $i\"; echo \"$i $(date +%s.%N)\" >>{ticks}; "
                    "sleep 0.2; done")
    d.wait_for(counter, r"(?m)^tick 5$")

    vim = d.shell_pane()
    notes = t.root / "notes" / "todo.txt"
    d.line(vim, f"vim -n -u NONE -N {notes}")
    d.wait_for(vim, r'"\S*todo\.txt" \[New\]', timeout=15)
    d.input(vim, "ihello from before the update\x1b")
    d.wait_for(vim, r"(?m)^hello from before the update$")

    htop = d.shell_pane()
    d.line(htop, "htop")
    d.wait_for(htop, r"Load average", timeout=15)

    less = d.shell_pane()
    d.line(less, f"less {t.root / 'big.txt'}")
    d.wait_for(less, r"(?m)^line 1$")
    d.key(less, "G")
    d.wait_for(less, r"(?m)^line 200000$")

    sleeper = d.shell_pane()
    d.line(sleeper, "sleep 1000")
    t.eventually(lambda: t.procs(r"^sleep 1000$"), "sleep 1000 runs")

    panes = sorted(p["pane"] for p in d.list())
    pane_pids = sorted(p.pid for p in server.panes())
    t.eq(len(pane_pids), 6, "six pane processes")
    before_lost = d.mark()

    r = server.cmd("update", "--cdn", cdn, timeout=180)
    t.eq(r.code, 0, "tilr-server update exits 0")
    t.has(r.out, manifest["next_version"])
    t.eventually(lambda: server.logged(RELOADED), "the server reloads", timeout=30)
    d.wait_event("disconnected", since=before_lost, timeout=30)
    back = d.cmd("reconnect", timeout_ms=30_000)

    t.check("the executable is the next version", lambda: t.eq(server.version(), manifest["next_version"], "version"))
    t.check("the server says it is the next version",
            lambda: t.eq(back["server_version"], manifest["next_version"], "HelloAck.server_version"))
    t.check("the server keeps its pid", lambda: (t.ok(server.alive(), "alive"),
                                                 t.eq(t.listener_pid(server.port), server.pid, "listener pid")))
    t.check("it reloaded once, with all six panes", lambda: t.eq(server.logged(f"{RELOADED} 6 pane"), 1, "reloads"))
    t.check("the panes are the same", lambda: t.eq(sorted(p["pane"] for p in d.list()), panes, "pane ids"))
    t.check("the pane processes are the same", lambda: t.eq(sorted(p.pid for p in server.panes()), pane_pids, "pids"))
    t.check("the shell keeps its variable, pid and job",
            lambda: t.eq(d.value(shell, "$MARK $$ $(jobs -r | grep -c sleep)"), shell_before, "shell"))

    def counter_went_on():
        n = int(d.wait_for(counter, r"(?m)^tick (\d+)$")["groups"][1])
        d.wait_for(counter, rf"(?m)^tick {n + 5}$")
        written = [line.split() for line in ticks.read_text().splitlines()]
        numbers = [int(n) for n, _ in written]
        t.eq(numbers, list(range(1, len(numbers) + 1)), "the counter skipped nothing")
        stamps = [float(s) for _, s in written]
        gap = max(b - a for a, b in zip(stamps, stamps[1:]))
        t.metric("counter_max_gap_ms", round(gap * 1000))
        t.ok(gap < 1.0, f"no pause over a second (longest {gap:.2f}s)")
    t.check("the counter printed through the update without a gap", counter_went_on)

    def vim_kept_its_buffer():
        screen = d.screen(vim)
        t.ok(screen["alt_screen"], "vim is still on the alternate screen")
        t.has(screen["text"], "hello from before the update")
        d.input(vim, "\x1b:wq\r")
        d.wait_for(vim, r"(?m)^\$ ?$")
        t.eq(notes.read_text(), "hello from before the update\n", "what :wq wrote")
    t.check("vim keeps its unsaved buffer and :wq writes it", vim_kept_its_buffer)

    def htop_still_renders():
        first = d.screen(htop)
        t.has(first["text"], "Load average")
        d.cmd("sleep", ms=4000)
        later = d.screen(htop)
        t.ok(later["frames"] > first["frames"], f"htop redraws ({first['frames']} -> {later['frames']} frames)")
        d.key(htop, "q")
        d.wait_for(htop, r"(?m)^\$ ?$")
    t.check("htop keeps rendering and quits", htop_still_renders)

    def less_answers():
        t.has(d.text(less), "line 200000")
        d.key(less, "g")
        d.wait_for(less, r"^line 1\n")
        d.key(less, "q")
        d.wait_for(less, r"(?m)^\$ ?$")
    t.check("less still shows the file and answers keys", less_answers)

    def ctrl_c_stops_sleep():
        d.key(sleeper, "ctrl-c")
        t.eq(d.value(sleeper, "$?"), "130", "sleep's exit status (SIGINT)")
        t.eventually(lambda: not t.procs(r"^sleep 1000$"), "sleep 1000 is gone", timeout=5)
    t.check("Ctrl+C stops the sleep", ctrl_c_stops_sleep)

    def second_update_does_nothing():
        again = server.cmd("update", "--cdn", cdn, timeout=60)
        t.eq(again.code, 0, "exit status")
        t.has(again.out, "is the latest release")
        time.sleep(3)
        t.eq(server.logged(RELOADED), 1, "reloads")
    t.check("a second update finds nothing to do", second_update_does_nothing)

    t.check("nothing was refused or lost in the log",
            lambda: t.eq(re.findall(r"not reloading|ERROR|panicked", server.log_text()), [], "bad log lines"))
