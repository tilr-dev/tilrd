"""UC-038: many connections at once: 1000 TCP connections that say nothing, 300 that send
an HTTP request a byte a second (slowloris), 1000 that connect and close at once. The
server stays up, a client with the token gets in and works meanwhile, and the idle ones
are closed by the handshake timeout (10 s), so they hold no descriptor longer.

There is no limit on connections as such: each that has not finished its handshake
within 10 s is closed. The server's own descriptor limit is recorded (open_files_limit).
"""

import resource
import socket
import threading
import time

from tt import T, case

HANDSHAKE_S = 10


def fds(t: T, pid: int) -> int:
    return t.proc(pid).fds


def established(t: T, port: int) -> int:
    return len(t.run("ss", "-Htn", "state", "established", f"sport = :{port}", quiet=True).out.splitlines())


def a_client_works(t: T, server, name: str) -> float:
    started = time.monotonic()
    d = t.driver(name)
    d.connect(server)
    pane = d.shell_pane()
    t.eq(d.value(pane, "$((6*7))"), "42", f"{name} works")
    took = time.monotonic() - started
    d.cmd("quit")
    return took


def open_many(port: int, n: int) -> list[socket.socket]:
    return [socket.create_connection(("127.0.0.1", port), timeout=10) for _ in range(n)]


@case(timeout=240, spec=["access-control/a-handshake-has-10-seconds"])
def a_thousand_silent_connections_and_a_slowloris_leave_the_server_working(t: T):
    """UC-038: 1000 silent + 300 slowloris + 1000 connect-and-close: the server serves a client meanwhile."""
    soft, hard = resource.getrlimit(resource.RLIMIT_NOFILE)
    resource.setrlimit(resource.RLIMIT_NOFILE, (hard, hard))
    server = t.server()
    limit = next(line for line in open(f"/proc/{server.pid}/limits") if line.startswith("Max open files"))
    t.metric("open_files_limit", int(limit.split()[3]))
    base = fds(t, server.pid)

    silent = open_many(server.port, 1000)
    t.eventually(lambda: established(t, server.port) >= 1000, "1000 connections accepted", timeout=20)
    t.metric("fds_with_1000_silent", fds(t, server.pid))
    t.metric("client_ms_with_1000_silent", round(a_client_works(t, server, "during-silent") * 1000))
    t.ok(server.alive(), "the server is up")
    closed_at = time.monotonic()
    t.eventually(lambda: established(t, server.port) == 0, "the silent connections are closed by the server",
                 timeout=HANDSHAKE_S + 10)
    t.metric("silent_closed_after_ms", round((time.monotonic() - closed_at) * 1000))
    for s in silent:
        s.close()
    t.eventually(lambda: fds(t, server.pid) <= base + 5, lambda: f"descriptors back near {base}: {fds(t, server.pid)}",
                 timeout=10)

    # Slowloris: an HTTP request a byte a second, never finished.
    stop = threading.Event()
    dropped = []
    loris = open_many(server.port, 300)

    def trickle(s: socket.socket):
        request = b"GET / HTTP/1.1\r\nHost: 127.0.0.1\r\nUpgrade: websocket\r\nX-Slow: " + b"a" * 1000
        started = time.monotonic()
        for byte in request:
            if stop.is_set():
                return
            try:
                s.sendall(bytes([byte]))
            except OSError:
                dropped.append(time.monotonic() - started)
                return
            time.sleep(1)

    threads = [threading.Thread(target=trickle, args=(s,), daemon=True) for s in loris]
    for th in threads:
        th.start()
    time.sleep(2)
    t.metric("client_ms_with_slowloris", round(a_client_works(t, server, "during-slowloris") * 1000))
    t.eventually(lambda: len(dropped) == len(loris), lambda: f"slowloris connections dropped: {len(dropped)}/300",
                 timeout=HANDSHAKE_S + 15)
    stop.set()
    t.metric("slowloris_dropped_after_max_ms", round(max(dropped) * 1000))
    t.ok(max(dropped) <= HANDSHAKE_S + 5, f"each was dropped within the handshake timeout ({max(dropped):.1f}s)")

    # A storm of connect-and-close.
    def storm():
        for _ in range(100):
            try:
                socket.create_connection(("127.0.0.1", server.port), timeout=5).close()
            except OSError:
                pass
    storms = [threading.Thread(target=storm) for _ in range(10)]
    for th in storms:
        th.start()
    took = a_client_works(t, server, "during-storm")
    for th in storms:
        th.join()
    t.metric("client_ms_during_storm", round(took * 1000))
    t.ok(server.alive(), "the server is up after it all")
    a_client_works(t, server, "after")
