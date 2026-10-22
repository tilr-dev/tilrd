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
input latency is seconds (p50 1-13 s over runs). The server keeps what it is behind on
out of the socket (4 KiB unsent at most) and skips frames for the lagging client
(frames_skipped), sending the latest screen whole, so stale frames do not pile up behind
lost packets; what remains is TCP's own backoff, both ways (the input goes up the same
lossy link). Those are recorded as metrics; the assertions are what the server owes: no
drop of a client that is only slow, at most a few disconnects, one viewer and a whole
screen once the network heals.
"""

import time
import urllib.request

from tt import T, Counter, case

NETEM = "delay 500ms 200ms distribution normal loss 30%"
WINDOW_S = 60


def frames_skipped(server) -> float:
    """Frames the server skipped for a client that lagged, all panes (tilrd_pane_frames_skipped_total)."""
    url = f"http://127.0.0.1:{server.port}/metrics?token={server.token()}"
    text = urllib.request.urlopen(url, timeout=5).read().decode()
    return sum(float(line.rsplit(" ", 1)[1]) for line in text.splitlines()
               if line.startswith("tilrd_pane_frames_skipped_total{"))


def server_drops(server) -> int:
    """Clients the server dropped for not being heard from, or not answering its pings."""
    return server.logged("client went silent, dropping") + server.logged("client stopped answering pings")


@case(fixture="server_state", hosts=["phone"], net_admin=True, timeout=400, spec=[
    "client-liveness/a-lagging-client-skips-to-the-latest-screen",
])
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
    drops = server_drops(server)
    t.metric("server_drops", drops)
    t.metric("frames_skipped", frames_skipped(server))
    t.metric("reconnects", reconnects)
    t.metric("longest_screen_stall_ms", round(longest_stall * 1000))
    t.metric("ticks_seen_in_window", last_tick)
    for k in ("count", "p50_ms", "p90_ms", "max_ms", "unanswered"):
        t.metric(f"input_latency_{k}", lat[k])

    # Hard assertions are about the server; what the client lives through over a TCP
    # this lossy (retransmission backoff, frames queued behind lost packets) is recorded.
    # The server drops a client whose pongs stop for its timeout (30 s); the client gives
    # up on a connection silent for 15 s. Over a stall that long the client lets go first:
    # the server never drops a connection the client still holds.
    t.ok(drops <= disconnects, f"the server drops only connections the client gave up on ({drops} drops, "
                               f"{disconnects} disconnects in {WINDOW_S}s)")
    t.ok(disconnects <= 3, f"the session is not torn down on every lag ({disconnects} disconnects in {WINDOW_S}s)")

    healed = time.monotonic()
    phone_host.net.clear_netem()
    t.net().clear_netem()
    # A connection deep in retransmission backoff is given up by the keepalive (15 s).
    counter.check_screen(phone, timeout=60)
    t.metric("recover_ms", round((time.monotonic() - healed) * 1000))
    t.eventually(lambda: observer.viewers(pane) == 1, "exactly one viewer after", timeout=40)
    counter.check_log()
    phone.interrupt(typing, timeout=60)
    t.eq(phone.run(typing, "echo typed-after", timeout=60), 0, "input works after the network heals")


@case(fixture="server_state", spec=["itest-echo-delay/controlled-input-echo-threshold-check"])
def fixed_network_delay_moves_echo_p90_across_five_seconds(t: T):
    """UC-043: fixed 3 s each way crosses 5 s echo p90; zero delay stays below it."""
    server = t.server(state=t.root / "state")
    proxy = t.proxy(server.port)
    client = t.driver("client")
    client.connect(proxy.url, token=server.token())
    pane = client.shell_pane(100, 10)
    client.line(pane, "cat >/dev/null")
    # Establish that cat is reading and the shell's setup output has drained.
    client.cmd("input", pane=pane, text="ready")
    client.wait_for(pane, r"(?m)^ready$")
    since = client.mark()
    echoed = "ready"
    for phase, delay_ms in (("baseline", 0), ("delayed", 3000), ("recovered", 0)):
        proxy.set(dir="both", delay_ms=delay_ms, jitter_ms=0, loss_pct=0)
        client.latency(reset=True)
        for char in "abcdefghij":
            echoed += char
            client.cmd("input", pane=pane, text=char)
            # Each distinct echo proves that this input's frame arrived. The driver
            # records its latency before updating the screen matched by wait_for.
            client.wait_for(pane, rf"(?m)^{echoed}$")
        lat = client.latency()
        for key in ("count", "pending", "unanswered", "p90_ms"):
            t.metric(f"{phase}_input_latency_{key}", lat[key])
        t.eq(lat["count"], 10, f"{phase}: every input has a measured echo")
        t.eq(lat["pending"], 0, f"{phase}: no input is still awaiting a frame")
        t.eq(lat["unanswered"], 0, f"{phase}: no input lost its connection")
        p90 = lat["p90_ms"]
        t.ok(p90 is not None and (p90 > 5000 if delay_ms else p90 < 5000),
             f"{phase}: echo p90 {p90} ms is {'above' if delay_ms else 'below'} 5000 ms")
    t.eq(sum(e.get("ev") == "disconnected" for e in client.events[since:]), 0,
         "the controlled delay never disconnects the client")
    t.eq(server_drops(server), 0, "the server never drops the delayed client")


@case(fixture="server_state", spec=["client-liveness/a-lagging-client-skips-to-the-latest-screen"])
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
    t.metric("frames_skipped", frames_skipped(server))
    t.metric("longest_screen_stall_ms", round(longest_stall * 1000))
    for k in ("count", "p50_ms", "p90_ms", "max_ms"):
        t.metric(f"input_latency_{k}", lat[k])
    t.eq(server_drops(server), 0, "the server never drops the slow client")
    t.eq(disconnects, 0, "the session is never torn down")
    t.ok(longest_stall <= 10, f"the screen keeps moving ({longest_stall:.1f}s at most without a new tick)")
    t.ok(lat["p90_ms"] is not None and lat["p90_ms"] < 5000, f"input gets through (p90 {lat['p90_ms']} ms)")
    proxy.set(delay_ms=0, jitter_ms=0, loss_pct=0)
    counter.check_screen(client)
    t.eq(observer.viewers(pane), 1, "one viewer")
    counter.check_log()
