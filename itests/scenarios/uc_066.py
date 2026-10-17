"""UC-066: roll back to the old version and forward again with panes open: the current
release updates to the next (the CDN container), back to the current one (a CDN of the
case's own that names it the latest: `tilrd update` installs whatever the CDN
calls latest), and forward again. Each step is a reload into another executable; after
each the server says its version, keeps its pid and every program in its panes.
"""

import time

from tt import SERVER_EXE, T, case
from tt.live import LivePanes, reload_times, wait_reloads

PROGRAMS = ("shell", "counter", "vim", "less", "python", "sleep", "tcp")


@case(fixture="server_state", services=["cdn", "echo"], timeout=480)
def rolling_back_and_forward_again_keeps_the_panes_and_their_programs(t: T):
    """UC-066: current -> next -> current -> next with panes open; versions as asked, programs alive throughout."""
    manifest = t.manifest()
    current, nxt = manifest["version"], manifest["next_version"]
    old_cdn = t.cdn()
    old_cdn.release(current, t.run("which", SERVER_EXE, quiet=True).out.strip())
    exe = t.install()
    server = t.server(exe=exe, state=t.root / "state")
    d = t.driver()
    d.connect(server)
    live = LivePanes(t, d, server, echo=t.service("echo"), which=PROGRAMS)

    steps = [("forward", t.service("cdn"), nxt), ("back", old_cdn.url, current), ("forward again", t.service("cdn"), nxt)]
    for n, (what, cdn, version) in enumerate(steps, 1):
        since = d.mark()
        r = server.cmd("update", "--cdn", cdn, timeout=180)
        t.eq(r.code, 0, f"the update {what} exits 0")
        t.has(r.out, f"-> {version}")
        wait_reloads(t, server, n)
        d.wait_event("disconnected", since=since, timeout=10)
        back = d.cmd("reconnect", timeout_ms=20_000)
        live.wait_settled()
        t.check(f"the server runs {version} after the update {what}",
                lambda: (t.eq(back["server_version"], version, "HelloAck.server_version"),
                         t.eq(server.version(), version, "the executable's version")))
        t.check(f"the server keeps its listening socket after the update {what}",
                lambda: t.eq(t.listener_pid(server.port), server.pid, "listener pid"))
        live.check(f"the update {what}")
        time.sleep(1)
    t.metric("reload_ms", reload_times(server))
    live.finish()
