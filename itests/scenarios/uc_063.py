"""UC-063: a client stays attached while the server updates: it reconnects on its own and
what the user types meanwhile is neither lost nor doubled.

The client (auto reconnect) types numbered lines into `cat >>typed` in a pane, one every
20 ms, through the whole update (from the CDN container); a line whose send fails (no
connection) is typed again once the client is back, as a client queues what the user
types while it reconnects. Afterwards every line typed must be in the file once, in
order. A line the driver handed to its socket before the server exec'd, but that the
server never read, is what "lost" means here: the update closes client connections (the
sockets are not carried over), so whatever sat unread in them goes with them.
"""

import threading
import time

from tt import T, case
from tt.live import reload_times


@case(fixture="server_state", services=["cdn"], timeout=300, spec=[
    "reload/client-connections-end-and-clients-reconnect",
    "update/live-programs-survive-an-update",
])
def a_client_typing_through_an_update_reconnects_and_nothing_typed_is_lost_or_doubled(t: T):
    """UC-063: typing every 20 ms through an update; the client reconnects itself; each line arrives once, in order."""
    manifest = t.manifest()
    exe = t.install()
    server = t.server(exe=exe, state=t.root / "state")
    d = t.driver()
    d.connect(server)
    d.cmd("keepalive", interval_ms=500, timeout_ms=3000, auto_reconnect=True, reconnect_timeout_ms=30_000)
    pane = d.shell_pane()
    typed = t.scratch("typed") / "lines"
    d.line(pane, f"stty -echo; cat >>{typed}")
    time.sleep(0.5)

    update = {}

    def run_update():
        update["r"] = server.cmd("update", "--cdn", t.service("cdn"), timeout=180)
    since = d.mark()
    n, retried, sent, back_at = 0, 0, [], None
    worker = threading.Thread(target=run_update)
    worker.start()
    deadline = time.monotonic() + 120
    while time.monotonic() < deadline:
        n_next = n + 1
        answer = d.cmd("input", pane=pane, text=f"line {n_next}\r", ok=False, quiet=True)
        if answer.get("ev") == "error":
            retried += 1
            time.sleep(0.02)
            continue
        n = n_next
        sent.append(n)
        if back_at is None and any(e.get("ev") == "reconnected" for e in d.events[since:]):
            back_at = time.monotonic()
        if back_at is not None and time.monotonic() - back_at > 2 and not worker.is_alive():
            break
        time.sleep(0.02)
    worker.join()
    t.eq(update["r"].code, 0, "the update succeeds")
    back = d.wait_event("reconnected", since=since, timeout=5)
    t.metric("reconnect_ms", back["ms"])
    t.metric("reload_ms", (reload_times(server) or [0])[-1])
    t.metric("lines_typed", len(sent))
    t.metric("sends_refused_while_away", retried)
    t.eq(back["server_version"], manifest["next_version"], "the client is back on the new version")

    d.key(pane, "ctrl-d")
    d.wait_for(pane, r"(?m)^\$ ?$")
    got = [int(l.split()[1]) for l in typed.read_text().splitlines() if l.startswith("line ")]
    doubled = sorted({x for x in got if got.count(x) > 1})
    lost = sorted(set(sent) - set(got))
    t.metric("lines_lost", len(lost))
    t.check("no line arrived twice", lambda: t.eq(doubled, [], "doubled lines"))
    t.check("the lines arrived in the order typed", lambda: t.eq(got, sorted(got), "order"))
    t.check("no line typed was lost", lambda: t.eq(lost, [], "lines typed but never arrived"))
    t.check("input works after the update", lambda: t.eq(d.run(pane, "echo after"), 0, "status"))
