"""UC-052: client identity: Hello.client_id names one client instance (a window, a tab, a
process) across its connections. A Hello with the client_id of a connection still open,
under the same token, supersedes that connection at once (WebSocket close 4000
"superseded"); other client ids, and the same id under another token, supersede nobody.

Each tilr-driver makes up a client_id of its own when it starts and keeps it across its
reconnects; `connect` with `client_id` names another, which is how a case plays a second
tab of the same client or a spoof.
"""

import time
import urllib.request

from tt import T, Counter, case


def drops(t: T, server, reason: str) -> float:
    """tilr_client_drops_total{reason} from the server's /metrics."""
    url = f"http://127.0.0.1:{server.port}/metrics?token={server.token()}"
    text = urllib.request.urlopen(url, timeout=5).read().decode()
    series = f'tilr_client_drops_total{{reason="{reason}"}} '
    return next((float(line[len(series):]) for line in text.splitlines() if line.startswith(series)), -1.0)


def viewer_ids(observer, pane: str) -> list[str]:
    panes = observer.cmd("list", quiet=True)["panes"]
    return sorted(v["client_id"] for p in panes if p["pane"] == pane for v in p["viewed_by"])


@case(fixture="server_state", hosts=["laptop"])
def two_tabs_of_one_host_with_client_ids_of_their_own_both_stay_live(t: T):
    """UC-052: two drivers on one host, each its own client_id, watch one pane: both stay, both type."""
    server = t.server(listen="0.0.0.0:0", state=t.root / "state")
    laptop = t.host("laptop")
    url = f"ws://{laptop.peer_ip}:{server.port}"
    observer = t.driver("observer")
    observer.connect(server)
    tab1, tab2 = laptop.driver("tab1"), laptop.driver("tab2")
    id1 = tab1.connect(url, token=server.token())["client_id"]
    id2 = tab2.connect(url, token=server.token())["client_id"]
    t.ok(id1 and id2 and id1 != id2, f"each driver has a client_id of its own ({id1}, {id2})")
    pane = tab1.shell_pane(100, 30)
    counter = Counter(t, tab1, pane)
    tab2.cmd("subscribe", pane=pane)
    counter.check_screen(tab2, what="the second tab's screen")
    t.eq(viewer_ids(observer, pane), sorted([id1, id2]), "both tabs watch the pane, by client_id")
    peers = {v["addr"].rsplit(":", 1)[0] for p in observer.cmd("list", quiet=True)["panes"] if p["pane"] == pane
             for v in p["viewed_by"]}
    t.eq(peers, {laptop.ip}, "both from the laptop's address")
    time.sleep(3)
    t.eq(observer.viewers(pane), 2, "still two viewers")
    t.eq(server.logged("superseded"), 0, "nobody superseded")
    t.ok(tab1.connected() and tab2.connected(), "both connections open")
    tab2.key(pane, "ctrl-c")
    t.eq(tab2.run(pane, "echo from-tab2"), 0, "the second tab types")
    t.eq(tab1.run(pane, "echo from-tab1"), 0, "the first tab types")
    t.eq(drops(t, server, "superseded"), 0.0, "no supersede counted")


@case(fixture="server_state")
def the_same_client_id_under_another_token_supersedes_nobody(t: T):
    """UC-052: a spoof with the victim's client_id: refused with a wrong token; on --no-auth another token is another client."""
    server = t.server(state=t.root / "state")
    observer = t.driver("observer")
    observer.connect(server)
    victim = t.driver("victim")
    victim_id = victim.connect(server)["client_id"]
    pane = victim.shell_pane(100, 30)
    spoof = t.driver("spoof")
    refused = spoof.cmd("connect", url=server.url, token="0" * 64, client_id=victim_id, ok=False)
    t.eq(refused.get("ev"), "error", "the spoof with a wrong token is refused")
    t.has(refused.get("message", ""), "wrong access token")
    time.sleep(1)
    t.ok(victim.connected(), "the victim's connection stays")
    t.eq(observer.viewers(pane), 1, "the victim still watches")
    t.eq(victim.run(pane, "echo still-here"), 0, "the victim still types")
    t.eq(server.logged("superseded"), 0, "nobody superseded")

    # Without a token any Hello gets in: another token is another user's client.
    open_server = t.server("--no-auth", name="tilrd-open", state=t.root / "state-open")
    one, two = t.driver("one"), t.driver("two")
    same = "6c1d0c8e-0000-4000-8000-0000000000aa"
    one.connect(open_server.url, token="one", client_id=same)
    pane = one.shell_pane(100, 30)
    two.connect(open_server.url, token="two", client_id=same)
    two.cmd("subscribe", pane=pane)
    t.eq(two.run(pane, "echo from-two"), 0, "the second client types")
    t.ok(one.connected(), "the first one stays")
    t.eq(one.viewers(pane), 2, "both watch")
    t.eq(open_server.logged("superseded"), 0, "nobody superseded")


@case(fixture="server_state")
def a_superseded_connection_is_closed_with_reason_superseded_and_not_reconnected(t: T):
    """UC-052: a second connection with the same client_id and token: the first closed at once (4000 superseded)."""
    server = t.server(state=t.root / "state")
    observer = t.driver("observer")
    observer.connect(server)
    old = t.driver("old")
    the_id = old.connect(server)["client_id"]
    old.cmd("keepalive", interval_ms=1000, timeout_ms=4000, auto_reconnect=True)
    pane = old.shell_pane(100, 30)
    counter = Counter(t, old, pane)
    old.wait_for(pane, r"(?m)^tick 10$")
    since = old.mark()
    new = t.driver("new")
    started = time.monotonic()
    new.connect(server, client_id=the_id)
    gone = old.wait_event("disconnected", since=since, timeout=5)
    t.metric("old_closed_ms", round((gone["_at"] - started) * 1000))
    t.has(gone.get("reason", ""), "4000 superseded")
    t.ok(gone.get("superseded") is True, "the old connection knows it was superseded")
    t.ok(gone["_at"] - started < 1, f"closed at once ({gone['_at'] - started:.2f}s)")
    t.eq(observer.viewers(pane), 0, "the old viewer is gone before the new one subscribes")
    new.cmd("subscribe", pane=pane)
    counter.check_screen(new, what="the new connection's screen")
    t.eq(observer.viewers(pane), 1, "one viewer")
    time.sleep(3)
    t.ok(not any(e.get("ev") == "reconnected" for e in old.events[since:]),
         "the superseded connection is not reconnected")
    t.eq(observer.viewers(pane), 1, "still one viewer")
    t.ok(server.logged("client superseded by a newer connection") == 1, "the server logged the supersede")
    t.eq(drops(t, server, "superseded"), 1.0, "tilr_client_drops_total counts it")
    counter.check_log()
    new.key(pane, "ctrl-c")
    t.eq(new.run(pane, "echo typed-after"), 0, "input works on the new connection")
