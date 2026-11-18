"""The transports to the broker (design B3, B10, K1): an inherited descriptor or an
instance socket (stream framing), a WebSocket on the TCP port or the server's Unix
socket (`websockets`)."""

from __future__ import annotations

import asyncio
import socket
import struct
from dataclasses import dataclass

import websockets
from websockets.asyncio.client import ClientConnection
from websockets.exceptions import ConnectionClosed, InvalidStatus

from tilr_bus._codec import (
    LENGTH_PREFIX,
    MAX_FRAME,
    CodecError,
    decode_stream_body,
    decode_ws,
    encode_stream,
    encode_ws,
)
from tilr_bus.proto import bus as wire

SUBPROTOCOL = "tilr.bus.v1"
_WS_MAX_SIZE = MAX_FRAME + (1 << 20)


@dataclass(frozen=True)
class CloseInfo:
    """How the connection ended."""

    #: The WebSocket close code (4000-4003, 1002, ...); None on stream sockets.
    code: int | None = None
    reason: str = ""
    #: The `ConnectionError` the broker sent, or what broke the connection: (code, message).
    error: tuple[str, str] | None = None


@dataclass(frozen=True)
class Incoming:
    """What arrived: an envelope, a message that did not decode, or the end."""

    envelope: wire.Envelope | None = None
    bad: str | None = None
    closed: CloseInfo | None = None


class TransportError(Exception):
    def __init__(self, code: str, message: str) -> None:
        super().__init__(f"{code}: {message}")
        self.code = code
        self.message = message


class Link:
    """One open connection to the broker, framing included."""

    framing: str

    def encode(self, envelope: wire.Envelope, max_frame: int) -> bytes:
        raise NotImplementedError

    async def write(self, frame: bytes) -> None:
        """Write a frame; returns when the socket takes more (drain)."""
        raise NotImplementedError

    async def recv(self) -> Incoming:
        raise NotImplementedError

    async def close(self) -> None:
        raise NotImplementedError


class StreamLink(Link):
    framing = "stream"

    def __init__(self, reader: asyncio.StreamReader, writer: asyncio.StreamWriter) -> None:
        self.reader = reader
        self.writer = writer

    def encode(self, envelope: wire.Envelope, max_frame: int) -> bytes:
        return encode_stream(envelope, max_frame)

    async def write(self, frame: bytes) -> None:
        self.writer.write(frame)
        await self.writer.drain()

    async def recv(self) -> Incoming:
        try:
            prefix = await self.reader.readexactly(LENGTH_PREFIX)
            (length,) = struct.unpack(">I", prefix)
            if length > MAX_FRAME:
                # The stream's boundaries are lost.
                message = f"bad packet: envelope of {length} bytes is over the limit"
                return Incoming(closed=CloseInfo(error=("bad_frame", message)))
            body = await self.reader.readexactly(length)
        except asyncio.IncompleteReadError:
            return Incoming(closed=CloseInfo())
        except OSError as err:
            return Incoming(closed=CloseInfo(error=("unavailable", f"read: {err}")))
        try:
            return Incoming(envelope=decode_stream_body(body))
        except CodecError as err:
            return Incoming(closed=CloseInfo(error=("bad_frame", f"bad packet: {err}")))

    async def close(self) -> None:
        try:
            self.writer.close()
            await self.writer.wait_closed()
        except OSError:
            pass


class WsLink(Link):
    framing = "ws"

    def __init__(self, ws: ClientConnection) -> None:
        self.ws = ws

    def encode(self, envelope: wire.Envelope, max_frame: int) -> bytes:
        return encode_ws(envelope, max_frame)

    async def write(self, frame: bytes) -> None:
        # `send` waits while the write buffer is over its limit (K3).
        await self.ws.send(frame)

    async def recv(self) -> Incoming:
        try:
            message = await self.ws.recv()
        except ConnectionClosed as closed:
            frame = closed.rcvd
            if frame is None:
                return Incoming(closed=CloseInfo())
            return Incoming(closed=CloseInfo(code=frame.code, reason=frame.reason))
        if isinstance(message, str):
            return Incoming(bad="bad packet: a text message")
        try:
            return Incoming(envelope=decode_ws(message))
        except CodecError as err:
            return Incoming(bad=f"bad packet: {err}")

    async def close(self) -> None:
        await self.ws.close()


async def open_fd(fd: int) -> Link:
    try:
        sock = socket.socket(fileno=fd)
        sock.setblocking(False)
        reader, writer = await asyncio.open_unix_connection(sock=sock)
    except OSError as err:
        raise TransportError("unavailable", f"bus descriptor {fd}: {err}") from err
    return StreamLink(reader, writer)


async def open_socket(path: str) -> Link:
    try:
        reader, writer = await asyncio.open_unix_connection(path)
    except OSError as err:
        raise TransportError("unavailable", f"connect to {path}: {err}") from err
    return StreamLink(reader, writer)


def _ws_options(origin: str | None) -> dict:
    options: dict = {
        "subprotocols": [SUBPROTOCOL],
        "max_size": _WS_MAX_SIZE,
        "compression": None,
        # The server pings; its pings are answered by websockets.
        "ping_interval": None,
        "open_timeout": None,
    }
    if origin:
        options["origin"] = origin
    return options


def _handshake_error(where: str, err: Exception) -> TransportError:
    if isinstance(err, InvalidStatus) and err.response.status_code in (401, 403):
        return TransportError("denied", f"{where}: HTTP {err.response.status_code}")
    return TransportError("unavailable", f"{where}: {err}")


async def open_ws(url: str, origin: str | None = None) -> Link:
    try:
        ws = await websockets.connect(url, **_ws_options(origin))
    except (OSError, websockets.exceptions.WebSocketException) as err:
        raise _handshake_error(url, err) from err
    if ws.subprotocol != SUBPROTOCOL:
        await ws.close(1002)
        raise TransportError("unavailable", f"{url} does not speak {SUBPROTOCOL}")
    return WsLink(ws)


async def open_unix_ws(path: str, origin: str | None = None) -> Link:
    try:
        ws = await websockets.unix_connect(path, "ws://localhost/", **_ws_options(origin))
    except (OSError, websockets.exceptions.WebSocketException) as err:
        raise _handshake_error(path, err) from err
    if ws.subprotocol != SUBPROTOCOL:
        await ws.close(1002)
        raise TransportError("unavailable", f"{path} does not speak {SUBPROTOCOL}")
    return WsLink(ws)
