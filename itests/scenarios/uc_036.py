"""UC-036: the SSH agent sockets of the panes (`agent/<pid>/<pane>.sock` in the runtime
directory) that a tilrd killed with SIGKILL leaves behind are removed when the next one
starts; those of another live server stay, even one that does not accept yet.
"""

import os
import signal
import socket
from pathlib import Path

from tt import T, case


@case(spec=["ssh-agent-forwarding/sockets-a-killed-server-left-are-swept-at-start"])
def agent_sockets_left_by_a_killed_server_are_removed_at_the_next_start(t: T):
    """UC-036: SIGKILL leaves agent/<pid>/<pane>.sock; the next tilrd removes them and makes its own."""
    user = t.user()
    agents = Path("/tmp") / f"tilr-{user.uid}" / "agent"
    t.eq(user.run("tilr", "new", "-d").code, 0, "tilr new -d")
    t.eq(user.run("tilr", "new", "-d").code, 0, "a second pane")
    left = sorted(agents.glob("*/*.sock"))
    t.eq(len(left), 2, f"a socket per pane in {agents}")
    daemon = t.tilrd_daemons()[0]
    t.eq({p.parent.name for p in left}, {str(daemon.pid)}, "in a directory named by the server's pid")
    os.kill(daemon.pid, signal.SIGKILL)
    t.eventually(lambda: not t.proc(daemon.pid) or not t.proc(daemon.pid).alive, "tilrd is gone", timeout=5)
    t.ok(all(p.exists() for p in left), "SIGKILL leaves them")

    # Another live server's socket (as a stand-in: the directory of a running pid, this
    # process, with a socket bound that does not listen yet, as one being made is).
    stand_in = agents / str(os.getpid())
    stand_in.mkdir()
    live = socket.socket(socket.AF_UNIX)
    live.bind(str(stand_in / "00000000-0000-0000-0000-000000000000.sock"))
    os.chmod(stand_in / "00000000-0000-0000-0000-000000000000.sock", 0o666)

    pane = user.run("tilr", "new", "-d").out.strip()
    now = sorted(agents.glob("*/*.sock"))
    t.eq([p for p in left if p.exists()], [], "the killed server's sockets are gone")
    t.ok(not (agents / str(daemon.pid)).exists(), "and its directory")
    t.ok(any(p.name == f"{pane}.sock" for p in now), "the new pane has its own")
    t.ok((stand_in / "00000000-0000-0000-0000-000000000000.sock").exists(), "a live server's socket stays")
    t.has((Path("/tmp") / f"tilr-{user.uid}" / "tilrd.log").read_text(), "removed 2 agent socket(s)")
