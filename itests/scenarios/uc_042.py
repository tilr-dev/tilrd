"""UC-042: zombie session: the old connection hangs half-open, the client comes back from
a new IP; the server collapses the old viewer, no ghost stays.

The client is a phone that changes network: on "wifi" (a container of its own) it watches
a pane with a counter; wifi goes silent (iptables DROP both ways on that container, or
the layer-1 proxy in a container of its own freezing its connection), and the client
comes back on "lte", another container with another IP, to the same pane. tilrd runs in
the case's container with --client-timeout 6; an observer connected locally counts the
viewers.

The client on lte is the same client instance (the same Hello.client_id, the same
token): its Hello supersedes the half-open connection at once, so the ghost viewer is
gone before the client has even subscribed again, well within a second, not when the
old connection's silence reaches the client timeout. Recorded as ghost_viewer_ms (from
the new connection's hello).
"""

import re
import time

from tt import T, Counter, case

CLIENT_TIMEOUT_S = 6
KEEPALIVE_MS, DEAD_AFTER_MS = 1000, 4000


def addresses(server) -> list[str]:
    return re.findall(r"client connected client=\d+ addr=([\d.]+):\d+", server.log_text())


def move_to_lte(t: T, server, observer, wifi, lte, lte_url: str, pane: str, counter: Counter, cut, restore,
                client_id: str):
    t.eq(observer.viewers(pane), 1, "viewers before")
    since = wifi.mark()
    cut_at = time.monotonic()
    cut()
    # The same app on its other network: the same client id.
    hello = time.monotonic()
    lte.connect(lte_url, token=server.token(), client_id=client_id)
    t.eventually(lambda: observer.viewers(pane) == 0, "the new connection's hello supersedes the old viewer",
                 timeout=1, interval=0.02)
    ghost = time.monotonic() - hello
    t.metric("ghost_viewer_ms", round(ghost * 1000))
    t.ok(ghost < 1, f"no ghost viewer once the client is back ({ghost:.2f}s)")
    t.ok(server.logged("client superseded by a newer connection") >= 1, "the server logged the supersede")
    lte.cmd("subscribe", pane=pane)
    counter.check_screen(lte, what="the screen on the new network")
    t.metric("new_network_screen_ms", round((time.monotonic() - cut_at) * 1000))
    seen = addresses(server)
    t.ok(len(set(seen)) >= 2, f"the new connection comes from another address ({seen})")
    t.eq(observer.viewers(pane), 1, "one viewer: the client on its new network")
    dead = wifi.wait_event("dead", since=since, timeout=30)
    t.metric("old_client_detect_ms", round((dead["_at"] - cut_at) * 1000))

    lte.interrupt(pane)
    t.eq(lte.run(pane, "echo from-lte"), 0, "input from the new network works")
    t.eq(len(re.findall(r"(?m)^from-lte$", lte.text(pane))), 1, "the output appears once")

    # The old network comes back: its connection is gone for good, no resurrected viewer.
    restore()
    time.sleep(3)
    t.eq(observer.viewers(pane), 1, "still one viewer once the old network is back")
    counter.check_log()


@case(fixture="server_state", hosts=["wifi", "lte"])
def a_client_back_from_a_new_ip_leaves_no_ghost_of_its_half_open_connection(t: T):
    """UC-042: iptables DROP on the old network's container, the client returns from another container; one viewer."""
    server = t.server("--client-timeout", CLIENT_TIMEOUT_S, listen="0.0.0.0:0", state=t.root / "state")
    wifi_host, lte_host = t.host("wifi"), t.host("lte")
    me = wifi_host.peer_ip
    observer = t.driver("observer")
    observer.connect(server)
    wifi = wifi_host.driver("wifi")
    client_id = wifi.connect(f"ws://{me}:{server.port}", token=server.token())["client_id"]
    wifi.cmd("keepalive", interval_ms=KEEPALIVE_MS, timeout_ms=DEAD_AFTER_MS)
    pane = wifi.shell_pane(100, 30)
    counter = Counter(t, wifi, pane)
    wifi.wait_for(pane, r"(?m)^tick 20$")
    lte = lte_host.driver("lte")
    move_to_lte(t, server, observer, wifi, lte, f"ws://{lte_host.peer_ip}:{server.port}", pane, counter,
                cut=lambda: wifi_host.net.drop(me, server.port), restore=wifi_host.net.restore,
                client_id=client_id)


@case(fixture="server_state", hosts=["wifi", "lte"], services=["proxy"])
def a_client_back_from_a_new_ip_leaves_no_ghost_when_its_old_path_froze_in_a_proxy(t: T):
    """UC-042: client, proxy and tilrd in three containers; the proxy freezes the old path, the client returns directly."""
    server = t.server("--client-timeout", CLIENT_TIMEOUT_S, listen="0.0.0.0:0", state=t.root / "state")
    wifi_host, lte_host = t.host("wifi"), t.host("lte")
    proxy = t.proxy_service()
    me = wifi_host.peer_ip
    proxy.target(me, server.port)
    observer = t.driver("observer")
    observer.connect(server)
    wifi = wifi_host.driver("wifi")
    client_id = wifi.connect(proxy.url, token=server.token())["client_id"]
    wifi.cmd("keepalive", interval_ms=KEEPALIVE_MS, timeout_ms=DEAD_AFTER_MS)
    pane = wifi.shell_pane(100, 30)
    counter = Counter(t, wifi, pane)
    wifi.wait_for(pane, r"(?m)^tick 20$")
    t.eq(proxy.stats()[0]["peer"].split(":")[0], wifi_host.ip, "the old path runs from the wifi container")
    lte = lte_host.driver("lte")
    move_to_lte(t, server, observer, wifi, lte, f"ws://{lte_host.peer_ip}:{server.port}", pane, counter,
                cut=lambda: proxy.freeze(conn=1), restore=lambda: proxy.resume(conn=1), client_id=client_id)
