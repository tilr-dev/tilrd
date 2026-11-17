"""Harness of the Python library's tests: a real `tilrd` in temporary directories, the
test runtimes (`tests/runtime.py`, the Deno library's `test/runtime.ts`) as separate
processes, and a fake broker that speaks envelopes by hand.

Every wait is bounded by the test's own deadline (`scenario`): a regression fails the
test instead of hanging it. Nothing sleeps to wait for a condition; each wait is woken
by what it waits for (a log line, a printed line, an envelope).
"""

from __future__ import annotations

import asyncio
import contextlib
import functools
import os
import shutil
import signal
import socket
import struct
import sys
import tempfile
from collections.abc import Awaitable, Callable
from pathlib import Path
from typing import Any

import websockets
from websockets.asyncio.server import ServerConnection
from websockets.typing import Subprotocol

from tilr_bus._codec import decode_stream_body, decode_ws, encode_stream, encode_ws
from tilr_bus.proto import bus as wire

SDK = Path(__file__).resolve().parents[1]
REPO = SDK.parents[1]
RUNTIME = SDK / "tests" / "runtime.py"
DENO_RUNTIME = REPO / "sdk" / "deno" / "test" / "runtime.ts"


def tilrd_path() -> str:
    """The tilrd to test: `TILRD`, else the workspace's debug build."""
    path = os.environ.get("TILRD") or str(REPO / "target" / "debug" / "tilrd")
    if not os.path.exists(path):
        raise RuntimeError(f"no tilrd at {path}: cargo build -p tilrd (or set TILRD)")
    return path


def deno_path() -> str:
    path = os.environ.get("DENO") or shutil.which("deno")
    if not path:
        raise RuntimeError("no deno on PATH (scripts/install-tools.sh DIR deno), or set DENO")
    return path


class Lines:
    """Lines of a process's output, as they come; each wait is woken by an arrival."""

    def __init__(self, reader: asyncio.StreamReader) -> None:
        self.lines: list[str] = []
        self._changed = asyncio.Condition()
        self._ended = False
        self._task = asyncio.ensure_future(self._read(reader))

    async def _read(self, reader: asyncio.StreamReader) -> None:
        while line := await reader.readline():
            async with self._changed:
                self.lines.append(line.decode(errors="replace").rstrip("\n"))
                self._changed.notify_all()
        async with self._changed:
            self._ended = True
            self._changed.notify_all()

    async def wait(self, what: str, match: Callable[[str], Any]) -> Any:
        """Wait for a line `match` takes (returns not None); fails if the output ends first."""
        seen = 0
        async with self._changed:
            while True:
                for line in self.lines[seen:]:
                    found = match(line)
                    if found is not None:
                        return found
                seen = len(self.lines)
                if self._ended:
                    raise AssertionError(f"{what}: the output ended:\n" + "\n".join(self.lines))
                await self._changed.wait()


Scope = contextlib.AsyncExitStack


def scenario(timeout: float) -> Callable[[Callable[[Scope], Awaitable[None]]], Callable[[], None]]:
    """A test bounded by `timeout` seconds in all: when it passes, the test fails and
    its processes are killed."""

    def wrap(fn: Callable[[Scope], Awaitable[None]]) -> Callable[[], None]:
        @functools.wraps(fn)
        def test() -> None:
            async def run() -> None:
                async with contextlib.AsyncExitStack() as scope:
                    try:
                        async with asyncio.timeout(timeout):
                            await fn(scope)
                    except TimeoutError:
                        raise AssertionError(
                            f"{fn.__name__}: not done within {timeout} s"
                        ) from None

            asyncio.run(run())

        # pytest reads the signature: the wrapper takes no fixtures.
        del test.__wrapped__
        return test

    return wrap


def temp_dir(scope: Scope) -> str:
    # Short: a Unix socket's path must fit sun_path (104 bytes on macOS).
    path = tempfile.mkdtemp(dir="/tmp", prefix="tp")
    scope.callback(shutil.rmtree, path, True)
    return path


async def _kill(process: asyncio.subprocess.Process) -> None:
    if process.returncode is None:
        with contextlib.suppress(ProcessLookupError):
            process.kill()
    await process.wait()


def _write_config(dir: str, files: dict[str, str]) -> None:
    for name, content in files.items():
        path = Path(f"{dir}/config/{name}")
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content)


