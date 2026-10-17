"""UC-067: a new version that reads another format of the reload state is installed: the
server does not reload into it (it would misread the panes and lose every shell), keeps
running the old code with all its panes, and says so in its log.

The release is a wrapper around the next build (a CDN of the case's own serves it) that
answers `state-formats` with formats one higher than this server writes and passes
everything else to the next build under the same argv[0]: it runs (`--version`, so the
update installs it) but cannot take over (crates/tilr-server/src/reload.rs,
check_executable). Panes started after it is installed run the executable on disk, the
new one, under the old supervisor; they must work too.
"""

import re
import signal
import time

from tt import NEXT_EXE, T, case
from tt.live import LivePanes

WRAPPER = """#!/bin/bash
if [ "$1" = state-formats ]; then
    printf 'supervisor-state 99\\npane-state 99\\n'
    exit 0
fi
exec -a "$0" {next} "$@"
"""


@case(fixture="server_state", services=["echo"])
def a_version_with_another_state_format_is_not_reloaded_into_and_the_shells_live_on(t: T):
    """UC-067: the update installs a release that reads another state format: no reload, panes and old version kept."""
    manifest = t.manifest()
    release = t.scratch("release") / "tilr-server"
    release.write_text(WRAPPER.format(next=NEXT_EXE))
    release.chmod(0o755)
    cdn = t.cdn()
    cdn.release("9.9.9-otherformat", release)
    exe = t.install()
    server = t.server(exe=exe, state=t.root / "state")
    d = t.driver()
    d.connect(server)
    live = LivePanes(t, d, server, echo=t.service("echo"), which=("shell", "counter", "vim", "python", "tcp"))

    r = server.cmd("update", "--cdn", cdn.url, timeout=120)
    t.eq(r.code, 0, "the update installs the release")
    t.eq(server.version(), manifest["next_version"], "the executable on disk is the new one")
    t.eventually(lambda: server.logged("not reloading"), "the server refuses to reload into it", timeout=15)
    t.has(server.log_text(), "supervisor-state format 99")
    time.sleep(2)
    t.check("the server did not reload", lambda: t.eq(server.logged("resumed after reload"), 0, "reloads"))
    t.check("the client stays connected", lambda: t.ok(d.connected(), "connected"))
    t.check("the server still says the old version",
            lambda: t.eq(t.driver("version").connect(server)["server_version"], manifest["version"],
                         "HelloAck.server_version"))
    live.check("the refused reload")

    def new_pane_works():
        pane = d.shell_pane()
        t.eq(d.run(pane, "echo in-a-new-pane"), 0, "a command in a pane started after the refusal")
        d.cmd("close_pane", pane=pane)
    t.check("a pane started after the refusal works", new_pane_works)

    def sigusr1_refused_too():
        server.signal(signal.SIGUSR1)
        t.eventually(lambda: server.logged("not reloading") >= 2, "SIGUSR1 is refused the same way", timeout=15)
        t.eq(server.logged("resumed after reload"), 0, "reloads")
    t.check("an explicit reload (SIGUSR1) is refused too", sigusr1_refused_too)
    live.finish()
    t.check("nothing was lost in the log",
            lambda: t.eq(re.findall(r"pane lost|panicked", server.log_text()), [], "bad log lines"))
