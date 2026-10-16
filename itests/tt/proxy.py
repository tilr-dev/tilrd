"""Layer-1 fault proxy: a TCP relay between a client and tilrd that a case controls.

    p = t.proxy(server.port)          # listens on 127.0.0.1:p.port
    p.freeze()                        # nothing moves either way; nothing is lost
    p.freeze("down")                  # only server -> client stops ("up" = client -> server)
    p.resume()                        # held bytes flow again
    p.cut()                           # every connection reset (RST) both ways
    p.set(delay_ms=200, jitter_ms=50) # every chunk waits before it is passed on
    p.set(cut_after_ws_messages=10)   # RST once 10 more WebSocket messages went server -> client
                                      # on a connection (once; counted from the set)

A frozen direction stops reading its source, so the sender's buffers fill as on a dead
network; new connections are accepted but frozen too. Delay is per chunk and keeps the
order. Knobs for stage 2 (loss, bandwidth, per-direction delay) go into `set`.

Also a standalone process, for a proxy in a container of its own:

    python3 -m tt.proxy --listen 0.0.0.0:9000 --target tilrd:7890 --control 0.0.0.0:9001

with JSON-lines control on --control: {"op": "freeze", "dir": "both"} -> {"ok": true}.
"""

from __future__ import annotations

import asyncio
import json
import random
import socket
import struct
import threading
import time

DIRS = ("up", "down")  # up: client -> server, down: server -> client


class WsCounter:
    """Counts complete WebSocket frames in a byte stream (either direction)."""

    def __init__(self):
        self.buf = b""
        self.skip = 0
        self.frames = 0

    def feed(self, data: bytes) -> int:
        if self.skip:
            take = min(self.skip, len(data))
            self.skip -= take
            data = data[take:]
            if self.skip == 0 and take:
                self.frames += 1
        self.buf += data
        while len(self.buf) >= 2:
            b1 = self.buf[1]
            length, at = b1 & 0x7F, 2
            if length == 126:
                if len(self.buf) < 4:
                    break
                length, at = struct.unpack(">H", self.buf[2:4])[0], 4
            elif length == 127:
                if len(self.buf) < 10:
                    break
                length, at = struct.unpack(">Q", self.buf[2:10])[0], 10
            if b1 & 0x80:
                at += 4
            if len(self.buf) < at:
                break
            body = self.buf[at:]
            if len(body) >= length:
                self.buf = body[length:]
                self.frames += 1
            else:
                self.skip = length - len(body)
                self.buf = b""
        return self.frames


class Conn:
    def __init__(self, n: int, cr, cw, sr, sw):
        self.n = n
        self.writers = {"client": cw, "server": sw}
        self.ends = {"up": (cr, sw), "down": (sr, cw)}
        self.bytes = {"up": 0, "down": 0}
        self.handshake_done = {"up": False, "down": False}
        self.ws = {"up": WsCounter(), "down": WsCounter()}
        self.done = {"up": False, "down": False}
        self.closed = False


