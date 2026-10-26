"""UC-06A: a new version that crashes as it resumes is installed: the server does not exec
into it (that would end the server and every shell), keeps running the old code with all
its panes, and logs why.

The release's tilrd is a wrapper around the next build (a CDN of the case's own serves
the archive, with the next tilr). It runs (`--version`, so the update installs it) and
says it reads this server's state (`state-formats` is the next build's), but it crashes
(SIGSEGV) whenever it resumes: in the check run a reloading server makes of it
(TILR_RELOAD_CHECK set) and, were the server to exec it, in the live resume
(`server-<pid>.state` waiting for it). Before the reload check
(crates/tilrd/src/reload.rs, check_successor) this update ended the server and its
shells.
"""

import re
import signal
import time

from tt import CLIENT_NEXT_EXE, NEXT_EXE, T, case
from tt.live import LivePanes

WRAPPER = """#!/bin/bash
if [ -n "$TILR_RELOAD_CHECK" ] || [ -e "$TILR_RUNTIME_DIR/server-$$.state" ]; then
    echo "this release crashes resuming" >&2
    kill -SEGV $$
fi
exec -a "$0" {next} "$@"
"""


@case(fixture="server_state", services=["echo"],
      spec=["reload/the-successor-shows-it-can-resume-before-the-exec"])
def a_version_that_crashes_resuming_is_not_reloaded_into_and_the_shells_live_on(t: T):
    """UC-06A: the update installs a release whose tilrd crashes resuming: no exec, server, pid, panes and old version kept."""
    manifest = t.manifest()
    release = t.scratch("release") / "tilrd"
    release.write_text(WRAPPER.format(next=NEXT_EXE))
    release.chmod(0o755)
    cdn = t.cdn()
    cdn.publish("9.9.9-crashes", server=release, client=CLIENT_NEXT_EXE)
    exe = t.install()
    server = t.server(exe=exe, state=t.root / "state")
    pid = server.pid
    d = t.driver()
    d.connect(server)
    live = LivePanes(t, d, server, echo=t.service("echo"), which=("shell", "counter", "vim", "python", "tcp"))

    r = server.cmd("update", "--cdn", cdn.url, timeout=120)
    t.eq(r.code, 0, "the update installs the release")
    t.eq(server.version(), manifest["next_version"], "the executable on disk is the new one")
    t.eventually(lambda: server.logged("not reloading: the new executable could not resume"),
                 "the server refuses to exec into it", timeout=30)
    t.has(server.log_text(), "this release crashes resuming")
    time.sleep(2)
    t.check("the server did not reload", lambda: t.eq(server.logged("resumed after reload"), 0, "reloads"))
    t.check("the same server process", lambda: (t.ok(server.alive(), "alive"), t.eq(server.pid, pid, "pid"),
                                                t.eq(t.listener_pid(server.port), pid, "listener pid")))
    t.check("the client stays connected", lambda: t.ok(d.connected(), "connected"))
    t.check("the server still says the old version",
            lambda: t.eq(t.driver("version").connect(server)["server_version"], manifest["version"],
                         "HelloAck.server_version"))
    t.check("no state or check file is left",
            lambda: t.eq(sorted(p.name for p in (server.dir / "runtime").rglob("*.state*")), [], "state files"))
    live.check("the refused reload")

    def sigusr1_refused_too():
        server.signal(signal.SIGUSR1)
        t.eventually(lambda: server.logged("not reloading: the new executable could not resume") >= 2,
                     "SIGUSR1 is refused the same way", timeout=30)
        t.eq(server.logged("resumed after reload"), 0, "reloads")
        t.ok(server.alive(), "alive")
    t.check("an explicit reload (SIGUSR1) is refused too", sigusr1_refused_too)
    live.check("the second refused reload")
    live.finish()
    t.check("nothing was lost in the log",
            lambda: t.eq(re.findall(r"pane lost|pane process died", server.log_text()), [], "bad log lines"))
