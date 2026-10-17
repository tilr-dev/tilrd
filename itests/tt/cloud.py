"""tilr Cloud as a case plays it: the WebSocket a linked server keeps open to Cloud
(the control connection, see crates/tilr-server/src/cloud.rs), and a stand-in
`cloudflared` that notes how it was started and stopped.

    cloud = t.cloud()                      # listens on 127.0.0.1, cloud.url is http://...
    cloud.link(state_dir)                  # state/cloud.json: the server is linked to it
    control = cloud.accept()               # the server's connection, once it comes
    control.greet()                        # its hello, answered with hello_ack
    control.send({"type": "tunnel_open", "tunnel": {...}})
    control.tunnel_is("up")                # read until tunnel_status says so

Messages are JSON in text frames; "ping" / "pong" are bare text. Only what a case needs
of RFC 6455: no extensions, frames from the server are masked, ours are not.
"""

from __future__ import annotations

import base64
import hashlib
import json
import os
import queue
import socket
import struct
import threading
import time
from pathlib import Path

GUID = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11"


class Control:
    """One control connection, seen from Cloud."""

    def __init__(self, conn: socket.socket, path: str, headers: dict):
        self.conn, self.path, self.headers = conn, path, headers
        self.messages: queue.Queue = queue.Queue()
        self.seen: list = []
        self.closed: tuple | None = None
        threading.Thread(target=self._read, daemon=True).start()

    @property
    def authorization(self) -> str:
        return self.headers.get("authorization", "")

    def _recv(self, n: int) -> bytes:
        data = b""
        while len(data) < n:
            chunk = self.conn.recv(n - len(data))
            if not chunk:
                raise EOFError
            data += chunk
        return data

    def _read(self):
        try:
            while True:
                b0, b1 = self._recv(2)
                op, n = b0 & 0x0F, b1 & 0x7F
                if n == 126:
                    n = struct.unpack(">H", self._recv(2))[0]
                elif n == 127:
                    n = struct.unpack(">Q", self._recv(8))[0]
                mask = self._recv(4) if b1 & 0x80 else b"\0\0\0\0"
                payload = bytes(b ^ mask[i % 4] for i, b in enumerate(self._recv(n)))
                if op == 8:
                    code = struct.unpack(">H", payload[:2])[0] if len(payload) >= 2 else None
                    self.closed = (code, payload[2:].decode(errors="replace"))
                    break
                if op == 9:
                    self._frame(10, payload)
                elif op == 1:
                    text = payload.decode()
                    message = text if text in ("ping", "pong") else json.loads(text)
                    self.seen.append(message)
                    self.messages.put(message)
        except (EOFError, OSError, ValueError):
            pass
        if self.closed is None:
            self.closed = (None, "connection ended")
        self.messages.put(None)

    def _frame(self, op: int, payload: bytes):
        head = bytes([0x80 | op])
        n = len(payload)
        head += bytes([n]) if n < 126 else bytes([126]) + struct.pack(">H", n) if n < 65536 \
            else bytes([127]) + struct.pack(">Q", n)
        self.conn.sendall(head + payload)

    def send(self, message):
        """A JSON message (or bare text) in a text frame."""
        text = message if isinstance(message, str) else json.dumps(message)
        print(f"  cloud -> {text[:200]}")
        self._frame(1, text.encode())

    def next(self, timeout: float = 10):
        """The server's next message that is not a keepalive; None once it closed."""
        deadline = time.monotonic() + timeout
        while True:
            left = deadline - time.monotonic()
            if left <= 0:
                raise TimeoutError(f"no message from the server within {timeout}s")
            try:
                message = self.messages.get(timeout=left)
            except queue.Empty:
                continue
            if message in ("ping", "pong"):
                continue
            if message is not None:
                print(f"  cloud <- {json.dumps(message)[:200]}")
            return message

    def greet(self) -> dict:
        """Take the server's hello and answer it."""
        hello = self.next()
        if not isinstance(hello, dict) or hello.get("type") != "hello":
            raise AssertionError(f"expected hello, got {hello!r}")
        self.send({"type": "hello_ack", "protocol": 1, "machine_id": "m1"})
        return hello

    def tunnel_is(self, *states: str, timeout: float = 15) -> dict:
        """Read until the server reports its tunnel in one of `states`."""
        deadline = time.monotonic() + timeout
        while True:
            message = self.next(max(0.1, deadline - time.monotonic()))
            if message is None:
                raise AssertionError(f"the server closed the control connection {self.closed}")
            if message.get("type") == "tunnel_status" and message.get("state") in states:
                return message

    def close(self):
        try:
            self.conn.close()
        except OSError:
            pass


