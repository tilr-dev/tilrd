"""Layer-1 fault proxy: a TCP relay between a client and tilrd that a case controls.

    p = t.proxy(server.port)          # listens on 127.0.0.1:p.port (p.url is ws://...)
    p.freeze()                        # nothing moves either way; nothing is lost
    p.freeze("down")                  # only server -> client stops ("up" = client -> server)
    p.freeze(conn=1)                  # only connection 1 (numbered from 1 as accepted);
                                      # conn="open": the connections open now, not later ones
    p.resume()                        # held bytes flow again (same `direction` / `conn` choice)
    p.cut()                           # every connection reset (RST) both ways; cut(conn=2)
    p.set(delay_ms=200, jitter_ms=50) # knobs, both directions, or set(dir="down", ...)
    p.stats()                         # per connection: bytes read and written, WS messages

Knobs (per direction; `set(dir="up"|"down"|"both", **knobs)`):

    delay_ms, jitter_ms     every chunk is passed on this much later (jitter: + up to
                            jitter_ms, never overtaking the chunk before: TCP keeps order)
    loss_pct, loss_stall_ms what packet loss does to a TCP stream, seen from above it: a
                            chunk "lost" (with probability loss_pct) is held back for a
                            retransmission timeout (loss_stall_ms, 200 by default, doubling
                            for each loss in a row) and everything behind it waits. Bytes are
                            never dropped, reordered or doubled: real packet loss, reordering
                            and duplication belong to netem (tt.Net), and TCP hides them
    rate_bps                bandwidth in bytes per second (0 = unlimited)
    buffer_bytes            bytes read but not yet passed on before the proxy stops reading
                            (64 KiB): with a low rate the sender's buffers fill, as on a slow link
    cut_after_ws_messages   RST once this many more WebSocket messages went this way on a
                            connection (once; counted from the set)
    cut_after_bytes         RST once this many more bytes went this way (once)
    cut_mid_ws_message      RST in the middle of the next WebSocket message going this way
                            (half of it written) (once)

Every byte is read once and written at most once, in order; stats() shows both counts.
A frozen direction stops reading its source, so the sender's buffers fill as on a network
that silently drops (iptables DROP), and what was held goes on once it is back, as TCP
retransmits; new connections are accepted but frozen too (unless the freeze named
connections).

Also a standalone process, for a proxy in a container of its own (services=["proxy"]):

    python3 -m tt.proxy --listen 0.0.0.0:9000 --control 0.0.0.0:9001 [--target tilrd:7890]

with JSON-lines control on --control: {"op": "freeze", "args": {"direction": "both"}} ->
{"ok": true, "result": ...}; ops: freeze, resume, cut, set, stats, target (host, port).
RemoteProxy speaks it with the same methods as Proxy.
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
KNOBS = {"delay_ms": 0, "jitter_ms": 0, "loss_pct": 0.0, "loss_stall_ms": 200, "rate_bps": 0,
         "buffer_bytes": 64 * 1024, "cut_after_ws_messages": 0, "cut_after_bytes": 0, "cut_mid_ws_message": False}


class WsCounter:
    """Counts complete WebSocket frames in a byte stream (either direction)."""

    def __init__(self):
        self.buf = b""
        self.skip = 0
        self.frames = 0

    def copy(self) -> "WsCounter":
        c = WsCounter()
        c.buf, c.skip, c.frames = self.buf, self.skip, self.frames
        return c

    @property
    def inside(self) -> bool:
        """In the middle of a frame (its header or body partly seen)."""
        return bool(self.skip or self.buf)

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
    def __init__(self, n: int, cr, cw, sr, sw, peer: str):
        self.n = n
        self.peer = peer
        self.opened = time.monotonic()
        self.writers = {"client": cw, "server": sw}
        self.ends = {"up": (cr, sw), "down": (sr, cw)}
        self.read = {"up": 0, "down": 0}
        self.written = {"up": 0, "down": 0}
        self.queued = {"up": 0, "down": 0}
        self.handshake_done = {"up": False, "down": False}
        self.ws = {"up": WsCounter(), "down": WsCounter()}
        self.done = {"up": False, "down": False}
        self.gates = {d: asyncio.Event() for d in DIRS}
        self.queues = {d: asyncio.Queue() for d in DIRS}
        self.room = {d: asyncio.Event() for d in DIRS}
        for d in DIRS:
            self.gates[d].set()
            self.room[d].set()
        self.closed = False


class Proxy:
    def __init__(self, target_host: str | None, target_port: int | None, listen_host: str = "127.0.0.1",
                 listen_port: int = 0):
        self.target_addr = (target_host, target_port)
        self.listen_host = listen_host
        self.loop = asyncio.new_event_loop()
        self.conns: list[Conn] = []
        self.knobs = {d: dict(KNOBS) for d in DIRS}
        self.events: list[tuple[float, str]] = []
        self.base: dict[tuple[int, str, str], int] = {}
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

    @property
    def url(self) -> str:
        host = "127.0.0.1" if self.listen_host in ("0.0.0.0", "") else self.listen_host
        return f"ws://{host}:{self.port}"

    # ---- control (thread-safe)

    def _call(self, fn, *args):
        done = threading.Event()
        result = []

        def run():
            try:
                result.append(fn(*args))
            except Exception as e:  # handed to the caller
                result.append(e)
            done.set()
        self.loop.call_soon_threadsafe(run)
        done.wait(10)
        if result and isinstance(result[0], Exception):
            raise result[0]
        return result[0] if result else None

    def _log(self, what: str):
        self.events.append((time.monotonic(), what))
        print(f"  proxy: {what}", flush=True)

    def _gates(self, direction: str, conn) -> list[asyncio.Event]:
        dirs = DIRS if direction == "both" else (direction,)
        if conn is None:
            return [self.gates[d] for d in dirs]
        if conn == "open":
            conns = [c for c in self.conns if not c.closed]
        else:
            conns = [c for c in self.conns if c.n == conn]
            if not conns:
                raise ValueError(f"no connection {conn}")
        return [c.gates[d] for c in conns for d in dirs]

    def freeze(self, direction: str = "both", conn: int | str | None = None):
        self._call(lambda: [g.clear() for g in self._gates(direction, conn)])
        self._log(f"freeze {direction}" + (f" conn {conn}" if conn is not None else ""))

    def resume(self, direction: str = "both", conn: int | str | None = None):
        self._call(lambda: [g.set() for g in self._gates(direction, conn)])
        self._log(f"resume {direction}" + (f" conn {conn}" if conn is not None else ""))

    def cut(self, conn: int | None = None) -> int:
        """Reset open connections (all, or the one numbered `conn`), both sides, with RST."""
        n = self._call(self._cut, conn)
        self._log(f"cut {n} connection(s) with RST")
        return n

    def set(self, dir: str = "both", **knobs):
        unknown = set(knobs) - set(KNOBS)
        if unknown:
            raise ValueError(f"unknown proxy knobs: {', '.join(sorted(unknown))}")

        def go():
            for d in DIRS if dir == "both" else (dir,):
                self.knobs[d].update(knobs)
                for c in self.conns:
                    self.base[(c.n, d, "ws")] = c.ws[d].frames
                    self.base[(c.n, d, "bytes")] = c.written[d]
        self._call(go)
        self._log(f"set {dir} {knobs}")

    def target(self, host: str, port: int):
        self.target_addr = (host, int(port))
        self._log(f"target {host}:{port}")

    def stats(self) -> list[dict]:
        def snap():
            return [{"n": c.n, "peer": c.peer, "closed": c.closed, "age_s": round(time.monotonic() - c.opened, 3),
                     **{f"read_{d}": c.read[d] for d in DIRS}, **{f"bytes_{d}": c.written[d] for d in DIRS},
                     **{f"queued_{d}": c.queued[d] for d in DIRS}, **{f"ws_{d}": c.ws[d].frames for d in DIRS}}
                    for c in self.conns]
        return self._call(snap)

    def close(self):
        if not self.loop.is_running():
            return
        self._call(self._cut, None)
        self.loop.call_soon_threadsafe(self.server.close)
        self.loop.call_soon_threadsafe(self.loop.stop)

    # ---- relay (loop thread)

    def _cut(self, conn: int | None) -> int:
        n = 0
        for c in self.conns:
            if not c.closed and (conn is None or c.n == conn):
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
        for d in DIRS:
            c.room[d].set()
            c.gates[d].set()
            c.queues[d].put_nowait(None)

    async def _accept(self, cr, cw):
        try:
            sr, sw = await asyncio.open_connection(*self.target_addr)
        except (OSError, TypeError):
            cw.transport.abort()
            return
        peer = cw.get_extra_info("peername")
        c = Conn(len(self.conns) + 1, cr, cw, sr, sw, f"{peer[0]}:{peer[1]}" if peer else "?")
        self.conns.append(c)
        await asyncio.gather(self._read(c, "up"), self._read(c, "down"), self._write(c, "up"), self._write(c, "down"))

    async def _open(self, c: Conn, d: str):
        while not (self.gates[d].is_set() and c.gates[d].is_set()) and not c.closed:
            await self.gates[d].wait()
            await c.gates[d].wait()

    async def _read(self, c: Conn, d: str):
        reader = c.ends[d][0]
        last_due = 0.0
        try:
            while not c.closed:
                await self._open(c, d)
                while c.queued[d] >= self.knobs[d]["buffer_bytes"] and not c.closed:
                    c.room[d].clear()
                    await c.room[d].wait()
                if c.closed:
                    break
                # Never more than the buffer has room for: it bounds what the proxy holds.
                data = await reader.read(max(1, min(65536, self.knobs[d]["buffer_bytes"] - c.queued[d])))
                if not data:
                    break
                c.read[d] += len(data)
                k = self.knobs[d]
                due = time.monotonic() + (k["delay_ms"] + random.uniform(0, k["jitter_ms"])) / 1000
                due = last_due = max(due, last_due)
                c.queued[d] += len(data)
                c.queues[d].put_nowait((due, data))
        except (OSError, asyncio.CancelledError):
            pass
        c.queues[d].put_nowait(None)

    async def _write(self, c: Conn, d: str):
        writer = c.ends[d][1]
        losses = 0
        try:
            while not c.closed:
                item = await c.queues[d].get()
                if item is None:
                    break
                due, data = item
                wait = due - time.monotonic()
                if wait > 0:
                    await asyncio.sleep(wait)
                k = self.knobs[d]
                if k["loss_pct"] and random.random() * 100 < k["loss_pct"]:
                    losses += 1
                    await asyncio.sleep(k["loss_stall_ms"] * 2 ** min(losses - 1, 6) / 1000)
                else:
                    losses = 0
                while data and not c.closed:
                    await self._open(c, d)
                    if c.closed:
                        break
                    rate = self.knobs[d]["rate_bps"]
                    piece = data if not rate else data[:max(256, rate // 20)]
                    if self._cut_now(c, d, piece):
                        return
                    writer.write(piece)
                    await writer.drain()
                    data = data[len(piece):]
                    c.queued[d] -= len(piece)
                    c.room[d].set()
                    c.written[d] += len(piece)
                    self._count(c, d, piece)
                    if self._cut_after(c, d):
                        return
                    if rate:
                        await asyncio.sleep(len(piece) / rate)
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

    def _cut_now(self, c: Conn, d: str, piece: bytes) -> bool:
        """cut_mid_ws_message: write half of a message, then reset."""
        k = self.knobs[d]
        if not k["cut_mid_ws_message"] or not c.handshake_done[d] or len(piece) < 16:
            return False
        half = len(piece) // 2
        probe = c.ws[d].copy()
        probe.feed(piece[:half])
        if not probe.inside:
            return False
        k["cut_mid_ws_message"] = False
        c.ends[d][1].write(piece[:half])
        c.written[d] += half
        self._count(c, d, piece[:half])
        self._reset(c)
        self._log(f"connection {c.n}: cut {d} in the middle of a WebSocket message ({half} of the chunk's "
                  f"{len(piece)} bytes written)")
        return True

    def _cut_after(self, c: Conn, d: str) -> bool:
        k = self.knobs[d]
        limit = k["cut_after_ws_messages"]
        if limit and c.ws[d].frames - self.base.get((c.n, d, "ws"), 0) >= limit:
            k["cut_after_ws_messages"] = 0
            self._reset(c)
            self._log(f"connection {c.n}: cut after {limit} more WebSocket messages {d}")
            return True
        limit = k["cut_after_bytes"]
        if limit and c.written[d] - self.base.get((c.n, d, "bytes"), 0) >= limit:
            k["cut_after_bytes"] = 0
            self._reset(c)
            self._log(f"connection {c.n}: cut after {limit} more bytes {d}")
            return True
        return False

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


class RemoteProxy:
    """A proxy in a container of its own, driven over its control port, with the methods
    of Proxy. `addr` is host:port of the relay; control is on the next port."""

    def __init__(self, addr: str):
        addr = addr.split("://", 1)[-1]
        self.host, _, port = addr.rpartition(":")
        self.port = int(port)
        self.control = (self.host, self.port + 1)
        self.sock = None
        deadline = time.monotonic() + 30
        while self.sock is None:
            try:
                self.sock = socket.create_connection(self.control, timeout=30)
            except OSError:
                if time.monotonic() > deadline:
                    raise
                time.sleep(0.2)
        self.f = self.sock.makefile("rwb", buffering=0)

    @property
    def url(self) -> str:
        return f"ws://{self.host}:{self.port}"

    def _op(self, op: str, **args):
        self.f.write((json.dumps({"op": op, "args": args}) + "\n").encode())
        answer = json.loads(self.f.readline())
        if not answer.get("ok"):
            raise RuntimeError(f"proxy {op}: {answer.get('error')}")
        if op != "stats":
            print(f"  proxy@{self.host}: {op} {args or ''}", flush=True)
        return answer.get("result")

    def freeze(self, direction: str = "both", conn=None):
        return self._op("freeze", direction=direction, conn=conn)

    def resume(self, direction: str = "both", conn=None):
        return self._op("resume", direction=direction, conn=conn)

    def cut(self, conn=None):
        return self._op("cut", conn=conn)

    def set(self, dir: str = "both", **knobs):
        return self._op("set", dir=dir, **knobs)

    def target(self, host: str, port: int):
        return self._op("target", host=host, port=port)

    def stats(self) -> list[dict]:
        return self._op("stats")

    def close(self):
        try:
            self.sock.close()
        except OSError:
            pass


def _main():
    import argparse
    ap = argparse.ArgumentParser(description="tt layer-1 fault proxy")
    ap.add_argument("--listen", default="0.0.0.0:9000")
    ap.add_argument("--target", default="")
    ap.add_argument("--control", default="0.0.0.0:9001")
    args = ap.parse_args()
    lh, lp = args.listen.rsplit(":", 1)
    th, tp = args.target.rsplit(":", 1) if args.target else (None, None)
    proxy = Proxy(th, int(tp) if tp else None, lh, int(lp))
    ch, cp = args.control.rsplit(":", 1)
    ops = {"freeze": proxy.freeze, "resume": proxy.resume, "cut": proxy.cut, "set": proxy.set,
           "stats": proxy.stats, "target": proxy.target}

    async def control(reader, writer):
        while line := await reader.readline():
            try:
                req = json.loads(line)
                kw = req.get("args")
                if kw is None:  # the older form: {"op": "freeze", "dir": "down"}, {"op": "set", "knobs": {}}
                    kw = {k: v for k, v in req.items() if k != "op"}
                    if "dir" in kw and req["op"] in ("freeze", "resume"):
                        kw["direction"] = kw.pop("dir")
                    kw.update(kw.pop("knobs", {}))
                out = await asyncio.to_thread(ops[req["op"]], **kw)
                writer.write((json.dumps({"ok": True, "result": out}) + "\n").encode())
            except Exception as e:
                writer.write((json.dumps({"ok": False, "error": f"{type(e).__name__}: {e}"}) + "\n").encode())
            await writer.drain()

    async def serve():
        server = await asyncio.start_server(control, ch, int(cp))
        print(f"proxy {args.listen} -> {args.target or '(no target yet)'}, control {args.control}", flush=True)
        await server.serve_forever()

    asyncio.run(serve())


if __name__ == "__main__":
    _main()
