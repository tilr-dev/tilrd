"""The bus client for Python (design B4-B7, K2-K3; the Rust client
`crates/tilr-bus/src/client.rs` is the reference).

A connection has one reader task, which dispatches what arrives, and one writer task,
which writes what the participant sends in the order sent. Calls wait for a slot
while `HelloAck.max_in_flight` of them are pending; sends wait while 4 MiB are queued
for the socket. Each incoming call runs as its own task, so a handler awaiting a
nested call holds up nobody; a `Cancel` cancels that task and no reply is sent.
"""

from __future__ import annotations

import asyncio
import base64
import collections
import inspect
import logging
import os
import time
from collections.abc import AsyncIterator, Awaitable, Callable, Mapping
from dataclasses import dataclass
from typing import Any

from tilr_bus import _transport
from tilr_bus._codec import MAX_FRAME, CodecError, chunk_bytes
from tilr_bus._names import matches, valid_method, valid_pattern, valid_subject
from tilr_bus._transport import CloseInfo, Incoming, Link, TransportError
from tilr_bus.proto import bus as wire

#: Handler failures that end nothing (an event handler's) are logged here.
logger = logging.getLogger("tilr_bus")

PROTOCOL_VERSION = 1
CONTENT_TYPE_PROTOBUF = "application/x-protobuf"
CONTENT_TYPE_JSON = "application/json"


class codes:
    """`Error.code` and `ConnectionError.code` values (design B5)."""

    DENIED = "denied"
    NOT_FOUND = "not_found"
    UNAVAILABLE = "unavailable"
    TIMEOUT = "timeout"
    BUSY = "busy"
    TOO_LARGE = "too_large"
    BAD_REQUEST = "bad_request"
    HANDLER_ERROR = "handler_error"
    BAD_REPLY = "bad_reply"
    NOT_SUPPORTED = "not_supported"
    CANCELLED = "cancelled"
    SPAWN_FAILED = "spawn_failed"
    START_FAILED = "start_failed"
    START_TIMEOUT = "start_timeout"
    BAD_CONFIG = "bad_config"
    EXPECTED_HELLO = "expected_hello"
    UNSUPPORTED_VERSION = "unsupported_version"
    RESERVED = "reserved"
    BAD_FRAME = "bad_frame"


class env:
    """Environment variables of the transports (design B10, K2)."""

    BUS_FD = "TILR_BUS_FD"
    BUS_SOCK = "TILR_BUS_SOCK"
    BUS_TOKEN = "TILR_BUS_TOKEN"
    BUS_URL = "TILR_BUS_URL"
    BUS_CREDENTIAL = "TILR_BUS_CREDENTIAL"
    INSTANCE = "TILR_INSTANCE"
    CONFIG_DIR = "TILR_CONFIG_DIR"


#: Bytes of envelopes queued for the socket before sends wait.
SEND_BUDGET = 4 << 20
#: Events held for a subscription that is not read; more are dropped (B6).
SUBSCRIPTION_QUEUE = 1024
HANDSHAKE_TIMEOUT = 10.0
#: How long a `ConnectionError` waits for the close that follows it (to keep its code).
_CLOSE_AFTER_ERROR = 2.0
_DEFAULT_MAX_IN_FLIGHT = 256
_DEFAULT_STREAM_WINDOW = 1 << 20
_MAX_STREAM_WINDOW = 16 << 20


class BusError(Exception):
    """A failure with a code of `codes` and a message meant for the user."""

    def __init__(self, code: str, message: str = "") -> None:
        super().__init__(code, message)
        self.code = code
        self.message = message

    def __str__(self) -> str:
        return f"{self.code}: {self.message}"


Payload = bytes | bytearray | memoryview | str


def _bytes(payload: Payload | None) -> bytes:
    if payload is None:
        return b""
    if isinstance(payload, str):
        return payload.encode()
    return bytes(payload)


def credential_bytes(credential: bytes | str) -> bytes:
    """A token as bytes: bytes as they are, text as `tilr bus token` prints it (base64url)."""
    if isinstance(credential, bytes | bytearray):
        return bytes(credential)
    text = credential.strip().rstrip("=")
    try:
        # validate=True: a character outside the alphabet is an error, not skipped.
        standard = text.replace("-", "+").replace("_", "/") + "=" * (-len(text) % 4)
        return base64.b64decode(standard, validate=True)
    except ValueError as err:
        raise BusError(codes.BAD_REQUEST, f"credential is not base64url: {err}") from err


@dataclass(frozen=True)
class Reply:
    """The answer to a call, or what a handler answers."""

    payload: bytes = b""
    #: Empty means `application/x-protobuf`.
    content_type: str = ""

    def text(self) -> str:
        return self.payload.decode()


@dataclass(frozen=True)
class Event:
    """An event a subscription received."""

    subject: str
    payload: bytes
    content_type: str
    #: The emitter's identity, set by the broker.
    from_: str
    #: `pane:<id>` for the core's pane events; empty otherwise.
    scope: str


def _error_of(error: BaseException) -> wire.Error:
    """A handler's failure as the reply's error: a BusError keeps its code."""
    if isinstance(error, BusError):
        return wire.Error(code=error.code, message=error.message)
    return wire.Error(code=codes.HANDLER_ERROR, message=f"{type(error).__name__}: {error}")


def _to_bus_error(error: BaseException) -> BusError:
    if isinstance(error, BusError):
        return error
    if isinstance(error, CodecError):
        return BusError(codes.TOO_LARGE if error.too_large else codes.BAD_REQUEST, str(error))
    if isinstance(error, TransportError):
        return BusError(error.code, error.message)
    return BusError(codes.UNAVAILABLE, str(error))


