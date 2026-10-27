"""UC-034: the daemon's Unix socket for local clients is the user's alone: the socket 0600,
its directory 0700, and a client of another uid cannot connect. Where the permissions
were opened by hand, the server still checks the peer's uid and turns it away.
"""

import os
import stat
from pathlib import Path

from tt import CLIENT_EXE, T, case

NOBODY = 65534


@case(spec=[
    "access-control/the-unix-socket-checks-the-peer-s-uid",
    "daemon-lifecycle/runtime-directory-and-socket-modes",
    "tilr-cli/tilr-checks-who-serves-the-socket",
])
def another_user_cannot_use_the_socket(t: T):
    """UC-034: 0600 socket in a 0700 directory; uid 65534 is refused, by the permissions and by the server."""
    t.need("procfs", "root")
    env = t.user_env()
    run = Path(env["XDG_RUNTIME_DIR"]) / "tilr"
    socket = run / "tilrd.sock"
    t.tilr("new", "-d", env=env, check=True)
    t.eq(stat.S_IMODE(run.stat().st_mode), 0o700, "the directory is 0700")
    t.eq(stat.S_IMODE(socket.stat().st_mode), 0o600, "the socket is 0600")
    t.eq(stat.S_IMODE((run / "tilrd.sock.lock").stat().st_mode), 0o600, "the lock is 0600")

    other = {"HOME": "/nonexistent", "XDG_CONFIG_HOME": "/nonexistent", "TERM": "xterm-256color"}

    def as_nobody(*args):
        return t.sh(f"exec setpriv --reuid={NOBODY} --regid={NOBODY} --clear-groups "
                    f"{CLIENT_EXE} --socket {socket} {' '.join(args)}", env=other)

    r = as_nobody("ls")
    t.ok(r.code != 0, "another user cannot reach the socket")
    t.has(r.err, "Permission denied")

    # Opened by hand: the kernel lets it connect, the server turns it away.
    os.chmod(Path(env["XDG_RUNTIME_DIR"]), 0o755)
    os.chmod(run, 0o755)
    os.chmod(socket, 0o666)
    r = as_nobody("ls")
    t.ok(r.code != 0, f"the server refuses another uid: {r.text}")
    # And tilr refuses a server of another uid (UC-03A) on its side, at once.
    t.has(r.err, f"is served by another user (uid {os.geteuid()})")
    t.eventually(lambda: "refused a connection on the Unix socket" in (run / "tilrd.log").read_text(),
                 "the server logs the refusal")
    t.has((run / "tilrd.log").read_text(), f"uid {NOBODY}")
    # tilr enters its runtime directory only while it is private again.
    r = t.tilr("ls", "-q", env=env)
    t.ok(r.code != 0, "a runtime directory open to others is refused")
    t.has(r.err, "open to other users (mode 0755)")
    os.chmod(run, 0o700)
    t.eq(t.tilr("ls", "-q", env=env, check=True).out.count("\n"), 1, "the user still gets in")
