"""The tilr bus (`tilr.bus.v1`) for Python: call methods, provide them, emit and
receive events, read and serve streams, over an inherited descriptor, an instance
socket or a WebSocket. The generated messages are in `tilr_bus.proto`."""

from tilr_bus._bus import (
    CONTENT_TYPE_JSON,
    CONTENT_TYPE_PROTOBUF,
    PROTOCOL_VERSION,
    Bus,
    BusError,
    ChunkStream,
    Context,
    Event,
    Reply,
    StreamWriter,
    Subscription,
    codes,
    credential_bytes,
    env,
)
from tilr_bus._codec import ENCODING_ZSTD, MAX_FRAME
from tilr_bus._names import matches, valid_method, valid_pattern, valid_subject
from tilr_bus._transport import CloseInfo
from tilr_bus.pane import PANE_OUTPUT_WINDOW, PaneOutput, Panes

__all__ = [
    "CONTENT_TYPE_JSON",
    "CONTENT_TYPE_PROTOBUF",
    "ENCODING_ZSTD",
    "MAX_FRAME",
    "PANE_OUTPUT_WINDOW",
    "PROTOCOL_VERSION",
    "Bus",
    "BusError",
    "ChunkStream",
    "CloseInfo",
    "Context",
    "Event",
    "PaneOutput",
    "Panes",
    "Reply",
    "StreamWriter",
    "Subscription",
    "codes",
    "credential_bytes",
    "env",
    "matches",
    "valid_method",
    "valid_pattern",
    "valid_subject",
]
