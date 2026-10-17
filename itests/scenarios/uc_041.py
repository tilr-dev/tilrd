"""UC-041: asymmetric cut: only server -> client stops, or only client -> server.

A one-way cut breaks a naive heartbeat: the side that still hears the other thinks the
connection is fine. The client (tilr-driver) reaches tilrd through the layer-1 proxy,
which freezes one direction; an observer connected directly counts the pane's viewers.
The server runs with --client-timeout 6 (a WebSocket ping every 2 s).

Observed (FAIL, product): with server -> client cut and a client that keeps sending
(its pings, its input) without noticing it hears nothing, the server never drops it.
supervisor.rs (serve_client, the `client_timeout` read loop) counts any message from the
client as a sign of life, so the client's own pings keep a connection alive whose
direction towards the client is dead; the server's WebSocket pings are never answered and
nothing looks at that. Frames pile up in the kernel buffers and the client queue
(CLIENT_QUEUE = 1024 packets) for minutes before "client too slow" would drop it.
"""

import time

from tt import T, Counter, case

CLIENT_TIMEOUT_S = 6
KEEPALIVE_MS, DEAD_AFTER_MS = 1000, 4000


def setup(t: T):
    server = t.server("--client-timeout", CLIENT_TIMEOUT_S, state=t.root / "state")
    proxy = t.proxy(server.port)
    observer = t.driver("observer")
    observer.connect(server)
    client = t.driver("client")
    client.connect(proxy.url, token=server.token())
    pane = client.shell_pane(100, 30)
    counter = Counter(t, client, pane)
    client.wait_for(pane, r"(?m)^tick 20$")
    t.eq(observer.viewers(pane), 1, "viewers before")
    return server, proxy, observer, client, pane, counter


def back_to_normal(t: T, proxy, observer, client, pane: str, counter: Counter, since: int):
    resumed = time.monotonic()
    proxy.resume()
    back = client.wait_event("reconnected", since=since, timeout=20)
    t.metric("back_after_resume_ms", round((back["_at"] - resumed) * 1000))
    t.ok(back["_at"] - resumed <= 5, f"the client is back within 5s of the network (took {back['_at'] - resumed:.1f}s)")
    counter.check_screen(client)
    t.eventually(lambda: observer.viewers(pane) == 1, "exactly one viewer once the client is back", timeout=10)
    time.sleep(CLIENT_TIMEOUT_S / 2)
    t.eq(observer.viewers(pane), 1, "no ghost viewer from the old connection")
    counter.check_log()
    client.key(pane, "ctrl-c")
    t.eq(client.run(pane, "echo typed-after-return"), 0, "input works after the return")


@case(fixture="server_state")
def a_cut_of_server_to_client_only_is_noticed_by_the_client_and_both_sides_let_go(t: T):
    """UC-041: freeze server -> client; the client notices the silence, the server drops it, it comes back."""
    server, proxy, observer, client, pane, counter = setup(t)
    client.cmd("keepalive", interval_ms=KEEPALIVE_MS, timeout_ms=DEAD_AFTER_MS, auto_reconnect=True,
               reconnect_timeout_ms=60_000)
    since = client.mark()
    frozen = time.monotonic()
    proxy.freeze("down")
    dead = client.wait_event("dead", since=since, timeout=30)
    t.metric("client_detect_ms", round((dead["_at"] - frozen) * 1000))
    t.ok(dead["_at"] - frozen <= (DEAD_AFTER_MS + KEEPALIVE_MS) / 1000 + 1.5,
         f"the client noticed within a few pings (took {dead['_at'] - frozen:.1f}s)")
    t.eventually(lambda: observer.viewers(pane) == 0, "the server drops the client", timeout=CLIENT_TIMEOUT_S + 3,
                 interval=0.1)
    t.metric("server_drop_ms", round((time.monotonic() - frozen) * 1000))
    time.sleep(1)
    t.ok(not any(e.get("ev") == "reconnected" for e in client.events[since:]),
         "no connection gets through while the server's replies cannot")
    back_to_normal(t, proxy, observer, client, pane, counter, since)


@case(fixture="server_state")
def a_client_that_still_talks_but_no_longer_hears_is_dropped_by_the_server(t: T):
    """UC-041: freeze server -> client under a client that keeps pinging without listening; the server must drop it."""
    server, proxy, observer, client, pane, counter = setup(t)
    # Pings every second, never gives up on its own: what the server sees of a client
    # whose incoming direction is gone and that has not noticed yet.
    client.cmd("keepalive", interval_ms=KEEPALIVE_MS, timeout_ms=600_000)
    frozen = time.monotonic()
    proxy.freeze("down")
    limit = CLIENT_TIMEOUT_S + 3
    deadline = frozen + 20
    while observer.viewers(pane) != 0 and time.monotonic() < deadline:
        time.sleep(0.2)
    took = time.monotonic() - frozen
    dropped = observer.viewers(pane) == 0
    t.metric("server_drop_ms", round(took * 1000) if dropped else -1)
    stats = proxy.stats()[0]
    t.metric("client_messages_reaching_server", stats["ws_up"])
    t.check("the server's own pings go unanswered, so it drops the client within "
            f"{limit}s (dropped: {dropped}, after {took:.1f}s)", lambda: t.ok(dropped and took <= limit, "dropped"))
    proxy.resume()
    counter.check_log()


@case(fixture="server_state")
def a_cut_of_client_to_server_only_is_noticed_by_the_server_and_by_a_client_that_counts_pongs(t: T):
    """UC-041: freeze client -> server; the server drops the silent client, the client misses its pongs; it comes back."""
    server, proxy, observer, client, pane, counter = setup(t)
    # Frames keep coming, so only the answers to its own pings tell the client.
    client.cmd("keepalive", interval_ms=KEEPALIVE_MS, timeout_ms=DEAD_AFTER_MS, auto_reconnect=True,
               reconnect_timeout_ms=60_000, mode="pong")
    since = client.mark()
    frozen = time.monotonic()
    proxy.freeze("up")
    t.eventually(lambda: observer.viewers(pane) == 0, "the server drops the client that went silent",
                 timeout=CLIENT_TIMEOUT_S + 3, interval=0.1)
    t.metric("server_drop_ms", round((time.monotonic() - frozen) * 1000))
    t.ok(server.logged("client went silent, dropping") >= 1, "the server logged why it dropped the client")
    # Whichever comes first: the client's own verdict, or the server's close reaching it.
    dead = client.wait_event(("dead", "disconnected"), since=since, timeout=30)
    t.metric("client_detect_ms", round((dead["_at"] - frozen) * 1000))
    t.ok(dead["_at"] - frozen <= (DEAD_AFTER_MS + KEEPALIVE_MS) / 1000 + 1.5,
         f"the client noticed its pings go unanswered (took {dead['_at'] - frozen:.1f}s)")
    time.sleep(1)
    back_to_normal(t, proxy, observer, client, pane, counter, since)
