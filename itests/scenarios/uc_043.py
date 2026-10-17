"""UC-043: degradation without a break: 30% packet loss and 500 ms ± 200 ms delay each way.
The session is not torn down on every lag, and it does not hang forever either.

Layer 2: the client runs in a container of its own ("phone"); netem shapes what it sends
to tilrd and what tilrd sends to it (on the case container's own interface, only the
server port's packets to the phone), so the observer and the case's control traffic are
not touched. tilrd keeps its default client timeout (30 s); the client pings every 2 s,
gives up after 15 s of silence and allows 10 s per reconnect attempt, as a real client
on such a network would. For 60 s the case watches the counter on the client's screen
and types into a second pane, then the network heals.

What TCP makes of 30% loss each way dominates what the client sees: retransmission
backoff stalls the screen for tens of seconds, a reconnect may need several attempts, and
the frames the server queues behind lost packets push input latency to 5-20 s (the
server sends every frame to a lagging client; it neither merges them nor skips to the
latest screen). Those are recorded as metrics; the assertions are what the server owes:
no drop of a client that is only slow, at most a few disconnects, one viewer and a
whole screen once the network heals.
"""

import time

from tt import T, Counter, case

NETEM = "delay 500ms 200ms distribution normal loss 30%"
WINDOW_S = 60


@case(fixture="server_state", hosts=["phone"], net_admin=True, timeout=400)
def a_lossy_slow_network_slows_the_session_without_tearing_it_down_or_hanging_it(t: T):
    """UC-043: 30% loss + 500±200 ms both ways for a minute: few disconnects, the screen keeps moving, recovery after."""
    server = t.server(listen="0.0.0.0:0", state=t.root / "state")
    phone_host = t.host("phone")
    me = phone_host.peer_ip
    observer = t.driver("observer")
    observer.connect(server)
    phone = phone_host.driver("phone")
    phone.connect(f"ws://{me}:{server.port}", token=server.token())
    pane = phone.shell_pane(100, 30)
    counter = Counter(t, phone, pane, period=0.5)
    phone.wait_for(pane, r"(?m)^tick 5$")
    typing = phone.shell_pane(100, 10)
    phone.line(typing, "cat >/dev/null")
    phone.cmd("keepalive", interval_ms=2000, timeout_ms=15_000, auto_reconnect=True, reconnect_timeout_ms=120_000,
              attempt_timeout_ms=10_000)

    phone_host.net.netem(NETEM, dst=me, port=server.port)
    t.net().netem(NETEM, dst=phone_host.ip, port=server.port)
    since = phone.mark()
    start = time.monotonic()
    phone.latency(reset=True)
    last_tick, last_move, longest_stall = counter.last(phone), start, 0.0
    while time.monotonic() - start < WINDOW_S:
        if phone.connected():
            phone.cmd("input", pane=typing, text="x", ok=False, quiet=True)
        time.sleep(1)
        now_tick = counter.last(phone)
        now = time.monotonic()
        if now_tick > last_tick:
            last_tick, last_move = now_tick, now
        longest_stall = max(longest_stall, now - last_move)
    events = phone.events[since:]
    disconnects = sum(e.get("ev") == "disconnected" for e in events)
    reconnects = sum(e.get("ev") == "reconnected" for e in events)
    lat = phone.latency()
    t.metric("disconnects", disconnects)
    t.metric("server_drops", server.logged("client went silent, dropping"))
    t.metric("reconnects", reconnects)
    t.metric("longest_screen_stall_ms", round(longest_stall * 1000))
    t.metric("ticks_seen_in_window", last_tick)
    for k in ("count", "p50_ms", "p90_ms", "max_ms", "unanswered"):
        t.metric(f"input_latency_{k}", lat[k])

    # Hard assertions are about the server; what the client lives through over a TCP
    # this lossy (retransmission backoff, frames queued behind lost packets) is recorded.
    drops = server.logged("client went silent, dropping")
    t.eq(drops, 0, f"the server never drops a client that is only slow (in {WINDOW_S}s)")
    t.ok(disconnects <= 3, f"the session is not torn down on every lag ({disconnects} disconnects in {WINDOW_S}s)")

    healed = time.monotonic()
    phone_host.net.clear_netem()
    t.net().clear_netem()
    # A connection deep in retransmission backoff is given up by the keepalive (15 s).
    counter.check_screen(phone, timeout=60)
    t.metric("recover_ms", round((time.monotonic() - healed) * 1000))
    t.eventually(lambda: observer.viewers(pane) == 1, "exactly one viewer after", timeout=40)
    counter.check_log()
    phone.key(typing, "ctrl-c")
    t.eq(phone.run(typing, "echo typed-after", timeout=60), 0, "input works after the network heals")


@case(fixture="server_state")
def a_session_through_a_lossy_slow_proxy_keeps_moving_without_a_disconnect(t: T):
    """UC-043: layer 1: 300-700 ms per chunk and 30% of chunks held for a retransmission; no drop, the screen moves."""
    server = t.server(state=t.root / "state")
    proxy = t.proxy(server.port)
    observer = t.driver("observer")
    observer.connect(server)
    client = t.driver("client")
    client.connect(proxy.url, token=server.token())
    pane = client.shell_pane(100, 30)
    counter = Counter(t, client, pane, period=0.5)
    client.wait_for(pane, r"(?m)^tick 5$")
    typing = client.shell_pane(100, 10)
    client.line(typing, "cat >/dev/null")
    client.cmd("keepalive", interval_ms=2000, timeout_ms=15_000, auto_reconnect=True, attempt_timeout_ms=10_000)
    proxy.set(delay_ms=300, jitter_ms=400, loss_pct=30, loss_stall_ms=200)
    since = client.mark()
    client.latency(reset=True)
    start = time.monotonic()
    last_tick, last_move, longest_stall = counter.last(client), start, 0.0
    while time.monotonic() - start < 30:
        client.cmd("input", pane=typing, text="x", ok=False, quiet=True)
        time.sleep(1)
        now_tick, now = counter.last(client), time.monotonic()
        if now_tick > last_tick:
            last_tick, last_move = now_tick, now
        longest_stall = max(longest_stall, now - last_move)
    lat = client.latency()
    disconnects = sum(e.get("ev") == "disconnected" for e in client.events[since:])
    t.metric("disconnects", disconnects)
    t.metric("longest_screen_stall_ms", round(longest_stall * 1000))
    for k in ("count", "p50_ms", "p90_ms", "max_ms"):
        t.metric(f"input_latency_{k}", lat[k])
    t.eq(server.logged("client went silent, dropping"), 0, "the server never drops the slow client")
    t.eq(disconnects, 0, "the session is never torn down")
    t.ok(longest_stall <= 10, f"the screen keeps moving ({longest_stall:.1f}s at most without a new tick)")
    t.ok(lat["p90_ms"] is not None and lat["p90_ms"] < 5000, f"input gets through (p90 {lat['p90_ms']} ms)")
    proxy.set(delay_ms=0, jitter_ms=0, loss_pct=0)
    counter.check_screen(client)
    t.eq(observer.viewers(pane), 1, "one viewer")
    counter.check_log()
