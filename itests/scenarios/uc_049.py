"""UC-049: a slow client (10 KB/s towards it) under heavy output: the server does not
hoard frames for it without bound, and the other clients do not slow down.

A pane prints a 200000-line file over and over. The slow client watches it through the
layer-1 proxy limited to 10 KB/s server -> client; a fast client (direct) watches the
same pane and types into another one. The case samples the server's memory (VmRSS of the
supervisor) and the fast client's input latency for 30 s.

Observed (FAIL, product): the supervisor decides the slow client is too slow
("client too slow, dropping", once its queue of CLIENT_QUEUE = 1024 packets is full) and
forgets it, but the connection stays open: serve_client's writer task goes on draining
the queued packets at the client's pace, and its read loop keeps the socket. The slow
client keeps receiving stale frames for minutes, its pings get no answer and its input
goes nowhere, with nothing to tell it it was dropped (supervisor.rs drop_client drops
the sender only; the writer ends when the queue is empty).
"""

import threading
import time

from tt import T, case

RATE = 10_000
GIVE_UP_S = 90


def rss_kb(t: T, pid: int) -> int:
    p = t.proc(pid)
    return p.rss_kb if p else 0


@case(fixture="server_state", timeout=400)
def a_slow_client_under_heavy_output_is_let_go_without_slowing_the_others(t: T):
    """UC-049: 10 KB/s client, output storm: server memory bounded, the fast client unaffected, the slow one let go."""
    server = t.server(state=t.root / "state")
    proxy = t.proxy(server.port)
    fast = t.driver("fast")
    fast.connect(server)
    storm = fast.shell_pane(200, 60)
    typing = fast.shell_pane(80, 10)
    fast.line(typing, "cat >/dev/null")
    slow = t.driver("slow")
    slow.connect(proxy.url, token=server.token())
    slow.cmd("keepalive", interval_ms=1000, timeout_ms=600_000, auto_reconnect=True)
    slow.cmd("subscribe", pane=storm)
    slow.wait_for(storm, r"(?m)^\$ ?$")

    proxy.set(dir="down", rate_bps=RATE)
    base_rss = rss_kb(t, server.pid)
    samples = []
    stop = threading.Event()

    def sample():
        while not stop.is_set():
            samples.append(rss_kb(t, server.pid))
            time.sleep(0.5)
    threading.Thread(target=sample, daemon=True).start()
    slow_since = slow.mark()
    fast.latency(reset=True)
    fast.cmd("frames", reset=True, since=10**12, quiet=True)
    start = time.monotonic()
    # Random text: every frame changes every row and compresses badly.
    fast.line(storm, "base64 /dev/urandom")
    while not server.logged("client too slow") and time.monotonic() - start < GIVE_UP_S:
        fast.input(typing, "x")
        fast.cmd("sleep", ms=200, quiet=True)
    gave_up = time.monotonic() - start if server.logged("client too slow") else None
    gone = None
    if gave_up is not None:
        try:
            gone = slow.wait_event("disconnected", since=slow_since, timeout=15)["_at"] - start - gave_up
        except Exception:
            pass
    lat = fast.latency()
    frames = fast.cmd("frames", since=10**12, quiet=True)
    fast.key(storm, "ctrl-c")
    stop.set()

    peak = max(samples)
    t.metric("server_rss_base_kb", base_rss)
    t.metric("server_rss_peak_kb", peak)
    t.metric("fast_frames", frames["seen"])
    t.metric("fast_frame_max_gap_ms", frames["max_gap_ms"])
    for k in ("count", "p50_ms", "p90_ms", "p99_ms", "max_ms"):
        t.metric(f"fast_input_latency_{k}", lat[k])
    t.metric("slow_bytes_received", sum(s["bytes_down"] for s in proxy.stats()))
    t.metric("slow_dropped_after_ms", round(gave_up * 1000) if gave_up is not None else -1)
    t.metric("slow_disconnected_after_drop_ms", round(gone * 1000) if gone is not None else -1)
    t.ok(peak - base_rss < 128 * 1024, f"the server's memory stays bounded (+{(peak - base_rss) // 1024} MB)")
    t.ok(lat["p99_ms"] is not None and lat["p99_ms"] < 1000,
         f"the fast client's input latency stays low (p99 {lat['p99_ms']} ms)")
    t.ok(frames["max_gap_ms"] < 1000, f"the fast client's frames keep coming ({frames['max_gap_ms']} ms at most apart)")
    t.ok(gave_up is not None, f"the server gives up on the slow client within {GIVE_UP_S}s")
    t.check("the slow client's connection ends once the server has dropped it (within 15 s)",
            lambda: t.ok(gone is not None, "disconnected"))

    # The network gets fast again: the slow client comes back to a whole screen.
    proxy.set(dir="down", rate_bps=0)
    slow.wait_event("reconnected", since=slow_since, timeout=60)
    slow.wait_for(storm, r"(?m)^\$ ?$", timeout=20)
    t.eq(fast.run(storm, "echo after-storm"), 0, "the pane takes input after the storm")
