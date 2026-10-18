"""UC-010: `tilr` without a running tilrd starts it itself (setsid, detached from the
terminal) and connects.

The client finds no socket in $XDG_RUNTIME_DIR/tilr, starts tilrd as a grandchild in a
session of its own (stdin /dev/null, its log in tilrd.log next to the socket), and
attaches to a new pane over that socket. The server binary is installed as `tilrd` on
PATH.
"""

import os
import shutil
import stat

from tt import T, case


@case()
def tilr_without_a_running_daemon_starts_tilrd_detached_and_connects(t: T):
    """UC-010: the first `tilr` on an empty machine spawns one detached tilrd and attaches."""
    t.need("procfs")
    xdg = t.scratch("xdg")
    os.chmod(xdg, 0o700)
    bin = t.scratch("bin")
    shutil.copy2(shutil.which("tilrd"), bin / "tilrd")
    home = t.scratch("home")
    env = {"PATH": f"{bin}:{os.environ['PATH']}", "XDG_RUNTIME_DIR": str(xdg), "XDG_STATE_HOME": str(home / "state"),
           "XDG_CONFIG_HOME": str(home / "config"), "TERM": "xterm-256color"}
    t.eq(t.procs(r"(^|/)tilrd( |$)"), [], "no daemon runs before")

    client = t.session("tilr", env=env, name="tilr")
    daemon = t.eventually(t.tilrd_daemons,
                          lambda: f"a tilrd process is started; the client's terminal:\n{client.read()}", timeout=10)
    t.eq(len(daemon), 1, "exactly one tilrd")
    d = daemon[0]
    t.eq(d.sid, d.pid, "tilrd leads a session of its own (setsid)")
    t.eq(d.tty, 0, "tilrd has no controlling terminal")
    t.ok(d.ppid != client.pid, "tilrd is not a child of the client")

    sockets = [p for p in (xdg / "tilr").glob("*") if stat.S_ISSOCK(p.stat().st_mode)] if (xdg / "tilr").exists() else []
    t.ok(sockets, f"tilrd listens on a socket in {xdg}/tilr")
    client.prompt(timeout=10)
    print("  ok: the client shows a shell prompt from a pane")

    client.hangup()
    t.eventually(lambda: client.p.poll() is not None, "the client exits with its terminal", timeout=5)
    p = t.proc(d.pid)
    t.ok(p and p.alive, "tilrd keeps running after the client's terminal is gone")
