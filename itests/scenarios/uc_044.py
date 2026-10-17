"""UC-044: resurrection: the old connection is dropped (DROP) for 10 s; the client has
already reconnected when it comes back to life. Exactly one live connection remains.

The layer-1 proxy freezes only the client's first connection, so the reconnect (a new
connection) gets through while the old one is still held; after 10 s the old one flows
again, with whatever the client sent into it. tilrd runs with its default client timeout
(30 s), longer than the break: what ends the old connection is the client's reconnect,
whose Hello (the same client_id and token) supersedes it at once, so there is one viewer
again as soon as the client is back, not two until the revival or the timeout.

Two clients: one closes the socket it gave up on (its close is held by the network and
arrives with the revival); one leaves it open and never reads it again, as a suspended
app does. Either way the server closed its side at the reconnect, and input held in the
old connection never runs.
"""

import re
import time

from tt import T, Counter, case

KEEPALIVE_MS, DEAD_AFTER_MS = 1000, 4000
BREAK_S = 10
CLIENT_TIMEOUT_S = 15


def resurrection(t: T, abandon: str, server_args=()):
    server = t.server(*server_args, state=t.root / "state")
    proxy = t.proxy(server.port)
    observer = t.driver("observer")
    observer.connect(server)
    client = t.driver("client")
    client.connect(proxy.url, token=server.token())
    pane = client.shell_pane(100, 30)
    counter = Counter(t, client, pane)
    client.wait_for(pane, r"(?m)^tick 20$")
    shell = client.shell_pane(100, 10)
    marks = t.scratch("marks") / "marks"
    client.cmd("keepalive", interval_ms=KEEPALIVE_MS, timeout_ms=DEAD_AFTER_MS, auto_reconnect=True,
               reconnect_timeout_ms=60_000, abandon=abandon)

    since = client.mark()
    frozen = time.monotonic()
    proxy.freeze(conn=1)
    # Typed into the dead connection before the client knows: held by the network.
    client.input(shell, f"echo held >>{marks}\r")
    back = client.wait_event("reconnected", since=since, timeout=20)
    t.metric("client_back_ms", round((back["_at"] - frozen) * 1000))
    t.eventually(lambda: observer.viewers(pane) == 1, "the reconnect supersedes the held connection: one viewer",
                 timeout=1, interval=0.02)
    t.metric("ghost_viewer_ms", round((time.monotonic() - back["_at"]) * 1000))
    t.ok(server.logged("client superseded by a newer connection") == 1, "the server superseded the old connection")
    counter.check_screen(client, what="the screen of the new connection")
    t.eq(observer.viewers(pane), 1, "no ghost of the held connection")
    time.sleep(max(0.0, frozen + BREAK_S - time.monotonic()))

    revived = time.monotonic()
    proxy.resume(conn=1)
    return server, proxy, observer, client, pane, counter, marks, revived


@case(fixture="server_state")
def a_revived_connection_the_client_closed_ends_at_once_leaving_one_live_connection(t: T):
    """UC-044: freeze the first connection 10 s; the client reconnects; on revival the old one ends, one viewer."""
    server, proxy, observer, client, pane, counter, marks, revived = resurrection(t, "close")
    t.eventually(lambda: observer.viewers(pane) == 1, "the revived connection ends, one viewer left", timeout=5,
                 interval=0.05)
    t.metric("one_live_after_revival_ms", round((time.monotonic() - revived) * 1000))
    time.sleep(3)
    t.eq(observer.viewers(pane), 1, "still exactly one viewer")
    t.ok(sum(1 for s in proxy.stats() if not s["closed"]) == 1, "one connection open through the network")
    held = marks.read_text().count("held") if marks.exists() else 0
    t.metric("held_input_delivered_late", held)
    t.ok(held <= 1, f"input held in the dead connection arrives at most once ({held})")
    counter.check_screen(client)
    counter.check_log()
    client.interrupt(pane)
    t.eq(client.run(pane, "echo typed-after"), 0, "input works on the live connection")
    t.eq(len(re.findall(r"(?m)^typed-after$", client.text(pane))), 1, "its output appears once")


@case(fixture="server_state")
def a_revived_connection_left_open_by_the_client_was_superseded_by_its_reconnect(t: T):
    """UC-044: same, the client leaves the old socket open and unread; its reconnect superseded it, one viewer."""
    server, proxy, observer, client, pane, counter, marks, revived = resurrection(
        t, "leak", ("--client-timeout", CLIENT_TIMEOUT_S))
    t.eventually(lambda: observer.viewers(pane) == 1, "one viewer after the revival", timeout=5, interval=0.1)
    t.metric("one_live_after_revival_ms", round((time.monotonic() - revived) * 1000))
    time.sleep(3)
    t.eq(observer.viewers(pane), 1, "still exactly one viewer")
    t.eq(server.logged("client went silent, dropping"), 0, "no timeout needed")
    held = marks.read_text().count("held") if marks.exists() else 0
    t.metric("held_input_delivered_late", held)
    t.ok(held <= 1, f"input held in the dead connection arrives at most once ({held})")
    counter.check_screen(client)
    counter.check_log()
    client.interrupt(pane)
    t.eq(client.run(pane, "echo typed-after"), 0, "input works on the live connection")
