"""UC-033: the TCP port is taken: tilrd stops at once with an error that says so (and
what to do), exit status 1, and leaves nothing (no socket). A tilrd that `tilr` starts
is the exception: the port may be the Tilr app's or another user's, so it warns in its
log and serves its Unix socket alone, and `tilr` works.
"""

import socket
import time
from pathlib import Path

from tt import SERVER_EXE, T, case


def taken(port: int = 0) -> socket.socket:
    s = socket.socket()
    s.bind(("127.0.0.1", port))
    s.listen()
    return s


@case()
def a_taken_port_is_a_clear_error(t: T):
    """UC-033: --listen on a taken port: exit 1 within seconds, the error names the port and the way out."""
    holder = taken()
    port = holder.getsockname()[1]
    dir = t.scratch("tilrd")
    started = time.monotonic()
    r = t.run(SERVER_EXE, "--listen", f"127.0.0.1:{port}", env=t.server_env(dir), timeout=20)
    t.metric("exit_ms", round((time.monotonic() - started) * 1000))
    t.eq(r.code, 1, "the exit status")
    t.has(r.err, f"127.0.0.1:{port} is taken: another tilrd, or another program, listens there",
          "--listen", "--no-listen")
    t.lacks(r.err, "panicked")
    t.ok(not (dir / "runtime" / "tilrd.sock").exists(), "no socket left")


@case()
def a_bare_tilrd_next_to_a_running_one_says_the_default_port_is_taken(t: T):
    """UC-033: `tilrd` while the user's tilrd holds 127.0.0.1:7890: exit 1, the port named."""
    user = t.user()
    t.eq(user.run("tilr", "new", "-d").code, 0, "tilr new -d")
    r = user.run("tilrd", timeout=20)
    t.eq(r.code, 1, f"the exit status:\n{r.text}")
    t.has(r.err, "127.0.0.1:7890 is taken")


@case()
def a_tilrd_tilr_starts_goes_on_without_the_taken_port(t: T):
    """UC-033: the default port is held by another program: `tilr` still starts tilrd, which warns and serves its socket."""
    holder = taken(7890)
    user = t.user()
    r = user.run("tilr", "new", "-d")
    t.eq(r.code, 0, f"tilr new -d:\n{r.text}")
    log = (Path("/tmp") / f"tilr-{user.uid}" / "tilrd.log").read_text()
    t.has(log, "127.0.0.1:7890 is taken", "serving the Unix socket only")
    t.lacks(log, "listening on ws://")
    t.eq(len(user.run("tilr", "ls", "-q").out.split()), 1, "tilr works")
    holder.close()
