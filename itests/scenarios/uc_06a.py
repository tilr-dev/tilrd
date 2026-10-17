"""UC-06A: a server from before the rename to tilrd, installed as `tilr-server`, updates
to tilrd with panes open. The old server asks the CDN for `tilr-server-<platform>`;
releases are published under both names (scripts/release.sh), so it finds the new one
under the name it asks for, puts it in place of its own executable, which keeps its file
name (and whatever starts it: a unit, a script), and reloads into it: same pid, same
listening socket, every program in the panes alive. From then on it is tilrd, and its
next update finds a release offered under the new name only.

The old server is the last commit named tilr-server (PREVIOUS in itests/dt), built as
version 0.0.9 (/opt/tt/tilr-server-previous). Without that tree the case is BLOCKED.
"""

import os

from tt import NEXT_EXE, PREVIOUS_EXE, SERVER_EXE, Blocked, T, case
from tt.live import LivePanes, reload_times, wait_reloads

PROGRAMS = ("shell", "counter", "vim", "python", "sleep", "tcp")


@case(fixture="server_state", services=["echo"], timeout=420)
def a_server_named_tilr_server_updates_to_tilrd_in_place_and_keeps_its_panes(t: T):
    """UC-06A: the old tilr-server updates to tilrd (released under both names), then tilrd updates on."""
    if not os.path.exists(PREVIOUS_EXE):
        raise Blocked(f"no {PREVIOUS_EXE}: ./dt builds it from the commit PREVIOUS")
    manifest = t.manifest()
    current, nxt = manifest["version"], manifest["next_version"]
    old = t.run(PREVIOUS_EXE, "--version", quiet=True, check=True).out.split()
    t.eq(old[0], "tilr-server", "the old server's name")
    cdn = t.cdn()
    cdn.release(current, t.run("which", SERVER_EXE, quiet=True).out.strip(), names=("tilrd", "tilr-server"))
    exe = t.install(PREVIOUS_EXE, name="tilr-server")
    server = t.server(exe=exe, state=t.root / "state")
    d = t.driver()
    d.connect(server)
    live = LivePanes(t, d, server, echo=t.service("echo"), which=PROGRAMS)

    steps = [("across the rename", current, ("tilrd", "tilr-server")), ("as tilrd", nxt, ("tilrd",))]
    for n, (what, version, names) in enumerate(steps, 1):
        if n > 1:
            cdn.release(version, NEXT_EXE, names=names)
        since = d.mark()
        r = server.cmd("update", "--cdn", cdn.url, timeout=180)
        t.eq(r.code, 0, f"the update {what} exits 0")
        t.has(r.out, f"-> {version}")
        wait_reloads(t, server, n)
        d.wait_event("disconnected", since=since, timeout=10)
        back = d.cmd("reconnect", timeout_ms=20_000)
        live.wait_settled()
        t.check(f"the server runs {version} after the update {what}",
                lambda: (t.eq(back["server_version"], version, "HelloAck.server_version"),
                         t.eq(server.version(), version, "the executable's version")))
        t.check(f"the executable is tilrd under its old name after the update {what}",
                lambda: (t.eq(exe.name, "tilr-server", "file name"),
                         t.eq(t.run(exe, "--version", quiet=True).out.split()[0], "tilrd", "name it gives")))
        t.check(f"the server keeps its pid and listening socket after the update {what}",
                lambda: t.eq(t.listener_pid(server.port), server.pid, "listener pid"))
        live.check(f"the update {what}")
    t.metric("reload_ms", reload_times(server))
    live.finish()
