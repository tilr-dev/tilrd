"""UC-048: input sent while the connection breaks may be lost, but never arrives twice;
after the return, input works again.

Every input is a command `echo <mark> >>marks` typed into an idle shell, so the marks
file on the server's machine (this container) says which inputs the pane ran, and how
often. Three breaks through the layer-1 proxy: a reset in the middle of a burst of
input, a silent freeze while typing (the client gives up and reconnects, the server
drops the old connection on its timeout), and a freeze that revives after the client
reconnected (what was typed into the dead connection arrives late, once).
"""

import time
from collections import Counter as Tally

from tt import T, case

CLIENT_TIMEOUT_S = 6


def setup(t: T, *server_args):
    server = t.server(*server_args, state=t.root / "state")
    proxy = t.proxy(server.port)
    client = t.driver("client")
    client.connect(proxy.url, token=server.token())
    client.cmd("keepalive", interval_ms=1000, timeout_ms=4000, auto_reconnect=True, reconnect_timeout_ms=60_000)
    pane = client.shell_pane(100, 30)
    marks = t.scratch("marks") / "marks"
    return server, proxy, client, pane, marks


def type_marks(client, pane: str, marks, names) -> list[str]:
    """Type a mark per name; the ones the client managed to send."""
    sent = []
    for name in names:
        if client.cmd("input", pane=pane, text=f"echo {name} >>{marks}\r", ok=False, quiet=True)["ev"] == "ok":
            sent.append(name)
    return sent


def ran(marks) -> list[str]:
    return marks.read_text().split() if marks.exists() else []


def after_return(t: T, client, pane: str, marks, typed: list[str]):
    t.eventually(lambda: client.connected(), "the client is back", timeout=30)
    client.wait_for(pane, r"(?m)^\$ ?$", timeout=10)
    t.eq(client.run(pane, f"echo after >>{marks}"), 0, "input works after the return")
    counts = Tally(ran(marks))
    twice = {m: n for m, n in counts.items() if n > 1}
    t.eq(twice, {}, "no input ran twice")
    lost = [m for m in typed if m not in counts]
    t.metric("inputs_typed", len(typed))
    t.metric("inputs_lost", len(lost))
    t.eq(counts["after"], 1, "the input after the return ran once")
    return counts


@case(fixture="server_state", spec=["client-liveness/a-new-connection-starts-afresh"])
def input_cut_by_a_reset_mid_burst_is_lost_or_kept_in_order_but_never_doubled(t: T):
    """UC-048: RST after 20 of 60 rapid inputs; what ran is the first part, in order, once; input works after."""
    server, proxy, client, pane, marks = setup(t)
    names = [f"burst{n:02d}" for n in range(60)]
    proxy.set(dir="up", cut_after_ws_messages=20)
    typed = type_marks(client, pane, marks, names)
    t.eventually(lambda: proxy.stats()[0]["closed"], "the proxy cut the connection", timeout=5)
    after_return(t, client, pane, marks, typed)
    got = [m for m in ran(marks) if m.startswith("burst")]
    t.eq(got, names[:len(got)], "the inputs that ran are the first ones, in order")


@case(fixture="server_state", spec=["client-liveness/a-new-connection-starts-afresh"])
def input_typed_into_a_silent_connection_is_lost_but_never_doubled(t: T):
    """UC-048: freeze, type for 3 s, the client reconnects after the network returns; held input never runs twice."""
    server, proxy, client, pane, marks = setup(t, "--client-timeout", CLIENT_TIMEOUT_S)
    since = client.mark()
    proxy.freeze()
    typed = []
    for n in range(10):
        typed += type_marks(client, pane, marks, [f"frozen{n}"])
        time.sleep(0.3)
    client.wait_event("dead", since=since, timeout=10)
    time.sleep(CLIENT_TIMEOUT_S)
    proxy.resume()
    client.wait_event("reconnected", since=since, timeout=20)
    after_return(t, client, pane, marks, typed)


@case(fixture="server_state", spec=["client-liveness/a-new-connection-starts-afresh"])
def input_held_in_a_connection_that_revives_arrives_once_at_most(t: T):
    """UC-048: freeze the connection, type, the client reconnects; the old connection revives with the held input."""
    server, proxy, client, pane, marks = setup(t)
    since = client.mark()
    proxy.freeze(conn=1)
    typed = type_marks(client, pane, marks, [f"held{n}" for n in range(5)])
    client.wait_event("reconnected", since=since, timeout=20)
    client.wait_for(pane, r"(?m)^\$ ?$", timeout=10)
    typed += type_marks(client, pane, marks, ["fresh"])
    proxy.resume(conn=1)
    time.sleep(2)
    counts = after_return(t, client, pane, marks, typed)
    t.metric("held_inputs_run_late", sum(counts[f"held{n}"] for n in range(5)))
    t.eq(counts["fresh"], 1, "input typed on the new connection ran once")
