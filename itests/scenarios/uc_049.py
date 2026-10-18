"""UC-049: a slow client (10 KB/s towards it) under heavy output: the server does not
hoard frames for it without bound, and the other clients do not slow down.

A pane prints random text without end. The slow client watches it through the layer-1
proxy limited to 10 KB/s server -> client; a fast client (direct) watches the same pane
and types into another one. The case samples the server's memory (VmRSS of the
supervisor) and the fast client's input latency for 30 s.

The server skips frames for a client that lags (its queue holds a packet for longer than
a moment) and sends it a full frame of the latest screen once it caught up: the slow
client sees fewer screens, each the latest the network lets through, and the server
keeps no backlog of stale ones (tilr_pane_frames_skipped_total). A client too slow even
for that, one whose answers to the server's pings stop for the client timeout, is
dropped, and its connection really ends: the second case.
"""

import threading
import time
import urllib.request

from tt import T, case

RATE = 10_000
WINDOW_S = 30


def rss_kb(t: T, pid: int) -> int:
    p = t.proc(pid)
    return p.rss_kb if p else 0


def metric(server, series: str) -> float:
    """One series of the server's /metrics, -1 if missing."""
    url = f"http://127.0.0.1:{server.port}/metrics?token={server.token()}"
    text = urllib.request.urlopen(url, timeout=5).read().decode()
    return next((float(line[len(series) + 1:]) for line in text.splitlines() if line.startswith(series + " ")),
                -1.0)


def sample_rss(t: T, pid: int) -> tuple[list[int], threading.Event]:
    samples, stop = [], threading.Event()

    def sample():
        while not stop.is_set():
            samples.append(rss_kb(t, pid))
            time.sleep(0.5)
    threading.Thread(target=sample, daemon=True).start()
    return samples, stop


@case(fixture="server_state", timeout=400, spec=[
    "client-liveness/a-lagging-client-skips-to-the-latest-screen",
    "metrics/a-lagging-client-skips-to-the-latest-screen",
    "metrics/per-pane-frame-counters",
])
def a_slow_client_under_heavy_output_gets_the_latest_screens_without_slowing_the_others(t: T):
    """UC-049: 10 KB/s client, output storm: frames skipped for it, server memory bounded, the fast client unaffected."""
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
    samples, stop = sample_rss(t, server.pid)
    slow_since = slow.mark()
    fast.latency(reset=True)
    fast.cmd("frames", reset=True, since=10**12, quiet=True)
    slow_seen = slow.cmd("frames", reset=True, since=10**12, quiet=True)["seen"]
    start = time.monotonic()
    # Random text: every frame changes every row and compresses badly.
    fast.line(storm, "base64 /dev/urandom")
    while time.monotonic() - start < WINDOW_S:
        fast.input(typing, "x")
        fast.cmd("sleep", ms=200, quiet=True)
    lat = fast.latency()
    frames = fast.cmd("frames", since=10**12, quiet=True)
    slow_frames = slow.cmd("frames", since=10**12, quiet=True)
    skipped = metric(server, f'tilr_pane_frames_skipped_total{{pane="{storm}"}}')
    produced = metric(server, f'tilr_pane_frames_total{{pane="{storm}"}}')
    fast.key(storm, "ctrl-c")
    stop.set()

    peak = max(samples)
    dropped = sum(server.logged(m) for m in ("client too slow", "client stopped answering pings",
                                             "client went silent"))
    t.metric("server_rss_base_kb", base_rss)
    t.metric("server_rss_peak_kb", peak)
    t.metric("fast_frames", frames["seen"])
    t.metric("fast_frame_max_gap_ms", frames["max_gap_ms"])
    for k in ("count", "p50_ms", "p90_ms", "p99_ms", "max_ms"):
        t.metric(f"fast_input_latency_{k}", lat[k])
    t.metric("storm_frames", produced)
    t.metric("frames_skipped", skipped)
    t.metric("slow_frames", slow_frames["seen"] - slow_seen)
    t.metric("slow_frame_max_gap_ms", slow_frames["max_gap_ms"])
    t.metric("slow_bytes_received", sum(s["bytes_down"] for s in proxy.stats()))
    t.metric("slow_drops", dropped)
    t.ok(peak - base_rss < 128 * 1024, f"the server's memory stays bounded (+{(peak - base_rss) // 1024} MB)")
    t.ok(lat["p99_ms"] is not None and lat["p99_ms"] < 1000,
         f"the fast client's input latency stays low (p99 {lat['p99_ms']} ms)")
    t.ok(frames["max_gap_ms"] < 1000, f"the fast client's frames keep coming ({frames['max_gap_ms']} ms at most apart)")
    t.ok(skipped > 0, f"frames are skipped for the slow client ({skipped:.0f} of {produced:.0f})")
    t.ok(slow_frames["seen"] > slow_seen, "the slow client still gets screens")
    if dropped:
        # Let go after all: then its connection ends, not minutes of stale frames later.
        t.check("a slow client the server let go is disconnected within 15 s",
                lambda: slow.wait_event("disconnected", since=slow_since, timeout=15))

    # The network gets fast again: the slow client has a whole, current screen.
    proxy.set(dir="down", rate_bps=0)
    slow.wait_for(storm, r"(?m)^\$ ?$", timeout=60)
    t.eq(fast.run(storm, "echo after-storm"), 0, "the pane takes input after the storm")
    slow.wait_for(storm, r"(?m)^after-storm$", timeout=20)


