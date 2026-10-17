"""UC-045: RST in the middle of a frame: the client reconnects at once, the server cleans up
the viewer at once.

Layer 1: the proxy writes half of a WebSocket message from the server, then resets both
sides. Layer 2: the client's own kernel destroys its socket (ss -K in the client's
container), which sends the server a reset. Either way nothing waits for a timeout:
tilrd runs with the default client timeout (30 s), so a clean-up within seconds is the
reset's doing.
"""

import time

from tt import T, Counter, case


def check_after_reset(t: T, observer, client, pane: str, counter: Counter, cut_at: float, since: int):
    lost = client.wait_event("disconnected", since=since, timeout=5)
    t.metric("client_detect_ms", round((lost["_at"] - cut_at) * 1000))
    back = client.wait_event("reconnected", since=since, timeout=10)
    t.metric("client_back_ms", round((back["_at"] - cut_at) * 1000))
    t.ok(back["_at"] - cut_at <= 2, f"the client is back at once (took {back['_at'] - cut_at:.2f}s)")
    t.ok(not any(e.get("ev") == "protocol_error" for e in client.events[since:]),
         "half a message is never taken for a message")
    t.eventually(lambda: observer.viewers(pane) == 1, "one viewer: the reset connection's is gone", timeout=3,
                 interval=0.05)
    t.metric("server_clean_ms", round((time.monotonic() - cut_at) * 1000))
    time.sleep(2)
    t.eq(observer.viewers(pane), 1, "still one viewer")
    counter.check_screen(client)
    counter.check_log()
    client.key(pane, "ctrl-c")
    t.eq(client.run(pane, "echo typed-after"), 0, "input works after the reset")


@case(fixture="server_state")
def a_reset_in_the_middle_of_a_frame_makes_the_client_reconnect_at_once(t: T):
    """UC-045: the proxy cuts mid-message with RST; reconnect within 2 s, the viewer cleaned up, the screen whole."""
    server = t.server(state=t.root / "state")
    proxy = t.proxy(server.port)
    observer = t.driver("observer")
    observer.connect(server)
    client = t.driver("client")
    client.connect(proxy.url, token=server.token())
    client.cmd("keepalive", interval_ms=1000, timeout_ms=4000, auto_reconnect=True)
    pane = client.shell_pane(100, 30)
    counter = Counter(t, client, pane, period=0.02)
    client.wait_for(pane, r"(?m)^tick 50$")
    since = client.mark()
    cut_at = time.monotonic()
    proxy.set(dir="down", cut_mid_ws_message=True)
    t.eventually(lambda: proxy.stats()[0]["closed"], "the proxy cut the connection mid-message", timeout=5)
    check_after_reset(t, observer, client, pane, counter, cut_at, since)


@case(fixture="server_state", hosts=["phone"])
def a_socket_reset_by_the_clients_kernel_is_cleaned_up_by_the_server_at_once(t: T):
    """UC-045: ss -K on the client's container resets its socket mid-output; reconnect at once, the viewer cleaned up."""
    server = t.server(listen="0.0.0.0:0", state=t.root / "state")
    phone_host = t.host("phone")
    me = phone_host.peer_ip
    observer = t.driver("observer")
    observer.connect(server)
    client = phone_host.driver("phone")
    client.connect(f"ws://{me}:{server.port}", token=server.token())
    client.cmd("keepalive", interval_ms=1000, timeout_ms=4000, auto_reconnect=True)
    pane = client.shell_pane(100, 30)
    counter = Counter(t, client, pane, period=0.02)
    client.wait_for(pane, r"(?m)^tick 50$")
    since = client.mark()
    cut_at = time.monotonic()
    killed = phone_host.net.kill_sockets(me, server.port)
    t.eq(killed.code, 0, "ss -K destroyed the client's socket")
    check_after_reset(t, observer, client, pane, counter, cut_at, since)
