"""UC-014: a stale socket (tilrd killed with SIGKILL): the attached tilr says the server
stopped; the next `tilr` removes the socket and starts a new tilrd.

A killed server leaves its socket file; its lock goes with the process. A socket nobody
accepts on whose lock is free is what a dead server left: the new tilrd (holding the
lock) removes it and listens there.
"""

import os
import signal
import stat
from pathlib import Path

from tt import T, case


@case()
def after_a_sigkill_the_next_tilr_replaces_the_socket_and_starts_a_new_daemon(t: T):
    """UC-014: tilrd SIGKILLed; its socket is left; the next tilr cleans it up and starts tilrd."""
    t.need("procfs")
    env = t.user_env()
    socket = Path(env["XDG_RUNTIME_DIR"]) / "tilr" / "tilrd.sock"
    first = t.session("tilr", env=env, name="first")
    first.prompt(timeout=15)
    old = t.tilrd_daemons()[0]
    os.kill(old.pid, signal.SIGKILL)
    t.eventually(lambda: not (t.proc(old.pid) and t.proc(old.pid).alive), "tilrd is gone")
    t.ok(stat.S_ISSOCK(socket.lstat().st_mode), "the killed server left its socket")
    first.expect(r"\[tilrd stopped\]", timeout=10)
    t.eq(first.wait_exit(), 0, "the attached tilr ends")
    t.eventually(lambda: not t.procs(r"(^|/)tilrd pane "), "the old panes' processes end", timeout=10)

    r = t.tilr("ls", env=env)
    t.ok(r.code != 0 and "no tilrd runs" in r.err, f"ls starts nothing and says so: {r.text}")
    second = t.session("tilr", env=env, name="second")
    second.prompt(timeout=15)
    new = t.tilrd_daemons()
    t.eq(len(new), 1, "one tilrd")
    t.ok(new[0].pid != old.pid, "a new one")
    log = (socket.parent / "tilrd.log").read_text()
    t.has(log, "removing the socket of a server that is gone")
    second.write('echo "new-$((40+2))"\r')
    second.expect("new-42")