class Cloud:
    """A Cloud that accepts control connections on 127.0.0.1."""

    def __init__(self):
        self.sock = socket.create_server(("127.0.0.1", 0))
        self.port = self.sock.getsockname()[1]
        self.url = f"http://127.0.0.1:{self.port}"
        self.connections: queue.Queue = queue.Queue()
        self.all: list[Control] = []
        threading.Thread(target=self._accept, daemon=True).start()

    def _accept(self):
        while True:
            try:
                conn, _ = self.sock.accept()
            except OSError:
                return
            threading.Thread(target=self._handshake, args=(conn,), daemon=True).start()

    def _handshake(self, conn: socket.socket):
        data = b""
        while b"\r\n\r\n" not in data:
            chunk = conn.recv(4096)
            if not chunk:
                conn.close()
                return
            data += chunk
        head = data.split(b"\r\n\r\n", 1)[0].decode(errors="replace").split("\r\n")
        path = head[0].split()[1]
        headers = {k.strip().lower(): v.strip() for k, _, v in (line.partition(":") for line in head[1:])}
        accept = base64.b64encode(hashlib.sha1((headers.get("sec-websocket-key", "") + GUID).encode())
                                  .digest()).decode()
        conn.sendall(("HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n"
                      f"Sec-WebSocket-Accept: {accept}\r\n\r\n").encode())
        control = Control(conn, path, headers)
        self.all.append(control)
        self.connections.put(control)

    def link(self, state_dir: Path, token: str = "tilr_m_test"):
        """Link the server whose TILR_STATE_DIR this is to this Cloud (as `tilr-server
        cloud login` leaves it)."""
        state_dir.mkdir(parents=True, exist_ok=True)
        path = state_dir / "cloud.json"
        path.write_text(json.dumps({"url": self.url, "machine_id": "m1", "token": token}))
        os.chmod(path, 0o600)

    def accept(self, timeout: float = 20) -> Control:
        """The next control connection."""
        try:
            return self.connections.get(timeout=timeout)
        except queue.Empty:
            raise AssertionError(f"no control connection within {timeout}s") from None

    def close(self):
        for c in self.all:
            c.close()
        try:
            self.sock.close()
        except OSError:
            pass


# The stand-in cloudflared: notes its arguments, config, pid and how it ended in its
# directory, says it registered (as the real one does on stderr), and runs until stopped.
CLOUDFLARED = r"""#!/bin/bash
dir=$(dirname "$0")
echo "$@" >"$dir/args"
cat "$3" >"$dir/config" 2>/dev/null
echo $$ >"$dir/pid"
date +%s.%N >"$dir/started"
trap 'date +%s.%N >"$dir/stopped"; echo TERM >"$dir/ended"; exit 0' TERM
trap 'echo INT >"$dir/ended"; exit 0' INT
echo 'INF Registered tunnel connection connIndex=0 location=test' >&2
while :; do sleep 0.1; done
"""


def fake_cloudflared(dir: Path) -> Path:
    dir.mkdir(parents=True, exist_ok=True)
    exe = dir / "cloudflared"
    exe.write_text(CLOUDFLARED)
    exe.chmod(0o755)
    return exe
