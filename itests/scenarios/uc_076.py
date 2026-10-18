"""UC-076: commands from a pane to its clients: `tilr edit <file>` and `tilr open <thing>`
in a pane reach every client that watches the pane and listens for them (HostEvent,
"host-events" in Hello.features), and only those; with nobody to hear them the command
says so and fails.

Four clients: "desk" and "tablet" (another host, another IP) watch the pane and listen;
"viewer" watches it but does not listen; "elsewhere" listens but watches another pane.
`tilr` is the server's executable under that name on the panes' PATH ($TILR_SHIMS).
"""

import time

from tt import TMP, T, case


def events(d, since: int) -> list[dict]:
    return [e for e in d.events[since:] if e.get("ev") == "host_event"]


@case(fixture="server_state", hosts=["tablet"], spec=[
    "clipboard-host-events/host-events-reach-every-listening-client-watching-the-pane",
    "clipboard-host-events/tilr-edit-and-tilr-open-say-what-to-open",
])
def tilr_edit_and_open_reach_every_client_watching_the_pane_and_no_other(t: T):
    """UC-076: tilr edit / open in a pane reach both listening viewers (one on another host), not the others."""
    server = t.server(listen="0.0.0.0:0", state=t.root / "state")
    desk = t.driver("desk")
    desk.connect(server, features=["host-events"])
    pane = desk.shell_pane()
    other_pane = desk.create_pane(subscribe=False)
    tablet_host = t.host("tablet")
    tablet = tablet_host.driver("tablet")
    tablet.connect(f"ws://{tablet_host.peer_ip}:{server.port}", token=server.token(), features=["host-events"])
    tablet.cmd("subscribe", pane=pane)
    viewer = t.driver("viewer")
    viewer.connect(server)
    viewer.cmd("subscribe", pane=pane)
    elsewhere = t.driver("elsewhere")
    elsewhere.connect(server, features=["host-events"])
    elsewhere.cmd("subscribe", pane=other_pane)
    t.eventually(lambda: desk.viewers(pane) == 3, "three clients watch the pane", timeout=10)
    cwd = desk.value(pane, "$PWD")

    asks = [
        ("tilr edit notes/todo.txt", {"name": "open", "kind": "file", "path": f"{cwd}/notes/todo.txt", "args": []}),
        ("tilr open https://example.com/a?b=1", {"name": "open", "kind": "url", "path": "https://example.com/a?b=1",
                                                 "args": []}),
        ("tilr open --kind diff left.txt --side-by-side -w",
         {"name": "open", "kind": "diff", "path": "left.txt", "args": ["--side-by-side", "-w"]}),
    ]
    for command, want in asks:
        marks = {d.name: d.mark() for d in (desk, tablet, viewer, elsewhere)}
        status = desk.run(pane, command)
        t.eq(status, 0, f"`{command}` succeeds")
        for d in (desk, tablet):
            got = d.wait_event("host_event", since=marks[d.name], timeout=10)
            t.eq({k: got[k] for k in ("name", "kind", "path", "args")}, want, f"{d.name} gets `{command}`")
            t.eq(got["pane"], pane, f"{d.name} is told which pane asked")
        time.sleep(0.5)
        for d in (viewer, elsewhere):
            t.eq(events(d, marks[d.name]), [], f"{d.name} gets nothing of `{command}`")
    t.ok(server.logged("host event delivered pane"), "the server logged the deliveries")


@case(fixture="server_state", spec=["clipboard-host-events/tilr-edit-fails-with-a-reason-when-nobody-listens"])
def tilr_edit_with_nobody_listening_fails_and_says_why(t: T):
    """UC-076: with no client listening on the pane, `tilr edit` exits non-zero: no tilr client attached."""
    server = t.server(state=t.root / "state")
    d = t.driver()
    d.connect(server)
    pane = d.shell_pane()
    t.ok(d.run(pane, f"tilr edit x.txt 2>{TMP}/err.txt") != 0, "tilr edit fails")
    t.has((TMP / "err.txt").read_text(), "no tilr client attached")
    listener = t.driver("listener")
    listener.connect(server, features=["host-events"])
    listener.cmd("subscribe", pane=pane)
    since = listener.mark()
    t.eq(d.run(pane, "tilr edit x.txt"), 0, "with a listener it succeeds")
    listener.wait_event("host_event", since=since, timeout=10)
    listener.cmd("unsubscribe", pane=pane)
    t.ok(d.run(pane, "tilr edit x.txt") != 0, "and fails again once the listener stops watching")
