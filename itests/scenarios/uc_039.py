"""UC-039: a half-open TCP connection: a client that connects and says nothing (or starts
a request and stops) is closed by the server when the handshake timeout (10 s) runs out;
one that finished the WebSocket handshake and then goes silent (no Hello, no pongs) is
dropped by the client timeout, and with the client timeout off by the 10 s Hello deadline
(close 4002 `timeout`).
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


def open_websocket(t: T, port: int) -> socket.socket:
    """A raw WebSocket on the port, handshake done, nothing sent after it."""
    s = socket.create_connection(("127.0.0.1", port))
    key = base64.b64encode(os.urandom(16)).decode()
    s.sendall((f"GET / HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n"
               f"Sec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n\r\n").encode())
    t.has(s.recv(4096).decode(errors="replace"), "101 Switching Protocols")
    return s


def close_frame(s: socket.socket, limit: float) -> tuple[float, int | None, str]:
    """Read server frames (answering pings) until the close: seconds taken, close code, reason."""
    s.settimeout(limit)
    started = time.monotonic()
    buf = b""
    while True:
        try:
            chunk = s.recv(4096)
        except (ConnectionResetError, socket.timeout):
            chunk = b""
        if not chunk:
            return time.monotonic() - started, None, ""
        buf += chunk
        while len(buf) >= 2:
            opcode, size = buf[0] & 0x0F, buf[1] & 0x7F
            head = 2
            if size == 126:
                if len(buf) < 4:
                    break
                size, head = int.from_bytes(buf[2:4], "big"), 4
            elif size == 127:
                if len(buf) < 10:
                    break
                size, head = int.from_bytes(buf[2:10], "big"), 10
            if len(buf) < head + size:
                break
            payload, buf = buf[head:head + size], buf[head + size:]
            if opcode == 0x8:
                code = int.from_bytes(payload[:2], "big") if len(payload) >= 2 else None
                return time.monotonic() - started, code, payload[2:].decode(errors="replace")
            if opcode == 0x9:  # ping: a masked pong with the same payload
                mask = os.urandom(4)
                s.sendall(bytes([0x8A, 0x80 | len(payload)]) + mask
                          + bytes(b ^ mask[i % 4] for i, b in enumerate(payload)))


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
    s = open_websocket(t, server.port)
    took = closed_after(s, 20)
    t.metric("hello_less_closed_ms", round(took * 1000))
    t.ok(took <= 4 + 3, f"dropped within the client timeout ({took:.1f}s)")
    t.ok(server.alive(), "the server is up")


@case(spec=["access-control/a-handshake-has-10-seconds#websocket-without-hello"])
def a_websocket_that_never_says_hello_is_closed_after_10_s(t: T):
    """UC-039: --client-timeout 0, WebSocket open, pings answered, no Hello: closed with 4002 `timeout` ~10 s later."""
    server = t.server("--client-timeout", 0)
    s = open_websocket(t, server.port)
    took, code, reason = close_frame(s, HANDSHAKE_S + 10)
    t.metric("no_hello_closed_ms", round(took * 1000))
    t.eq(code, 4002, "close code")
    t.eq(reason, "timeout", "close reason")
    t.ok(HANDSHAKE_S - 1 <= took <= HANDSHAKE_S + 3, f"closed after the Hello deadline ({took:.1f}s)")
    t.ok(server.logged("client sent no Hello in 10s"), "the log says why")
    t.ok(server.alive(), "the server is up")
