"""UC-050: a thousand break/return cycles: no viewers pile up, the server's memory and
descriptors stay flat.

Each cycle the layer-1 proxy resets the client's connection (RST both ways); the client
reconnects on its own and watches the pane again. Every 100 cycles the case reads the
server's VmRSS and open descriptors from /proc and the pane's viewers from an observer.
Resets, not silent freezes: a freeze costs a timeout (seconds) per cycle.
"""

import time

from tt import T, Counter, case

CYCLES = 1000


@case(fixture="server_state", timeout=900, spec=["client-liveness/a-connection-that-ends-is-forgotten-at-once"])
def a_thousand_resets_leave_one_viewer_and_flat_server_memory_and_descriptors(t: T):
    """UC-050: 1000 RST cycles with auto reconnect; viewers 1, server fds flat, RSS growth small."""
    server = t.server(state=t.root / "state")
    proxy = t.proxy(server.port)
    observer = t.driver("observer")
    observer.connect(server)
    client = t.driver("client")
    client.connect(proxy.url, token=server.token())
    client.cmd("keepalive", interval_ms=1000, timeout_ms=4000, auto_reconnect=True, reconnect_timeout_ms=30_000)
    pane = client.shell_pane(100, 30)
    counter = Counter(t, client, pane, period=0.05)
    client.wait_for(pane, r"(?m)^tick 20$")
    # Warm up allocators and buffers before the baseline.
    for _ in range(20):
        since = client.mark()
        proxy.cut()
        client.wait_event("reconnected", since=since, timeout=10)
    t.eventually(lambda: observer.viewers(pane) == 1, "one viewer after the warm-up", timeout=5)
    base = t.proc(server.pid)
    points = [(0, base.rss_kb, base.fds)]
    started = time.monotonic()
    worst = 0.0
    for n in range(1, CYCLES + 1):
        since = client.mark()
        cut = time.monotonic()
        proxy.cut()
        back = client.wait_event("reconnected", since=since, timeout=10)
        worst = max(worst, back["_at"] - cut)
        if n % 100 == 0:
            t.eventually(lambda: observer.viewers(pane) == 1, f"one viewer after {n} cycles", timeout=5,
                         interval=0.05)
            p = t.proc(server.pid)
            points.append((n, p.rss_kb, p.fds))
            print(f"  cycle {n}: rss {p.rss_kb} kB, fds {p.fds}")
    t.metric("cycles", CYCLES)
    t.metric("cycles_s", round(time.monotonic() - started, 1))
    t.metric("worst_reconnect_ms", round(worst * 1000))
    t.metric("server_rss_start_kb", points[0][1])
    t.metric("server_rss_end_kb", points[-1][1])
    t.metric("server_fds_start", points[0][2])
    t.metric("server_fds_end", points[-1][2])
    t.metric("server_connections_logged", server.logged("client connected"))

    t.ok(points[-1][2] - points[0][2] <= 3, f"server descriptors stay flat ({[p[2] for p in points]})")
    growth = points[-1][1] - points[0][1]
    t.ok(growth <= 16 * 1024, f"server memory grows by at most 16 MB over {CYCLES} cycles (+{growth} kB)")
    second_half = points[-1][1] - points[len(points) // 2][1]
    t.ok(second_half <= 4 * 1024, f"and no longer grows in the second half (+{second_half} kB)")
    t.eq(observer.viewers(pane), 1, "one viewer at the end")
    t.eq(len(server.panes()), 1, "one pane process")
    counter.check_screen(client)
    counter.check_log()
    client.interrupt(pane)
    t.eq(client.run(pane, "echo typed-after"), 0, "input works after the cycles")
