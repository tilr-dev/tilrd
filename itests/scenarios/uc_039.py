"""UC-039: a half-open TCP connection: a client that connects and says nothing (or starts
a request and stops) is closed by the server when the handshake timeout (10 s) runs out;
one that finished the WebSocket handshake and then goes silent (no Hello, no pongs) is
dropped by the client timeout.
"""

import base64
import os
import socket
import time

from tt import T, case

HANDSHAKE_S = 10


def closed_after(s: socket.socket, limit: float) -> float:
    """Seconds until the server closes the connection (EOF or reset)."""
    s.settimeout(limit)
    started = time.monotonic()
    try:
        while s.recv(4096):
            pass
    except ConnectionResetError:
        pass
    return time.monotonic() - started


@case(spec=["access-control/a-handshake-has-10-seconds"])
def a_silent_connection_is_closed_by_the_handshake_timeout(t: T):
    """UC-039: connect and silence: closed after ~10 s; a request begun and abandoned: the same."""
    server = t.server()
    silent = socket.create_connection(("127.0.0.1", server.port))
    partial = socket.create_connection(("127.0.0.1", server.port))
    partial.sendall(b"GET / HTTP/1.1\r\nHost: 127.0.0.1\r\n")
    took = closed_after(silent, HANDSHAKE_S + 10)
    t.metric("silent_closed_ms", round(took * 1000))
    t.ok(HANDSHAKE_S - 1 <= took <= HANDSHAKE_S + 3, f"closed after the handshake timeout ({took:.1f}s)")
    took = closed_after(partial, 5)
    t.ok(took < 5, "the abandoned request too")
    t.ok(server.alive(), "the server is up")


@case(spec=["client-liveness/a-silent-client-is-dropped"])
def a_websocket_that_never_says_hello_is_dropped_by_the_client_timeout(t: T):
    """UC-039: WebSocket open, then silence (no Hello, no pongs): dropped within --client-timeout."""
    server = t.server("--client-timeout", 4)
    s = socket.create_connection(("127.0.0.1", server.port))
    key = base64.b64encode(os.urandom(16)).decode()
    s.sendall((f"GET / HTTP/1.1\r\nHost: 127.0.0.1:{server.port}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n"
               f"Sec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n\r\n").encode())
    t.has(s.recv(4096).decode(errors="replace"), "101 Switching Protocols")
    took = closed_after(s, 20)
    t.metric("hello_less_closed_ms", round(took * 1000))
    t.ok(took <= 4 + 3, f"dropped within the client timeout ({took:.1f}s)")
    t.ok(server.alive(), "the server is up")
