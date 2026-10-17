"""UC-062: job control across a reload: the terminal of a pane is still the controlling
terminal of its shell's jobs. Ctrl+C interrupts a foreground `sleep`; Ctrl+Z stops one,
`jobs` lists it, `fg` brings it back (the process runs again, as /proc shows), and Ctrl+C
then ends it.

The reload is SIGUSR1; the pane process execs with the pty master kept open, so the
line discipline (ISIG, VINTR, VSUSP) and the session of the shell are untouched.
"""

import signal

from tt import T, case
from tt.live import wait_reloads


def reloaded(t: T):
    server = t.server(state=t.root / "state")
    d = t.driver()
    d.connect(server)
    pane = d.shell_pane()
    since = d.mark()
    server.signal(signal.SIGUSR1)
    wait_reloads(t, server, 1)
    d.wait_event("disconnected", since=since, timeout=10)
    d.cmd("reconnect", timeout_ms=20_000)
    d.wait_for(pane, r"(?m)^\$ ?$")
    return server, d, pane


@case(fixture="server_state")
def ctrl_c_after_a_reload_interrupts_a_foreground_sleep(t: T):
    """UC-062: a reload, then sleep 1000 in the foreground; Ctrl+C ends it with status 130."""
    server, d, pane = reloaded(t)
    d.line(pane, "sleep 1000")
    t.eventually(lambda: t.procs(r"^sleep 1000$"), "sleep 1000 runs")
    d.key(pane, "ctrl-c")
    d.wait_for(pane, r"(?m)^\$ ?$")  # the prompt: what is typed before sleep dies is flushed
    t.eq(d.value(pane, "$?"), "130", "sleep's exit status (SIGINT)")
    t.eventually(lambda: not t.procs(r"^sleep 1000$"), "sleep 1000 is gone", timeout=5)


@case(fixture="server_state")
def a_sleep_running_through_a_reload_is_interrupted_by_ctrl_c(t: T):
    """UC-062: sleep 1000 started before a reload; Ctrl+C after it ends the sleep (130)."""
    server = t.server(state=t.root / "state")
    d = t.driver()
    d.connect(server)
    pane = d.shell_pane()
    d.line(pane, "sleep 1000")
    t.eventually(lambda: t.procs(r"^sleep 1000$"), "sleep 1000 runs")
    since = d.mark()
    server.signal(signal.SIGUSR1)
    wait_reloads(t, server, 1)
    d.wait_event("disconnected", since=since, timeout=10)
    d.cmd("reconnect", timeout_ms=20_000)
    t.ok(t.procs(r"^sleep 1000$"), "sleep 1000 still runs after the reload")
    d.key(pane, "ctrl-c")
    d.wait_for(pane, r"(?m)^\$ ?$")  # the prompt: what is typed before sleep dies is flushed
    t.eq(d.value(pane, "$?"), "130", "sleep's exit status (SIGINT)")


@case(fixture="server_state")
def ctrl_z_and_fg_after_a_reload_stop_and_resume_a_job(t: T):
    """UC-062: after a reload, Ctrl+Z stops sleep 1000 (state T), jobs lists it, fg resumes it, Ctrl+C ends it."""
    server, d, pane = reloaded(t)
    d.line(pane, "sleep 1000")
    sleep = t.eventually(lambda: t.procs(r"^sleep 1000$"), "sleep 1000 runs")[0]
    d.key(pane, "ctrl-z")
    d.wait_for(pane, r"Stopped\s+sleep 1000")
    t.eventually(lambda: t.proc(sleep.pid).state == "T", "the job is stopped (state T)", timeout=5)
    t.has(d.value(pane, "$(jobs -s)"), "sleep 1000")
    d.line(pane, "fg")
    t.eventually(lambda: t.proc(sleep.pid).state in ("S", "R"), "fg runs the job again", timeout=5)
    d.key(pane, "ctrl-c")
    d.wait_for(pane, r"(?m)^\$ ?$")  # the prompt: what is typed before sleep dies is flushed
    t.eq(d.value(pane, "$?"), "130", "sleep's exit status after fg and Ctrl+C")
    t.eventually(lambda: not t.procs(r"^sleep 1000$"), "sleep 1000 is gone", timeout=5)