class Proxy:
    def __init__(self, target_host: str, target_port: int, listen_host: str = "127.0.0.1", listen_port: int = 0):
        self.target = (target_host, target_port)
        self.loop = asyncio.new_event_loop()
        self.conns: list[Conn] = []
        self.knobs = {"delay_ms": 0, "jitter_ms": 0, "cut_after_ws_messages": 0}
        self.events: list[tuple[float, str]] = []
        self.cut_base: dict[int, int] = {}
        ready = threading.Event()

        def main():
            asyncio.set_event_loop(self.loop)
            self.gates = {d: asyncio.Event() for d in DIRS}
            for g in self.gates.values():
                g.set()
            self.server = self.loop.run_until_complete(
                asyncio.start_server(self._accept, listen_host, listen_port))
            self.port = self.server.sockets[0].getsockname()[1]
            ready.set()
            self.loop.run_forever()

        self.thread = threading.Thread(target=main, daemon=True)
        self.thread.start()
        ready.wait(10)

    # ---- control (thread-safe)

    def _call(self, fn, *args):
        done = threading.Event()
        result = []

        def run():
            result.append(fn(*args))
            done.set()
        self.loop.call_soon_threadsafe(run)
        done.wait(10)
        return result[0] if result else None

    def _log(self, what: str):
        self.events.append((time.monotonic(), what))
        print(f"  proxy: {what}")

    def freeze(self, direction: str = "both"):
        def go():
            for d in DIRS if direction == "both" else (direction,):
                self.gates[d].clear()
        self._call(go)
        self._log(f"freeze {direction}")

    def resume(self, direction: str = "both"):
        def go():
            for d in DIRS if direction == "both" else (direction,):
                self.gates[d].set()
        self._call(go)
        self._log(f"resume {direction}")

    def cut(self):
        """Reset every open connection, both sides, with RST."""
        n = self._call(self._cut_all)
        self._log(f"cut {n} connection(s) with RST")

    def set(self, **knobs):
        unknown = set(knobs) - set(self.knobs)
        if unknown:
            raise ValueError(f"unknown proxy knobs: {', '.join(sorted(unknown))}")
        def go():
            self.knobs.update(knobs)
            if "cut_after_ws_messages" in knobs:
                self.cut_base = {c.n: c.ws["down"].frames for c in self.conns}
        self._call(go)
        self._log(f"set {knobs}")

    def stats(self) -> list[dict]:
        return self._call(lambda: [{"n": c.n, "closed": c.closed, **{f"bytes_{d}": c.bytes[d] for d in DIRS},
                                    **{f"ws_{d}": c.ws[d].frames for d in DIRS}} for c in self.conns])

    def close(self):
        if not self.loop.is_running():
            return
        self._call(self._cut_all)
        self.loop.call_soon_threadsafe(self.server.close)
        self.loop.call_soon_threadsafe(self.loop.stop)

    # ---- relay (loop thread)

    def _cut_all(self) -> int:
        n = 0
        for c in self.conns:
            if not c.closed:
                self._reset(c)
                n += 1
        return n

    def _reset(self, c: Conn):
        c.closed = True
        for w in c.writers.values():
            sock = w.get_extra_info("socket")
            try:
                if sock is not None:
                    sock.setsockopt(socket.SOL_SOCKET, socket.SO_LINGER, struct.pack("ii", 1, 0))
            except OSError:
                pass
            w.transport.abort()

    async def _accept(self, cr, cw):
        try:
            sr, sw = await asyncio.open_connection(*self.target)
        except OSError:
            cw.transport.abort()
            return
        c = Conn(len(self.conns) + 1, cr, cw, sr, sw)
        self.conns.append(c)
        await asyncio.gather(self._pump(c, "up"), self._pump(c, "down"))

    async def _pump(self, c: Conn, d: str):
        reader, writer = c.ends[d]
        try:
            while not c.closed:
                await self.gates[d].wait()
                data = await reader.read(65536)
                if not data:
                    break
                delay = self.knobs["delay_ms"] + random.uniform(0, self.knobs["jitter_ms"])
                if delay:
                    await asyncio.sleep(delay / 1000)
                await self.gates[d].wait()
                if c.closed:
                    break
                writer.write(data)
                await writer.drain()
                c.bytes[d] += len(data)
                self._count(c, d, data)
                limit = self.knobs["cut_after_ws_messages"]
                if limit and d == "down" and c.ws[d].frames - self.cut_base.get(c.n, 0) >= limit:
                    self.knobs["cut_after_ws_messages"] = 0
                    self._reset(c)
                    self._log(f"connection {c.n}: cut after {limit} more WebSocket messages")
        except (OSError, asyncio.CancelledError):
            pass
        # End of this direction: pass the end on; once both ended, close.
        c.done[d] = True
        if not c.closed:
            try:
                writer.write_eof()
            except (OSError, RuntimeError):
                pass
            if all(c.done.values()):
                c.closed = True
                for w in c.writers.values():
                    w.close()

    def _count(self, c: Conn, d: str, data: bytes):
        # The HTTP upgrade comes first; frames start after its blank line.
        if not c.handshake_done[d]:
            head = c.ws[d].buf + data
            end = head.find(b"\r\n\r\n")
            if end < 0:
                c.ws[d].buf = head[-4096:]
                return
            c.handshake_done[d] = True
            c.ws[d].buf = b""
            data = head[end + 4:]
        c.ws[d].feed(data)


def _main():
    import argparse
    ap = argparse.ArgumentParser(description="tt layer-1 fault proxy")
    ap.add_argument("--listen", default="0.0.0.0:9000")
    ap.add_argument("--target", required=True)
    ap.add_argument("--control", default="0.0.0.0:9001")
    args = ap.parse_args()
    lh, lp = args.listen.rsplit(":", 1)
    th, tp = args.target.rsplit(":", 1)
    proxy = Proxy(th, int(tp), lh, int(lp))
    ch, cp = args.control.rsplit(":", 1)

    async def control(reader, writer):
        while line := await reader.readline():
            try:
                req = json.loads(line)
                op = req.pop("op")
                fn = {"freeze": lambda: proxy.freeze(req.get("dir", "both")),
                      "resume": lambda: proxy.resume(req.get("dir", "both")),
                      "cut": proxy.cut, "set": lambda: proxy.set(**req.get("knobs", {})),
                      "stats": proxy.stats}[op]
                out = await asyncio.to_thread(fn)
                writer.write((json.dumps({"ok": True, "result": out}) + "\n").encode())
            except Exception as e:
                writer.write((json.dumps({"ok": False, "error": str(e)}) + "\n").encode())
            await writer.drain()

    async def serve():
        server = await asyncio.start_server(control, ch, int(cp))
        print(f"proxy {args.listen} -> {args.target}, control {args.control}", flush=True)
        await server.serve_forever()

    asyncio.run(serve())


if __name__ == "__main__":
    _main()