class Tilrd:
    """A running tilrd in its own temporary directories."""

    def __init__(self, dir: str, process: asyncio.subprocess.Process, log: Lines, url: str):
        self.dir = dir
        self.process = process
        self.log = log
        #: `ws://127.0.0.1:<port>` of the TCP port.
        self.url = url

    @property
    def socket(self) -> str:
        return f"{self.dir}/runtime/tilrd.sock"

    @property
    def unix_url(self) -> str:
        """`unix:<path>`: the owner on the Unix socket, as `TILR_BUS_URL` names it."""
        return f"unix:{self.socket}"

    def token(self) -> str:
        """The access token of the TCP port."""
        return Path(f"{self.dir}/state/token").read_text().strip()

    @staticmethod
    async def start(scope: Scope, files: dict[str, str] | None = None) -> Tilrd:
        dir = temp_dir(scope)
        for sub in ("config", "state", "runtime"):
            os.mkdir(f"{dir}/{sub}", 0o700)
        _write_config(dir, files or {})
        process = await asyncio.create_subprocess_exec(
            tilrd_path(),
            "--listen",
            "127.0.0.1:0",
            env={
                "TILR_RUNTIME_DIR": f"{dir}/runtime",
                "TILR_STATE_DIR": f"{dir}/state",
                "TILR_CONFIG_DIR": f"{dir}/config",
                "HOME": dir,
                "PATH": "/usr/bin:/bin",
                "SHELL": "/bin/sh",
                "PS1": "$ ",
                "RUST_LOG": "info",
            },
            stdin=asyncio.subprocess.DEVNULL,
            stdout=asyncio.subprocess.DEVNULL,
            stderr=asyncio.subprocess.PIPE,
        )
        scope.push_async_callback(_kill, process)
        assert process.stderr is not None
        log = Lines(process.stderr)

        def listening(line: str) -> str | None:
            _, found, rest = line.partition("listening on ")
            return rest.strip() if found else None

        url = await log.wait("tilrd did not start", listening)
        return Tilrd(dir, process, log, url if url.startswith("ws") else f"ws://{url}")

    async def kill(self) -> None:
        """Kill the server outright (SIGKILL)."""
        self.process.send_signal(signal.SIGKILL)
        await self.process.wait()


def _child_env(env: dict[str, str]) -> dict[str, str]:
    out = {"PATH": os.environ.get("PATH", "/usr/bin:/bin"), "HOME": os.environ.get("HOME", "/tmp")}
    for name in ("DENO_DIR", "XDG_CACHE_HOME", "VIRTUAL_ENV", "PYTHONPATH"):
        if os.environ.get(name):
            out[name] = os.environ[name]
    out.update(env)
    return out


class Runtime:
    """A test runtime process: this library's (`python`) or the Deno library's (`deno`)."""

    def __init__(self, process: asyncio.subprocess.Process) -> None:
        self.process = process
        assert process.stdout is not None and process.stderr is not None
        self.stdout = Lines(process.stdout)
        self.stderr = Lines(process.stderr)

    async def ready(self) -> str:
        """The identity it printed once greeted."""

        def ready(line: str) -> str | None:
            return line.removeprefix("ready ") if line.startswith("ready ") else None

        return await self.stdout.wait("the runtime did not connect", ready)

    @staticmethod
    def command(language: str) -> list[str]:
        if language == "deno":
            return [deno_path(), "run", "--quiet", "-A", str(DENO_RUNTIME)]
        return [sys.executable, str(RUNTIME)]

    @staticmethod
    async def spawn(
        scope: Scope, env: dict[str, str], language: str = "python", fd: int | None = None
    ) -> Runtime:
        """Start one; with `fd`, that descriptor is the child's descriptor 3."""
        argv = Runtime.command(language)
        pass_fds: tuple[int, ...] = ()
        move_fd = None
        if fd is not None:
            # The child gets `fd` as descriptor 3: moved there after the fork. Keeping 3
            # in pass_fds stops the close of the other descriptors, which runs after it.
            # (Not a shell's `3<&N`: dash takes single-digit descriptors only.)
            def move_fd() -> None:
                os.dup2(fd, 3)

            pass_fds = (fd, 3)
        process = await asyncio.create_subprocess_exec(
            *argv,
            env=_child_env(env),
            stdin=asyncio.subprocess.DEVNULL,
            stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.PIPE,
            pass_fds=pass_fds,
            preexec_fn=move_fd,
        )
        scope.push_async_callback(_kill, process)
        return Runtime(process)

    @staticmethod
    async def start(scope: Scope, env: dict[str, str], language: str = "python") -> Runtime:
        """Start one and wait until it is greeted."""
        runtime = await Runtime.spawn(scope, env, language)
        await runtime.ready()
        return runtime


# ---------------------------------------------------------------------------
# The fake broker
# ---------------------------------------------------------------------------