class _Slots:
    """A counting semaphore whose waiters are served in order and fail on close."""

    def __init__(self, count: int) -> None:
        self._free = count
        self._waiters: collections.deque[asyncio.Future[None]] = collections.deque()
        self._closed: BusError | None = None

    async def acquire(self) -> None:
        if self._closed:
            raise self._closed
        if self._free > 0 and not self._waiters:
            self._free -= 1
            return
        waiter = asyncio.get_running_loop().create_future()
        self._waiters.append(waiter)
        try:
            await waiter
        except asyncio.CancelledError:
            if waiter.done() and not waiter.cancelled() and waiter.exception() is None:
                self.release()  # a slot handed over just as the waiter gave up
            elif waiter in self._waiters:
                self._waiters.remove(waiter)
            raise

    def release(self) -> None:
        while self._waiters:
            waiter = self._waiters.popleft()
            if not waiter.done():
                waiter.set_result(None)
                return
        self._free += 1

    def close(self, error: BusError) -> None:
        self._closed = error
        while self._waiters:
            waiter = self._waiters.popleft()
            if not waiter.done():
                waiter.set_exception(error)


class _CreditGate:
    """Credit a provider holds for one stream (B7)."""

    def __init__(self, initial: int) -> None:
        self.credit = initial
        self._changed = asyncio.Event()
        self.closed = False

    def add(self, count: int) -> None:
        self.credit += count
        self._changed.set()

    def close(self) -> None:
        self.closed = True
        self._changed.set()

    def try_take(self, count: int) -> bool:
        if self.closed:
            raise BusError(codes.CANCELLED, "the stream has ended")
        if self.credit >= count:
            self.credit -= count
            return True
        return False

    async def take(self, count: int) -> None:
        while not self.try_take(count):
            self._changed.clear()
            await self._changed.wait()


class Context:
    """What a handler knows of the call it answers."""

    def __init__(
        self,
        bus: Bus,
        call: wire.Call,
    ) -> None:
        self.bus = bus
        #: The call's id on this connection.
        self.id = call.id
        self.method = call.method
        #: The caller's identity, set by the broker.
        self.from_ = getattr(call, "from")
        self.content_type = call.content_type
        self.trace_id = call.trace_id
        #: `Call.timeout_ms` as received: the time the caller had left (0 = none).
        self.timeout_ms = call.timeout_ms
        #: `time.monotonic()` when the caller's time is up; None without a deadline.
        self.deadline = time.monotonic() + call.timeout_ms / 1000 if call.timeout_ms else None
        #: The per-call credential the caller presented (empty = none).
        self.credential = call.credential
        #: Set when the call is cancelled (the caller's `Cancel`, its deadline).
        self.cancelled = asyncio.Event()

    def time_left(self) -> float | None:
        """Seconds until the deadline; None without one."""
        if self.deadline is None:
            return None
        return max(0.0, self.deadline - time.monotonic())

    def _nested(self, kwargs: dict[str, Any]) -> dict[str, Any]:
        if kwargs.get("trace_id") is None and self.trace_id:
            kwargs["trace_id"] = self.trace_id
        left = self.time_left()
        if kwargs.get("timeout") is None and left is not None:
            kwargs["timeout"] = max(0.001, left)
        return kwargs

    async def call(self, method: str, payload: Payload | None = None, **kwargs: Any) -> Reply:
        """A nested call carrying this call's trace id and the time it has left."""
        return await self.bus.call(method, payload, **self._nested(kwargs))

    def stream(self, method: str, payload: Payload | None = None, **kwargs: Any) -> ChunkStream:
        """A nested stream, as `call`."""
        return self.bus.stream(method, payload, **self._nested(kwargs))


class StreamWriter:
    """Writes the chunks of a provided stream, within the credit its reader gave."""

    def __init__(
        self, bus: Bus, call_id: int, gate: _CreditGate, window: int, served: _Served
    ) -> None:
        self.bus = bus
        self.id = call_id
        self._gate = gate
        self._served = served
        #: The reader's initial window; no chunk may be larger.
        self.window = window

    @property
    def credit(self) -> int:
        """Payload bytes this provider may send now."""
        return self._gate.credit

    @property
    def credited(self) -> int:
        """Payload bytes the reader's `Credit`s gave back so far (beyond the window)."""
        return self._served.credited

    def would_wait(self, count: int) -> bool:
        """Whether `write` of `count` bytes would wait for credit now."""
        return self._gate.credit < count

    def _check(self, payload: Payload) -> bytes:
        data = _bytes(payload)
        if len(data) > self.window:
            raise BusError(
                codes.TOO_LARGE,
                f"a chunk of {len(data)} bytes is larger than the stream's window of {self.window}",
            )
        return data

    async def write(self, payload: Payload, content_encoding: str = "") -> None:
        """Send a chunk once the reader's credit allows it."""
        data = self._check(payload)
        await self._gate.take(len(data))
        chunk = wire.Chunk(id=self.id, payload=data, content_encoding=content_encoding)
        await self.bus._send(wire.Envelope(chunk=chunk))

    async def try_write(self, payload: Payload, content_encoding: str = "") -> bool:
        """Send a chunk only if the credit allows it now; False when it does not."""
        data = self._check(payload)
        if not self._gate.try_take(len(data)):
            return False
        chunk = wire.Chunk(id=self.id, payload=data, content_encoding=content_encoding)
        await self.bus._send(wire.Envelope(chunk=chunk))
        return True


