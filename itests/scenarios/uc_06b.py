"""UC-06B: a server `tilr` started (with `--socket` and `--lock-fd`) has its executable
replaced by a release that crashes resuming: the reload is refused, and the socket with its
lock stays served, so `tilr` finds the same server and never starts a second one.

The release is a wrapper around the next build that crashes (SIGSEGV) in the check run
a reloading server makes of it (TILR_RELOAD_CHECK set), as in UC-06A. `tilr` and `tilrd`
are copies in a directory of the case's own: `tilr` starts the `tilrd` next to it, which
the case replaces the way an install does (write next to it, rename into place).
"""

import os
import shutil
import signal
import stat
from pathlib import Path

from tt import CLIENT_EXE, NEXT_EXE, SERVER_EXE, T, case

WRAPPER = """#!/bin/bash
if [ -n "$TILR_RELOAD_CHECK" ]; then
    echo "this release crashes resuming" >&2
    kill -SEGV $$
fi
exec -a "$0" {next} "$@"
"""


@case(spec=["reload/a-refused-reload-keeps-the-socket-serving"])
def a_refused_reload_keeps_the_socket_and_tilr_finds_the_same_server(t: T):
    """UC-06B: refused reload of a tilr-started server: socket and lock kept, tilr ls and a new tilr reach the same tilrd."""
    t.need("procfs")
    env = t.user_env()
    socket = Path(env["XDG_RUNTIME_DIR"]) / "tilr" / "tilrd.sock"
    bin = t.scratch("bin")
    shutil.copy2(shutil.which(SERVER_EXE) or SERVER_EXE, bin / "tilrd")
    shutil.copy2(shutil.which(CLIENT_EXE) or CLIENT_EXE, bin / "tilr")

    first = t.session(str(bin / "tilr"), env=env, name="first")
    first.prompt(timeout=15)
    first.write("export KEPT=yes\r")
    first.prompt(timeout=10)
    daemons = t.tilrd_daemons()
    t.eq(len(daemons), 1, "one tilrd")
    pid = daemons[0].pid
    t.has(daemons[0].cmdline, "--lock-fd")
    t.has(daemons[0].cmdline, "--socket")

    staged = bin / "tilrd.new"
    staged.write_text(WRAPPER.format(next=NEXT_EXE))
    staged.chmod(0o755)
    staged.rename(bin / "tilrd")
    os.kill(pid, signal.SIGUSR1)
    log = socket.parent / "tilrd.log"
    t.eventually(lambda: "not reloading: the new executable could not resume" in log.read_text(),
                 "the server refuses to exec into the new file", timeout=30)
    t.has(log.read_text(), "this release crashes resuming")
    t.eq([p.pid for p in t.tilrd_daemons()], [pid], "the same tilrd, no other")
    t.ok(stat.S_ISSOCK(socket.lstat().st_mode), "the socket is in place")

    first.write('echo "still-$((40+2))"\r')
    first.expect("still-42", timeout=10)
    r = t.tilr("ls", "-q", env=env, check=True)
    t.eq(len(r.out.split()), 1, "tilr ls lists the pane through the socket")

    second = t.session(str(bin / "tilr"), env=env, name="second")
    second.prompt(timeout=15)
    second.write('echo "kept=$KEPT"\r')
    second.expect("kept=yes", timeout=10)
    t.eq([p.pid for p in t.tilrd_daemons()], [pid], "still the one tilrd: the new tilr started none")
    t.eq("resumed after reload" in log.read_text(), False, "no reload happened")
