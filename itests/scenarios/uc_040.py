"""UC-040: clean half-open: everything stops moving both ways in the middle of output.
Both sides notice within a few pings; the client comes back to the same pane, whose
screen shows the counter without lost or doubled lines.

The client (tilr-driver) reaches tilrd through the layer-1 proxy; an observer connected
directly watches the pane's viewer count, which is how the server's side is measured.
The server runs with --client-timeout 6 (a WebSocket ping every 2 s); the client pings
every second and gives up after 4 s of silence, then reconnects through the same proxy,
which only lets it through once the network is back.
"""

import re
import time

from tt import T, case

CLIENT_TIMEOUT_S = 6
KEEPALIVE_MS, DEAD_AFTER_MS = 1000, 4000


def viewers(observer, pane: str) -> int:
    return next(p["viewers"] for p in observer.cmd("list", quiet=True)["panes"] if p["pane"] == pane)


def tick_numbers(text: str) -> list[int]:
    return [int(m) for m in re.findall(r"(?m)^tick (\d+)$", text)]


@case(fixture="server_state", spec=["client-liveness/a-silent-client-is-dropped"])
def a_frozen_connection_is_dropped_by_both_sides_and_the_client_returns_to_the_same_pane(t: T):
    """UC-040: freeze both directions mid-output; server drops the viewer, client reconnects after the network returns."""
    server = t.server("--client-timeout", CLIENT_TIMEOUT_S, state=t.root / "state")
    proxy = t.proxy(server.port)
    observer = t.driver("observer")
    observer.connect(server)
    client = t.driver("client")
    client.connect(f"ws://127.0.0.1:{proxy.port}", token=server.token())
    pane = client.shell_pane(100, 30)
    ticks = t.root / "ticks"
    client.line(pane, f"i=0; while :; do i=$((i+1)); echo \"tick $i\"; echo \"$i $(date +%s.%N)\" >>{ticks}; "
                      "sleep 0.1; done")
    client.wait_for(pane, r"(?m)^tick 20$")
    client.cmd("keepalive", interval_ms=KEEPALIVE_MS, timeout_ms=DEAD_AFTER_MS, auto_reconnect=True,
               reconnect_timeout_ms=60_000)
    t.eq(viewers(observer, pane), 1, "viewers before")

    since = client.mark()
    frozen = time.monotonic()
    proxy.freeze()
    t.eventually(lambda: viewers(observer, pane) == 0, "the server drops the frozen client", timeout=30, interval=0.1)
    server_drop = time.monotonic() - frozen
    dead = client.wait_event("dead", since=since, timeout=30)
    client_detect = dead["_at"] - frozen
    t.metric("server_drop_ms", round(server_drop * 1000))
    t.metric("client_detect_ms", round(client_detect * 1000))
    t.ok(server_drop <= CLIENT_TIMEOUT_S + 3, f"the server noticed within {CLIENT_TIMEOUT_S + 3}s "
                                              f"(took {server_drop:.1f}s)")
    t.ok(client_detect <= (DEAD_AFTER_MS + KEEPALIVE_MS) / 1000 + 1.5,
         f"the client noticed within a few pings (took {client_detect:.1f}s)")
    t.ok(server.logged("client went silent, dropping") >= 1, "the server logged why it dropped the client")
    time.sleep(1)
    t.ok(not any(e.get("ev") == "reconnected" for e in client.events[since:]),
         "no connection gets through while the network is down")

    resumed = time.monotonic()
    proxy.resume()
    back = client.wait_event("reconnected", since=since, timeout=20)
    t.metric("back_after_resume_ms", round((back["_at"] - resumed) * 1000))
    t.metric("reconnect_attempts", back["attempts"])
    t.ok(back["_at"] - resumed <= 5, f"the client is back within 5s of the network (took {back['_at'] - resumed:.1f}s)")

    # The same pane, its counter still going, on a screen without holes or doubles.
    last = max(tick_numbers(client.text(pane)))
    client.wait_for(pane, rf"(?m)^tick {last + 5}$")
    seen = tick_numbers(client.text(pane))
    t.eq(seen, list(range(seen[0], seen[0] + len(seen))), "counter lines on the screen are consecutive")
    t.eventually(lambda: viewers(observer, pane) == 1, "exactly one viewer once the client is back", timeout=10)
    time.sleep(CLIENT_TIMEOUT_S / 2)
    t.eq(viewers(observer, pane), 1, "no ghost viewer from the old connection")

    # The pane never waited for the frozen client.
    written = [line.split() for line in ticks.read_text().splitlines()]
    numbers = [int(n) for n, _ in written]
    t.eq(numbers, list(range(1, len(numbers) + 1)), "the counter skipped nothing")
    stamps = [float(s) for _, s in written]
    gap = max(b - a for a, b in zip(stamps, stamps[1:]))
    t.metric("counter_max_gap_ms", round(gap * 1000))
    t.ok(gap < 1.0, f"the counter never stalled while the client was frozen (longest gap {gap:.2f}s)")

    # Until the prompt is back: what is typed before the counter dies is flushed with it.
    client.interrupt(pane)
    t.eq(client.run(pane, "echo typed-after-return"), 0, "input works after the return")
    t.eq(len(re.findall(r"(?m)^typed-after-return$", client.text(pane))), 1, "the output appears once")