class ChunkStream:
    """The chunks of a stream, as an async iterator of payloads (decompressed when
    their producer compressed them). The call is sent when the iteration begins;
    credit goes back as each chunk is taken. `end` holds the `End` payload once it
    ended.

    Read it in `async with`, so leaving the block (a `break`, an exception) cancels
    the stream at once: the provider gets `Cancel` and the call's slot is free::

        async with bus.stream("demo.lines") as lines:
            async for line in lines:
                if done(line):
                    break

    `cancel()` and `aclose()` cancel it too. A plain `async for` left early cancels it
    only when the stream is collected (`__del__`, at once in CPython while nothing else
    holds it): a best effort, not a promise."""

    def __init__(self, bus: Bus, start: Callable[[ChunkStream], Awaitable[None]]) -> None:
        self.bus = bus
        self.id = 0
        #: The `End` payload, once the stream ended.
        self.end: bytes | None = None
        # The bus holds this queue, not the stream: dropping the stream cancels it.
        self._items: asyncio.Queue[tuple[str, Any, int]] = asyncio.Queue()
        self._start: Callable[[ChunkStream], Awaitable[None]] | None = start
        self._done = False
        self._cancelled = False

    def __aiter__(self) -> ChunkStream:
        return self

    async def __anext__(self) -> bytes:
        if self._done:
            raise StopAsyncIteration
        if self._start is not None:
            start, self._start = self._start, None
            try:
                await start(self)
            except BaseException as error:
                self._finish()
                if isinstance(error, Exception):
                    raise _to_bus_error(error) from None
                raise
            if self._done:  # cancelled meanwhile
                raise StopAsyncIteration
        kind, value, carried = await self._items.get()
        if kind == "chunk":
            self.bus._credit(self.id, carried)
            return value
        self._finish()
        if kind == "end":
            self.end = value
            raise StopAsyncIteration
        raise value

    async def aclose(self) -> None:
        self.cancel()

    async def __aenter__(self) -> ChunkStream:
        return self

    async def __aexit__(self, *exc: object) -> None:
        self.cancel()

    def cancel(self) -> None:
        """Stop the stream (`Cancel`)."""
        if self._done:
            return
        self._cancelled = True
        self.bus._cancel_stream(self.id)
        self._finish()

    def _finish(self) -> None:
        if self._done:
            return
        self._done = True
        self.bus._release_stream(self)

    def __del__(self) -> None:
        # Best effort: a loop left early outside `async with` drops the last reference.
        if not self._done and self.id:
            try:
                self.cancel()
            except Exception:
                pass


class Subscription:
    """The events of one subscription: handed to its handler, or read with `async for`
    when it has none (1024 unread at most; more are dropped and counted)."""

    def __init__(self, bus: Bus, sub_id: int, pattern: str) -> None:
        self.bus = bus
        self.id = sub_id
        self.pattern = pattern
        self.dropped = 0
        self._queue: asyncio.Queue[Event | None] = asyncio.Queue(SUBSCRIPTION_QUEUE)
        self._task: asyncio.Task[None] | None = None

    def _deliver(self, event: Event | None) -> None:
        try:
            self._queue.put_nowait(event)
        except asyncio.QueueFull:
            if event is not None:
                self.dropped += 1

    def __aiter__(self) -> AsyncIterator[Event]:
        return self._events()

    async def _events(self) -> AsyncIterator[Event]:
        while (event := await self._queue.get()) is not None:
            yield event

    def unsubscribe(self) -> None:
        """End the subscription (`Unsubscribe`)."""
        self.bus._unsubscribe(self)
        if self._task:
            self._task.cancel()
        self._deliver(None)

    async def __aenter__(self) -> Subscription:
        return self

    async def __aexit__(self, *exc: object) -> None:
        self.unsubscribe()


UnaryHandler = Callable[[bytes, Context], Any]
StreamHandler = Callable[[bytes, Context, StreamWriter], Any]


@dataclass
class _Provided:
    spec: wire.MethodSpec
    handler: Callable[..., Any]
    stream: bool


@dataclass
class _Served:
    """A call this participant is answering."""

    ctx: Context
    gate: _CreditGate | None
    task: asyncio.Task[None] | None = None
    #: Bytes the reader's `Credit`s gave (counted apart from the gate).
    credited: int = 0


@dataclass
class _CallerStream:
    """A stream this participant reads."""

    queue: asyncio.Queue[tuple[str, Any, int]]
    allowed: int
    received: int = 0
    #: Set once the `Call` is queued for the socket: a `Cancel` may follow it.
    queued: bool = False
    #: Cancelled before its `Call` was queued: the `Cancel` goes out after it.
    cancelled: bool = False
    timer: asyncio.TimerHandle | None = None


class _ConnectDescriptor:
    """`Bus.connect(...)` makes a Bus and connects it; `bus.connect(...)` connects that one."""

    def __get__(self, bus: Bus | None, owner: type[Bus]) -> Callable[..., Awaitable[Bus]]:
        if bus is None:

            async def connect_new(**options: Any) -> Bus:
                return await owner(**options)._connect()

            return connect_new

        async def connect(**options: Any) -> Bus:
            return await bus._connect(**options)

        return connect