@case(fixture="server_state", timeout=200, spec=[
    "client-liveness/a-client-whose-queue-fills-is-dropped",
    "client-liveness/a-dropped-connection-really-ends",
])
def a_client_too_slow_even_for_the_latest_screen_is_dropped_and_its_connection_ends(t: T):
    """UC-049: 1 KB/s towards a client under an output storm: dropped within the client timeout, connection gone."""
    timeout_s = 6
    server = t.server("--client-timeout", timeout_s, state=t.root / "state")
    proxy = t.proxy(server.port)
    fast = t.driver("fast")
    fast.connect(server)
    storm = fast.shell_pane(200, 60)
    slow = t.driver("slow")
    slow.connect(proxy.url, token=server.token())
    # It never gives up on its own: what ends its connection is the server.
    slow.cmd("keepalive", interval_ms=1000, timeout_ms=600_000)
    slow.cmd("subscribe", pane=storm)
    slow.wait_for(storm, r"(?m)^\$ ?$")
    # 1 KB/s, and the proxy holds little: a network that is barely there.
    proxy.set(dir="down", rate_bps=1000, buffer_bytes=4096)
    base_rss = rss_kb(t, server.pid)
    samples, stop = sample_rss(t, server.pid)
    since = slow.mark()
    start = time.monotonic()
    fast.line(storm, "base64 /dev/urandom")
    dropped_at = t.eventually(
        lambda: (time.monotonic() if fast.viewers(storm) == 1 else None), "the server drops the slow client",
        timeout=timeout_s + 10, interval=0.1)
    gone = slow.wait_event("disconnected", since=since, timeout=15)
    fast.key(storm, "ctrl-c")
    stop.set()
    t.metric("slow_dropped_after_ms", round((dropped_at - start) * 1000))
    t.metric("slow_disconnected_after_drop_ms", round((gone["_at"] - dropped_at) * 1000))
    t.metric("server_rss_base_kb", base_rss)
    t.metric("server_rss_peak_kb", max(samples))
    t.ok(dropped_at - start <= timeout_s + 3, f"dropped within the client timeout ({dropped_at - start:.1f}s)")
    t.ok(server.logged("client stopped answering pings") + server.logged("client too slow") >= 1,
         "the server logged why")
    t.ok(max(samples) - base_rss < 128 * 1024, "the server's memory stays bounded")
    t.ok(gone["_at"] - dropped_at <= 15, f"its connection ends ({gone['_at'] - dropped_at:.1f}s after the drop)")
    t.eq(fast.run(storm, "echo after-storm"), 0, "the pane takes input after the storm")