class Peer:
    """The broker's end of one connection, driven step by step by a test."""

    def __init__(
        self,
        send_frame: Callable[[bytes], Awaitable[None]],
        encode: Callable[[wire.Envelope], bytes],
        close: Callable[[], Awaitable[None]],
    ) -> None:
        self._send_frame = send_frame
        self._encode = encode
        self._close = close
        self.queue: asyncio.Queue[wire.Envelope | None] = asyncio.Queue()

    async def send(self, envelope: wire.Envelope) -> None:
        await self._send_frame(self._encode(envelope))

    async def try_recv(self) -> wire.Envelope | None:
        """The next envelope; None once the client closed the connection."""
        return await self.queue.get()

    async def recv(self) -> wire.Envelope:
        envelope = await self.try_recv()
        if envelope is None:
            raise AssertionError("the client closed the connection")
        return envelope

    async def greet(self, **ack: Any) -> wire.Hello:
        """Take the client's Hello and answer HelloAck with `ack`'s fields."""
        hello = await self.recv()
        assert hello.WhichOneof("msg") == "hello", hello
        await self.send(wire.Envelope(hello_ack=hello_ack(**ack)))
        return hello.hello

    async def close(self) -> None:
        await self._close()


def hello_ack(**fields: Any) -> wire.HelloAck:
    values: dict[str, Any] = {
        "protocol_version": 1,
        "identity": "owner",
        "connection_id": 7,
        "max_frame": 16 << 20,
        "max_in_flight": 256,
        "stream_window": 1 << 20,
        "connection_window": 1 << 20,
        "server_id": "server-1",
    }
    values.update(fields)
    return wire.HelloAck(**values)


def _stream_peer(reader: asyncio.StreamReader, writer: asyncio.StreamWriter) -> Peer:
    async def send(frame: bytes) -> None:
        writer.write(frame)
        await writer.drain()

    async def close() -> None:
        writer.close()
        with contextlib.suppress(OSError):
            await writer.wait_closed()

    peer = Peer(send, encode_stream, close)

    async def read() -> None:
        try:
            while True:
                (length,) = struct.unpack(">I", await reader.readexactly(4))
                peer.queue.put_nowait(decode_stream_body(await reader.readexactly(length)))
        except (asyncio.IncompleteReadError, OSError):
            peer.queue.put_nowait(None)

    peer._reader = asyncio.ensure_future(read())  # type: ignore[attr-defined]
    return peer


class UnixListener:
    """A Unix socket the fake broker listens on; `accept` takes one connection."""

    def __init__(self) -> None:
        self.accepted: asyncio.Queue[Peer] = asyncio.Queue()

    @staticmethod
    async def start(scope: Scope, path: str) -> UnixListener:
        listener = UnixListener()

        async def connected(reader: asyncio.StreamReader, writer: asyncio.StreamWriter) -> None:
            peer = _stream_peer(reader, writer)
            scope.push_async_callback(peer.close)
            listener.accepted.put_nowait(peer)

        server = await asyncio.start_unix_server(connected, path)
        scope.push_async_callback(server.wait_closed)
        scope.callback(server.close)
        return listener

    async def accept(self) -> Peer:
        return await self.accepted.get()


async def runtime_on_fd(
    scope: Scope, env: dict[str, str], language: str = "python"
) -> tuple[Peer, Runtime]:
    """Start a test runtime with its descriptor 3 a stream socket the fake broker holds."""
    ours, theirs = socket.socketpair(socket.AF_UNIX, socket.SOCK_STREAM)
    try:
        runtime = await Runtime.spawn(scope, {"TILR_BUS_FD": "3", **env}, language, theirs.fileno())
    finally:
        theirs.close()
    reader, writer = await asyncio.open_unix_connection(sock=ours)
    peer = _stream_peer(reader, writer)
    scope.push_async_callback(peer.close)
    return peer, runtime


class WsListener:
    """A WebSocket server for the fake broker on 127.0.0.1; `accept` takes one client."""

    def __init__(self) -> None:
        self.url = ""
        self.accepted: asyncio.Queue[tuple[Peer, ServerConnection]] = asyncio.Queue()

    @staticmethod
    async def start(scope: Scope) -> WsListener:
        listener = WsListener()

        async def handler(connection: ServerConnection) -> None:
            peer = Peer(connection.send, encode_ws, connection.close)
            listener.accepted.put_nowait((peer, connection))
            try:
                async for message in connection:
                    assert isinstance(message, bytes)
                    peer.queue.put_nowait(decode_ws(message))
            except websockets.exceptions.ConnectionClosed:
                pass
            peer.queue.put_nowait(None)

        server = await websockets.serve(
            handler,
            "127.0.0.1",
            0,
            subprotocols=[Subprotocol("tilr.bus.v1")],
            compression=None,
        )
        scope.push_async_callback(server.wait_closed)
        scope.callback(server.close)
        port = next(iter(server.sockets)).getsockname()[1]
        listener.url = f"ws://127.0.0.1:{port}/"
        return listener

    async def accept(self) -> tuple[Peer, ServerConnection]:
        return await self.accepted.get()
