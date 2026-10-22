"""UC-063: a client stays attached while the server updates: it reconnects on its own and
what the user types meanwhile is not lost and keeps its order.

The client retains admitted input until the PTY acknowledges it and replays it
on reconnect. Every numbered line, including lines admitted while disconnected,
must arrive in order; socket-write success alone is no receipt. Reload discards
delivery history, so applied input whose acknowledgement was lost may arrive
again. Check the order of first appearances while allowing those replays.
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
    """UC-063: typing through an update; reconnect preserves input and its order, allowing replay after reload."""
    manifest = t.manifest()
    exe = t.install()
    server = t.server(exe=exe, state=t.root / "state")
    d = t.driver()
    d.connect(server)
    d.cmd("keepalive", interval_ms=500, timeout_ms=3000, auto_reconnect=True, reconnect_timeout_ms=30_000)
    pane = d.shell_pane()
    typed = t.scratch("typed") / "lines"
    d.line(pane, f"stty -echo; printf 'input-%s\\n' ready; cat >>{typed}")
    d.wait_for(pane, "input-ready")

    update = {}

    def run_update():
        update["r"] = server.cmd("update", "--cdn", t.service("cdn"), timeout=180)
    since = d.mark()
    n, sent, back_at = 0, [], None
    worker = threading.Thread(target=run_update)
    worker.start()
    deadline = time.monotonic() + 120
    while time.monotonic() < deadline:
        n_next = n + 1
        d.cmd("input", pane=pane, text=f"line {n_next}\r", quiet=True)
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
    t.eq(back["server_version"], manifest["next_version"], "the client is back on the new version")

    d.key(pane, "ctrl-d")
    d.wait_for(pane, r"(?m)^\$ ?$")
    got = [int(l.split()[1]) for l in typed.read_text().splitlines() if l.startswith("line ")]
    first_arrivals = list(dict.fromkeys(got))
    lost = sorted(set(sent) - set(got))
    t.metric("lines_lost", len(lost))
    t.metric("lines_replayed", len(got) - len(first_arrivals))
    t.check("the lines first arrived in the order typed", lambda: t.eq(first_arrivals, sent, "first arrivals"))
    t.check("no line typed was lost", lambda: t.eq(lost, [], "lines typed but never arrived"))
    t.check("input works after the update", lambda: t.eq(d.run(pane, "echo after"), 0, "status"))