class Bus:
    """A connection to the bus. Provide methods with `provide` before `connect`; then
    `call`, `stream`, `emit`, `on`.

    ```python
    bus = Bus(client_name="demo")

    @bus.provide("demo.echo")
    async def echo(payload, ctx):
        return payload

    await bus.connect()  # TILR_BUS_FD, TILR_BUS_SOCK + TILR_BUS_TOKEN, or TILR_BUS_URL
    print((await bus.call("demo.echo", "hi")).text())
    ```
    """

    connect = _ConnectDescriptor()

    def __init__(
        self,
        *,
        url: str | None = None,
        credential: bytes | str | None = None,
        access_token: str | None = None,
        fd: int | None = None,
        socket: tuple[str, str] | None = None,
        origin: str | None = None,
        client_name: str = "",
        client_id: str = "",
        handshake_timeout: float = HANDSHAKE_TIMEOUT,
        environ: Mapping[str, str] | None = None,
    ) -> None:
        """Options of the connection. Without `url`, `fd` or `socket` (an instance
        socket's path and token) the environment names the transport."""
        self._options: dict[str, Any] = {
            "url": url,
            "credential": credential,
            "access_token": access_token,
            "fd": fd,
            "socket": socket,
            "origin": origin,
            "client_name": client_name,
            "client_id": client_id,
            "handshake_timeout": handshake_timeout,
            "environ": environ,
        }
        self._provided: dict[str, _Provided] = {}
        self._link: Link | None = None
        self._connecting = False
        self._ack = wire.HelloAck()
        self._max_frame = MAX_FRAME
        self._stream_window = _DEFAULT_STREAM_WINDOW
        self._in_flight = _Slots(_DEFAULT_MAX_IN_FLIGHT)
        self._next_id = 0
        self._calls: dict[int, asyncio.Future[Reply]] = {}
        self._streams: dict[int, _CallerStream] = {}
        self._subscriptions: dict[int, Subscription] = {}
        self._served: dict[int, _Served] = {}
        self._pending_error: tuple[str, str] | None = None
        self._pending_timer: asyncio.TimerHandle | None = None
        self._queue: collections.deque[tuple[bytes, bool]] = collections.deque()
        self._queued = 0
        self._wake_writer = asyncio.Event()
        self._budget = asyncio.Event()
        self._credential: bytes | None = None
        self._close_info: CloseInfo | None = None
        self._closed = asyncio.Event()
        self._tasks: set[asyncio.Task[Any]] = set()
        self._panes: Any = None

    # ------------------------------------------------------------------
    # Providing and connecting
    # ------------------------------------------------------------------

    def provide(
        self,
        method: str,
        handler: Callable[..., Any] | None = None,
        *,
        kind: str | None = None,
        lane: str = "interactive",
        select: str = "round_robin",
    ) -> Any:
        """Provide `method`, answered by `handler(payload, ctx)` (or, for a stream,
        `handler(payload, ctx, out)`); only before `connect` (there is no later
        registration, B4). Without `handler` it is a decorator.

        Name the kind: `kind="stream"` for a stream handler, `kind="unary"` for one
        that answers once. Without it, a handler whose signature has three parameters
        or more is taken for a stream handler; a wrapped or `*args` function may not
        show them, so pass `kind`."""
        if handler is None:

            def decorate(function: Callable[..., Any]) -> Callable[..., Any]:
                self.provide(method, function, kind=kind, lane=lane, select=select)
                return function

            return decorate
        if self._link is not None or self._connecting:
            raise BusError(
                codes.BAD_REQUEST,
                f"provide {method} before connect: there is no later registration",
            )
        if not valid_method(method):
            raise BusError(codes.BAD_REQUEST, f"{method!r} is not a method name")
        if kind is None:
            stream = len(inspect.signature(handler).parameters) >= 3
        else:
            stream = kind == "stream"
        spec = wire.MethodSpec(
            method=method,
            kind=wire.STREAM if stream else wire.UNARY,
            lane=wire.BULK if lane == "bulk" else wire.INTERACTIVE,
            select=wire.LAST_ACTIVE if select == "last_active" else wire.ROUND_ROBIN,
        )
        self._provided[method] = _Provided(spec=spec, handler=handler, stream=stream)
        return handler

    def _transport(self, options: dict[str, Any]) -> tuple[str, Any, dict[str, Any]]:
        """The transport the options or the environment name, in the order of K2."""
        credential = options.get("credential")
        credential = None if credential is None else credential_bytes(credential)
        access_token = options.get("access_token") or ""
        if options.get("fd") is not None:
            return "fd", options["fd"], {}
        if options.get("socket") is not None:
            path, token = options["socket"]
            return "socket", path, {"instance_token": token}
        ws_auth = {"access_token": access_token, "credential": credential}
        if options.get("url"):
            return "ws", options["url"], ws_auth
        environ = options.get("environ")
        lookup = environ if environ is not None else os.environ
        fd, sock, token, url, env_credential = (
            lookup.get(name) or None
            for name in (env.BUS_FD, env.BUS_SOCK, env.BUS_TOKEN, env.BUS_URL, env.BUS_CREDENTIAL)
        )
        if fd is not None:
            if not fd.isdigit():
                raise BusError(codes.BAD_REQUEST, f"{env.BUS_FD}={fd} is not a descriptor")
            return "fd", int(fd), {}
        if sock is not None:
            if token is None:
                raise BusError(codes.UNAVAILABLE, f"{env.BUS_SOCK} is set without {env.BUS_TOKEN}")
            return "socket", sock, {"instance_token": token}
        if url is not None:
            if credential is None and env_credential:
                ws_auth["credential"] = credential_bytes(env_credential)
            return "ws", url, ws_auth
        raise BusError(
            codes.UNAVAILABLE,
            f"no bus to connect to: set {env.BUS_FD}, {env.BUS_SOCK} and {env.BUS_TOKEN}, "
            f"or {env.BUS_URL} (and {env.BUS_CREDENTIAL})",
        )

    async def _open(self, kind: str, where: Any, origin: str | None) -> Link:
        if kind == "fd":
            return await _transport.open_fd(where)
        if kind == "socket":
            return await _transport.open_socket(where)
        if where.startswith("unix:"):
            return await _transport.open_unix_ws(where[len("unix:") :], origin)
        return await _transport.open_ws(where, origin)

    async def _connect(self, **overrides: Any) -> Bus:
        if self._link is not None or self._connecting:
            raise BusError(codes.BAD_REQUEST, "connected already")
        options = {**self._options, **{k: v for k, v in overrides.items() if v is not None}}
        kind, where, auth = self._transport(options)
        self._connecting = True
        link: Link | None = None

        async def handshake() -> wire.HelloAck:
            nonlocal link
            link = await self._open(kind, where, options.get("origin"))
            hello = wire.Hello(
                protocol_version=PROTOCOL_VERSION,
                client_name=options.get("client_name") or "",
                client_id=options.get("client_id") or "",
                access_token=auth.get("access_token") or "",
                credential=auth.get("credential") or b"",
                instance_token=auth.get("instance_token") or "",
                provides=[provided.spec for provided in self._provided.values()],
            )
            await link.write(link.encode(wire.Envelope(hello=hello), MAX_FRAME))
            incoming = await link.recv()
            if incoming.closed is not None:
                info = incoming.closed
                if info.error:
                    raise BusError(*info.error)
                detail = "" if info.code is None else f" ({info.code} {info.reason})"
                raise BusError(codes.UNAVAILABLE, f"closed before HelloAck{detail}")
            if incoming.bad is not None:
                raise BusError(codes.BAD_FRAME, incoming.bad)
            envelope = incoming.envelope
            assert envelope is not None
            which = envelope.WhichOneof("msg")
            if which == "connection_error":
                refused: Any = envelope.connection_error  # its .pyi type shadows a builtin
                raise BusError(refused.code, refused.message)
            if which != "hello_ack":
                raise BusError(codes.BAD_REPLY, "the broker did not answer Hello with HelloAck")
            ack = envelope.hello_ack
            if ack.protocol_version != PROTOCOL_VERSION:
                raise BusError(
                    codes.UNSUPPORTED_VERSION,
                    f"unsupported protocol version {ack.protocol_version}: "
                    f"this client speaks {PROTOCOL_VERSION}",
                )
            return ack

        timeout = options.get("handshake_timeout") or HANDSHAKE_TIMEOUT
        try:
            async with asyncio.timeout(timeout):
                ack = await handshake()
        except TimeoutError:
            self._connecting = False
            if link is not None:
                await link.close()
            raise BusError(
                codes.TIMEOUT, f"no HelloAck within {int(timeout * 1000)} ms of connecting"
            ) from None
        except BaseException as error:
            self._connecting = False
            if link is not None:
                await link.close()
            raise _to_bus_error(error) from error
        assert link is not None
        self._ack = ack
        self._max_frame = min(ack.max_frame or MAX_FRAME, MAX_FRAME)
        self._stream_window = ack.stream_window or _DEFAULT_STREAM_WINDOW
        self._in_flight = _Slots(ack.max_in_flight or _DEFAULT_MAX_IN_FLIGHT)
        self._credential = ack.credential or auth.get("credential")
        self._link = link
        self._connecting = False
        self._spawn(self._read_loop())
        self._spawn(self._write_loop())
        return self

    def _spawn(self, coroutine: Awaitable[Any]) -> asyncio.Task[Any]:
        task = asyncio.ensure_future(coroutine)
        self._tasks.add(task)
        task.add_done_callback(self._tasks.discard)
        return task

    # ------------------------------------------------------------------
    # What the connection is
    # ------------------------------------------------------------------

    @property
    def identity(self) -> str:
        """This connection's identity, set by the broker: `owner`, `instance:<name>`, ..."""
        return self._ack.identity

    @property
    def connection_id(self) -> int:
        return self._ack.connection_id

    @property
    def hello_ack(self) -> wire.HelloAck:
        return self._ack

    @property
    def credential(self) -> bytes | None:
        """The token this connection holds: the one presented, an instance's from
        HelloAck, or the latest `CredentialUpdate`."""
        return self._credential

    @property
    def is_closed(self) -> bool:
        return self._close_info is not None

    @property
    def pane(self) -> Any:
        """Typed helpers for the core's `pane.*` methods and events."""
        if self._panes is None:
            from tilr_bus.pane import Panes

            self._panes = Panes(self)
        return self._panes

    async def wait_closed(self) -> CloseInfo:
        """Wait until the connection has ended, and say how."""
        await self._closed.wait()
        assert self._close_info is not None
        return self._close_info

    def close(self) -> None:
        """End the connection: pending calls fail `unavailable`, the socket is closed."""
        self._shutdown(CloseInfo(reason="closed by the client"))

    async def update_credential(self, credential: bytes | str) -> None:
        """Present a new token on this connection (after `bus.renew`)."""
        data = credential_bytes(credential)
        update = wire.CredentialUpdate(credential=data)
        await self._send(wire.Envelope(credential_update=update))
        self._credential = data

    async def renew(self) -> bytes:
        """Renew this connection's token (`bus.renew`) and present the new one."""
        if not self._credential:
            raise BusError(codes.BAD_REQUEST, "this connection holds no token to renew")
        request = wire.RenewRequest(credential=self._credential)
        reply = await self.call("bus.renew", request.SerializeToString())
        renewed = wire.RenewReply.FromString(reply.payload).credential
        await self.update_credential(renewed)
        return renewed

    # ------------------------------------------------------------------
    # Calls, streams, events
    # ------------------------------------------------------------------

    def _require_open(self) -> None:
        if self._link is None:
            raise BusError(codes.UNAVAILABLE, "not connected")
        if self._close_info is not None:
            raise self._closed_error()

    def _call_message(
        self,
        call_id: int,
        method: str,
        payload: Payload | None,
        target: str | None,
        timeout: float | None,
        content_type: str | None,
        credential: bytes | str | None,
        trace_id: str | None,
        window: int,
    ) -> wire.Envelope:
        return wire.Envelope(
            call=wire.Call(
                id=call_id,
                target=target or "",
                method=method,
                payload=_bytes(payload),
                trace_id=trace_id or "",
                timeout_ms=0 if timeout is None else max(1, int(timeout * 1000)),
                content_type=content_type or "",
                credential=b"" if credential is None else credential_bytes(credential),
                window=window,
            )
        )

    async def call(
        self,
        method: str,
        payload: Payload | None = None,
        *,
        target: str | None = None,
        timeout: float | None = None,
        content_type: str | None = None,
        credential: bytes | str | None = None,
        trace_id: str | None = None,
    ) -> Reply:
        """Call `method` and wait for its reply. Waits for a slot while `max_in_flight`
        calls and streams are pending; a payload over `max_frame` fails `too_large`
        unsent; `timeout` (seconds) is the call's deadline, sent and kept here."""
        self._require_open()
        try:
            await self._in_flight.acquire()
        except BusError:
            raise self._closed_error() from None
        try:
            if self._close_info is not None:
                raise self._closed_error()
            self._next_id += 1
            call_id = self._next_id
            answer: asyncio.Future[Reply] = asyncio.get_running_loop().create_future()
            self._calls[call_id] = answer
            envelope = self._call_message(
                call_id, method, payload, target, timeout, content_type, credential, trace_id, 0
            )
            try:
                await self._send(envelope)
            except BaseException:
                self._calls.pop(call_id, None)  # never sent: nothing to cancel
                raise
            try:
                if timeout is None:
                    return await asyncio.shield(answer)
                async with asyncio.timeout(timeout):
                    return await asyncio.shield(answer)
            except TimeoutError:
                assert timeout is not None
                if self._calls.pop(call_id, None) is not None:
                    self._send_now(wire.Envelope(cancel=wire.Cancel(id=call_id, reason="timeout")))
                raise BusError(
                    codes.TIMEOUT, f"{method} did not answer within {int(timeout * 1000)} ms"
                ) from None
            except asyncio.CancelledError:
                # The caller gave up: tell the broker.
                if self._calls.pop(call_id, None) is not None:
                    cancel = wire.Cancel(id=call_id, reason="cancelled by the caller")
                    self._send_now(wire.Envelope(cancel=cancel))
                raise
        finally:
            self._in_flight.release()

    def stream(
        self,
        method: str,
        payload: Payload | None = None,
        *,
        window: int = 0,
        target: str | None = None,
        timeout: float | None = None,
        content_type: str | None = None,
        credential: bytes | str | None = None,
        trace_id: str | None = None,
    ) -> ChunkStream:
        """Open a stream: a call answered with chunks (see `ChunkStream`); `window` is
        its initial credit in bytes (0: `HelloAck.stream_window`)."""

        async def start(stream: ChunkStream) -> None:
            self._require_open()
            if window > _MAX_STREAM_WINDOW:
                raise BusError(
                    codes.BAD_REQUEST,
                    f"a window of {window} is over the limit of {_MAX_STREAM_WINDOW}",
                )
            try:
                await self._in_flight.acquire()
            except BusError:
                raise self._closed_error() from None
            if self._close_info is not None:
                self._in_flight.release()
                raise self._closed_error()
            size = window or self._stream_window
            self._next_id += 1
            call_id = self._next_id
            stream.id = call_id
            pending = _CallerStream(queue=stream._items, allowed=size)
            self._streams[call_id] = pending
            envelope = self._call_message(
                call_id, method, payload, target, timeout, content_type, credential, trace_id, size
            )
            try:
                await self._send(envelope)
            except BaseException:
                self._streams.pop(call_id, None)  # never sent: nothing to cancel
                self._in_flight.release()
                stream.id = 0
                raise
            pending.queued = True
            if pending.cancelled:
                # Cancelled while the Call waited for the send budget: the Cancel follows it.
                cancel = wire.Cancel(id=call_id, reason="cancelled by the caller")
                self._send_now(wire.Envelope(cancel=cancel))
                return
            if timeout is not None and self._streams.get(call_id) is pending:
                loop = asyncio.get_running_loop()
                pending.timer = loop.call_later(
                    timeout, self._stream_timed_out, call_id, pending, method, timeout
                )

        return ChunkStream(self, start)

    def _stream_timed_out(
        self, call_id: int, pending: _CallerStream, method: str, timeout: float
    ) -> None:
        if self._streams.get(call_id) is not pending:
            return
        self._forget_stream(call_id)
        self._send_now(wire.Envelope(cancel=wire.Cancel(id=call_id, reason="timeout")))
        message = f"{method} did not end within {int(timeout * 1000)} ms"
        pending.queue.put_nowait(("error", BusError(codes.TIMEOUT, message), 0))

    def _forget_stream(self, call_id: int) -> _CallerStream | None:
        """A stream of this caller is over: no more of it is taken, its timer stops."""
        pending = self._streams.pop(call_id, None)
        if pending is not None and pending.timer is not None:
            pending.timer.cancel()
        return pending

    async def emit(
        self, subject: str, payload: Payload | None = None, content_type: str = ""
    ) -> None:
        """Emit an event (at most once, to the subscribers connected now)."""
        self._require_open()
        if not valid_subject(subject):
            raise BusError(codes.BAD_REQUEST, f"{subject!r} is not an event subject")
        event = wire.Event(subject=subject, payload=_bytes(payload), content_type=content_type)
        await self._send(wire.Envelope(event=event))

    async def on(
        self,
        pattern: str,
        handler: Callable[[Event], Any] | None = None,
    ) -> Subscription:
        """Subscribe to the events matching `pattern`; returns once the broker took the
        subscription (`denied` if not allowed). With `handler`, it is called for each
        event in order (a coroutine is awaited); without, read the subscription with
        `async for`. `subscription.unsubscribe()` ends it."""
        self._require_open()
        if not valid_pattern(pattern):
            raise BusError(codes.BAD_REQUEST, f"{pattern!r} is not a subscription pattern")
        self._next_id += 1
        sub_id = self._next_id
        subscription = Subscription(self, sub_id, pattern)
        self._subscriptions[sub_id] = subscription
        answer: asyncio.Future[Reply] = asyncio.get_running_loop().create_future()
        self._calls[sub_id] = answer
        try:
            await self._send(wire.Envelope(subscribe=wire.Subscribe(id=sub_id, pattern=pattern)))
            await answer
        except BaseException:
            self._subscriptions.pop(sub_id, None)
            self._calls.pop(sub_id, None)
            raise
        if handler is not None:

            async def run() -> None:
                async for event in subscription:
                    try:
                        result = handler(event)
                        if inspect.isawaitable(result):
                            await result
                    except Exception:  # a handler's failure ends nothing
                        logger.exception("tilr bus: the handler of %s failed", pattern)

            subscription._task = self._spawn(run())
        return subscription

    def _unsubscribe(self, subscription: Subscription) -> None:
        if self._subscriptions.pop(subscription.id, None) is not None:
            unsubscribe = wire.Unsubscribe(id=subscription.id)
            self._send_now(wire.Envelope(unsubscribe=unsubscribe))

    async def activate(self) -> None:
        """Tell the broker this participant is now the user's active one for its
        `LAST_ACTIVE` methods (B11)."""
        self._require_open()
        await self._send(wire.Envelope(activate=wire.Activate()))

    # ------------------------------------------------------------------
    # Sending
    # ------------------------------------------------------------------

    def _encode(self, envelope: wire.Envelope) -> bytes:
        assert self._link is not None
        try:
            return self._link.encode(envelope, self._max_frame)
        except CodecError as error:
            raise _to_bus_error(error) from None

    async def _send(self, envelope: wire.Envelope) -> None:
        """Queue an envelope, waiting while the send budget is spent."""
        if self._close_info is not None:
            raise self._closed_error()
        frame = self._encode(envelope)
        while self._queued > 0 and self._queued + len(frame) > SEND_BUDGET:
            self._budget.clear()
            await self._budget.wait()
            if self._close_info is not None:
                raise self._closed_error()
        self._queued += len(frame)
        self._queue.append((frame, True))
        self._wake_writer.set()

    def _send_now(self, envelope: wire.Envelope) -> None:
        """Queue a small control envelope at once (credits, cancels, unsubscribes)."""
        if self._close_info is not None or self._link is None:
            return
        try:
            frame = self._encode(envelope)
        except BusError:
            return
        self._queue.append((frame, False))
        self._wake_writer.set()

    async def _write_loop(self) -> None:
        assert self._link is not None
        while self._close_info is None:
            if not self._queue:
                self._wake_writer.clear()
                await self._wake_writer.wait()
                continue
            frame, budgeted = self._queue.popleft()
            try:
                await self._link.write(frame)
            except Exception as error:
                self._shutdown(CloseInfo(error=(codes.UNAVAILABLE, f"write: {error}")))
                return
            if budgeted:
                self._queued -= len(frame)
                self._budget.set()

    def _credit(self, call_id: int, count: int) -> None:
        if count == 0:
            return
        pending = self._streams.get(call_id)
        if pending is None:
            return
        pending.allowed += count
        self._send_now(wire.Envelope(credit=wire.Credit(id=call_id, bytes=count)))

    def _cancel_stream(self, call_id: int) -> None:
        pending = self._forget_stream(call_id) if call_id else None
        if pending is None:
            return
        # A Call still waiting for the send budget is not queued: its Cancel follows it.
        if pending.queued:
            cancel = wire.Cancel(id=call_id, reason="cancelled by the caller")
            self._send_now(wire.Envelope(cancel=cancel))
        else:
            pending.cancelled = True

    def _release_stream(self, stream: ChunkStream) -> None:
        if stream.id:
            self._forget_stream(stream.id)
            self._in_flight.release()
            stream.id = 0

    def _closed_error(self) -> BusError:
        info = self._close_info
        if info is not None and info.error:
            return BusError(*info.error)
        if info is not None and info.code is not None:
            return BusError(codes.UNAVAILABLE, f"connection closed ({info.code} {info.reason})")
        return BusError(codes.UNAVAILABLE, "connection closed")

    def _shutdown(self, info: CloseInfo) -> None:
        if self._close_info is not None:
            return
        self._close_info = info
        if self._pending_timer is not None:
            self._pending_timer.cancel()
        error = self._closed_error()
        for answer in self._calls.values():
            if not answer.done():
                answer.set_exception(error)
                answer.exception()  # retrieved: a call that gave up is not "unretrieved"
        self._calls.clear()
        for pending in self._streams.values():
            if pending.timer is not None:
                pending.timer.cancel()
            pending.queue.put_nowait(("error", error, 0))
        self._streams.clear()
        for subscription in self._subscriptions.values():
            subscription._deliver(None)
        self._subscriptions.clear()
        for served in self._served.values():
            served.ctx.cancelled.set()
            if served.task is not None:
                served.task.cancel()
            if served.gate is not None:
                served.gate.close()
        self._served.clear()
        self._in_flight.close(error)
        self._budget.set()
        self._wake_writer.set()
        self._queue.clear()
        if self._link is not None:
            self._spawn(self._link.close())
        self._closed.set()

    # ------------------------------------------------------------------
    # Receiving
    # ------------------------------------------------------------------

    async def _read_loop(self) -> None:
        assert self._link is not None
        while self._close_info is None:
            incoming: Incoming = await self._link.recv()
            if incoming.closed is not None:
                closed = incoming.closed
                if closed.error is None and self._pending_error is not None:
                    # The ConnectionError that came first says why; the close keeps its code.
                    closed = CloseInfo(closed.code, closed.reason, self._pending_error)
                self._shutdown(closed)
                return
            # A WebSocket keeps message boundaries: a bad one is skipped.
            if incoming.envelope is not None:
                self._dispatch(incoming.envelope)

    def _dispatch(self, envelope: wire.Envelope) -> None:
        which = envelope.WhichOneof("msg")
        if which == "reply":
            self._on_reply(envelope.reply)
        elif which == "chunk":
            self._on_chunk(envelope.chunk)
        elif which == "end":
            pending = self._forget_stream(envelope.end.id)
            if pending is not None:
                pending.queue.put_nowait(("end", envelope.end.payload, 0))
        elif which == "event":
            self._on_event(envelope.event)
        elif which == "call":
            self._on_call(envelope.call)
        elif which == "cancel":
            served = self._served.pop(envelope.cancel.id, None)
            if served is not None:
                served.ctx.cancelled.set()
                if served.task is not None:
                    served.task.cancel()
                if served.gate is not None:
                    served.gate.close()
        elif which == "credit":
            served = self._served.get(envelope.credit.id)
            if served is not None:
                served.credited += envelope.credit.bytes
                if served.gate is not None:
                    served.gate.add(envelope.credit.bytes)
        elif which == "credential_update":
            self._credential = envelope.credential_update.credential
        elif which == "connection_error":
            error: Any = envelope.connection_error  # its .pyi type shadows a builtin
            # bad_frame leaves the connection open; anything else is followed by the
            # close, whose code is kept with this error (unless it does not come).
            if error.code != codes.BAD_FRAME and self._pending_error is None:
                self._pending_error = (error.code, error.message)
                self._pending_timer = asyncio.get_running_loop().call_later(
                    _CLOSE_AFTER_ERROR,
                    lambda: self._shutdown(CloseInfo(error=self._pending_error)),
                )
        # Anything else: a variant of a later version, or one a participant never gets.

    def _on_reply(self, reply: wire.Reply) -> None:
        answer = self._calls.pop(reply.id, None)
        if answer is not None:
            if answer.done():
                return
            if reply.WhichOneof("result") == "error":
                answer.set_exception(BusError(reply.error.code, reply.error.message))
            else:
                answer.set_result(Reply(reply.payload, reply.content_type))
            return
        pending = self._forget_stream(reply.id)
        if pending is not None:
            if reply.WhichOneof("result") == "error":
                error = BusError(reply.error.code, reply.error.message)
            else:
                error = BusError(
                    codes.BAD_REPLY, "a stream was answered with a reply instead of chunks"
                )
            pending.queue.put_nowait(("error", error, 0))

    def _on_chunk(self, chunk: wire.Chunk) -> None:
        pending = self._streams.get(chunk.id)
        if pending is None:
            return  # a chunk of a stream given up
        carried = len(chunk.payload)
        pending.received += carried
        if pending.received > pending.allowed:
            self._forget_stream(chunk.id)
            error = BusError(codes.BAD_REPLY, "the provider sent more than the stream's credit")
            pending.queue.put_nowait(("error", error, 0))
            self._send_now(wire.Envelope(cancel=wire.Cancel(id=chunk.id, reason="credit overrun")))
            return
        try:
            data = chunk_bytes(chunk.payload, chunk.content_encoding)
        except CodecError as error:
            self._forget_stream(chunk.id)
            pending.queue.put_nowait(("error", BusError(codes.BAD_REPLY, str(error)), 0))
            self._send_now(wire.Envelope(cancel=wire.Cancel(id=chunk.id, reason="bad chunk")))
            return
        pending.queue.put_nowait(("chunk", data, carried))

    def _on_event(self, event: wire.Event) -> None:
        delivered = Event(
            subject=event.subject,
            payload=event.payload,
            content_type=event.content_type,
            from_=getattr(event, "from"),
            scope=event.scope,
        )
        for subscription in self._subscriptions.values():
            if matches(subscription.pattern, delivered.subject):
                subscription._deliver(delivered)

    def _on_call(self, call: wire.Call) -> None:
        provided = self._provided.get(call.method)
        if provided is None:
            error = wire.Error(
                code=codes.NOT_SUPPORTED, message=f"{self.identity} does not provide {call.method}"
            )
            self._send_now(wire.Envelope(reply=wire.Reply(id=call.id, error=error)))
            return
        window = call.window or self._stream_window
        gate = _CreditGate(window) if provided.stream else None
        served = _Served(ctx=Context(self, call), gate=gate)
        self._served[call.id] = served
        served.task = self._spawn(self._serve(call.id, provided, call.payload, served, window))

    async def _serve(
        self,
        call_id: int,
        provided: _Provided,
        payload: bytes,
        served: _Served,
        window: int,
    ) -> None:
        ctx = served.ctx
        try:
            if provided.stream:
                assert served.gate is not None
                out = StreamWriter(self, call_id, served.gate, window, served)
                result = provided.handler(payload, ctx, out)
                if inspect.isawaitable(result):
                    result = await result
                answer = wire.Envelope(end=wire.End(id=call_id, payload=_bytes(result)))
            else:
                result = provided.handler(payload, ctx)
                if inspect.isawaitable(result):
                    result = await result
                if isinstance(result, Reply):
                    reply = wire.Reply(
                        id=call_id, payload=result.payload, content_type=result.content_type
                    )
                else:
                    reply = wire.Reply(id=call_id, payload=_bytes(result))
                answer = wire.Envelope(reply=reply)
        except asyncio.CancelledError:
            if ctx.cancelled.is_set():
                return  # the caller's Cancel, or the connection's end: no reply
            # Not the bus's cancel (the handler awaited something cancelled, or the task
            # itself is being cancelled): the caller still gets an answer.
            if self._served.pop(call_id, None) is not None:
                error = wire.Error(code=codes.CANCELLED, message="the handler was cancelled")
                self._send_now(wire.Envelope(reply=wire.Reply(id=call_id, error=error)))
            task = asyncio.current_task()
            if task is not None and task.cancelling() > 0:
                raise
            return
        except Exception as error:
            if ctx.cancelled.is_set():
                return
            answer = wire.Envelope(reply=wire.Reply(id=call_id, error=_error_of(error)))
        # A call cancelled meanwhile gets no answer.
        if self._served.pop(call_id, None) is None:
            return
        try:
            await self._send(answer)
        except BusError as error:
            if error.code == codes.TOO_LARGE:
                reply = wire.Reply(
                    id=call_id, error=wire.Error(code=error.code, message=error.message)
                )
                try:
                    await self._send(wire.Envelope(reply=reply))
                except BusError:
                    pass
