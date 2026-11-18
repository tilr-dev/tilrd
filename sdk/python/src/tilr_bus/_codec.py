"""Framing of envelopes (design B3; `crates/tilr-bus/src/codec.rs` is the reference).

- WebSocket: one binary message is a flag byte (0 plain, 1 zstd) and one envelope;
  envelopes of 256 bytes or more are compressed unless they carry a chunk its
  producer compressed, or compressing does not make them smaller.
- Stream sockets (an instance's descriptor 3, its instance socket): a u32 big-endian
  length and one envelope, never compressed.

An envelope is at most MAX_FRAME bytes after decompression, both ways.
"""

from __future__ import annotations

import struct

import zstandard

from tilr_bus.proto import bus as wire

MAX_FRAME = 16 << 20
FLAG_PLAIN = 0
FLAG_ZSTD = 1
COMPRESS_THRESHOLD = 256
ENCODING_ZSTD = "zstd"
LENGTH_PREFIX = 4

_COMPRESSOR = zstandard.ZstdCompressor(level=1)


class CodecError(Exception):
    """An envelope could not be framed or read."""

    def __init__(self, message: str, *, too_large: bool = False) -> None:
        super().__init__(message)
        self.too_large = too_large


def _check(envelope: wire.Envelope, max_frame: int) -> bytes:
    body = envelope.SerializeToString()
    if len(body) > max_frame:
        raise CodecError(
            f"an envelope of {len(body)} bytes is over the limit of {max_frame}", too_large=True
        )
    return body


def _carries_compressed_payload(envelope: wire.Envelope) -> bool:
    return (
        envelope.WhichOneof("msg") == "chunk" and envelope.chunk.content_encoding == ENCODING_ZSTD
    )


def encode_ws(envelope: wire.Envelope, max_frame: int = MAX_FRAME) -> bytes:
    """Frame an envelope as one binary WebSocket message."""
    body = _check(envelope, max_frame)
    if len(body) >= COMPRESS_THRESHOLD and not _carries_compressed_payload(envelope):
        compressed = _COMPRESSOR.compress(body)
        if len(compressed) <= len(body):
            return bytes([FLAG_ZSTD]) + compressed
    return bytes([FLAG_PLAIN]) + body


def _decompress(data: bytes) -> bytes:
    reader = zstandard.ZstdDecompressor().stream_reader(data)
    parts = []
    size = 0
    while chunk := reader.read(1 << 20):
        parts.append(chunk)
        size += len(chunk)
        if size > MAX_FRAME:
            raise CodecError(
                f"envelope of more than {MAX_FRAME} bytes is over the limit", too_large=True
            )
    return b"".join(parts)


def decode_ws(message: bytes) -> wire.Envelope:
    """Read one binary WebSocket message."""
    if not message:
        raise CodecError("empty packet")
    flag, body = message[0], message[1:]
    if flag == FLAG_ZSTD:
        try:
            body = _decompress(body)
        except zstandard.ZstdError as err:
            raise CodecError(f"zstd: {err}") from err
    elif flag != FLAG_PLAIN:
        raise CodecError(f"unknown packet flag {flag}")
    if len(body) > MAX_FRAME:
        raise CodecError(
            f"envelope of {len(body)} bytes is over the limit of {MAX_FRAME}", too_large=True
        )
    envelope = wire.Envelope()
    try:
        envelope.ParseFromString(body)
    except Exception as err:  # protobuf's DecodeError
        raise CodecError(f"protobuf: {err}") from err
    return envelope


def encode_stream(envelope: wire.Envelope, max_frame: int = MAX_FRAME) -> bytes:
    """Frame an envelope for a stream socket: u32 big-endian length, then the envelope."""
    body = _check(envelope, max_frame)
    return struct.pack(">I", len(body)) + body


def decode_stream_body(body: bytes) -> wire.Envelope:
    envelope = wire.Envelope()
    try:
        envelope.ParseFromString(body)
    except Exception as err:
        raise CodecError(f"protobuf: {err}") from err
    return envelope


def chunk_bytes(payload: bytes, content_encoding: str) -> bytes:
    """A chunk's payload, decompressed when its producer compressed it."""
    if not content_encoding:
        return payload
    if content_encoding != ENCODING_ZSTD:
        raise CodecError(f"chunk has an unknown content encoding {content_encoding!r}")
    try:
        return _decompress(payload)
    except zstandard.ZstdError as err:
        raise CodecError(f"chunk: zstd: {err}") from err
