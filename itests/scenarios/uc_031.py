"""UC-031: `--listen 0.0.0.0` / `[::]`: the port is reachable from another container, and
the access token is still required there: none or a wrong one is refused, the right one
gets a shell. `[::]` takes IPv4 clients too (dual stack).
"""

from tt import T, case


def from_peer(t: T, server):
    peer = t.host("peer")
    url = f"ws://{peer.peer_ip}:{server.port}"
    for name, token in (("none", None), ("wrong", "not-the-token")):
        d = peer.driver(f"token-{name}")
        args = {"url": url, **({"token": token} if token else {})}
        r = d.cmd("connect", ok=False, **args)
        t.ok(r.get("ev") != "connected", f"without the right token ({name}) the server refuses: {r}")
    d = peer.driver("with-token")
    d.connect(url, token=server.token())
    pane = d.shell_pane()
    t.eq(d.value(pane, "$((6*7))"), "42", "a shell from the other container")
    t.ok(server.logged("client connected"), "the server saw it")


@case(hosts=["peer"], spec=["access-control/tcp-clients-present-the-token-in-hello"])
def listening_on_every_ipv4_address_asks_for_the_token(t: T):
    """UC-031: --listen 0.0.0.0:7890: reachable from another container, token required."""
    from_peer(t, t.server(listen="0.0.0.0:7890"))


@case(hosts=["peer"], spec=["access-control/tcp-clients-present-the-token-in-hello"])
def listening_on_every_address_takes_ipv4_too_and_asks_for_the_token(t: T):
    """UC-031: --listen [::]:7890 (dual stack): another container over IPv4, token required."""
    from_peer(t, t.server(listen="[::]:7890"))
