from google.protobuf import descriptor as _descriptor
from google.protobuf import message as _message
from typing import ClassVar as _ClassVar, Optional as _Optional

DESCRIPTOR: _descriptor.FileDescriptor

class AgentRequest(_message.Message):
    __slots__ = ("conn_id", "pane_id", "message")
    CONN_ID_FIELD_NUMBER: _ClassVar[int]
    PANE_ID_FIELD_NUMBER: _ClassVar[int]
    MESSAGE_FIELD_NUMBER: _ClassVar[int]
    conn_id: int
    pane_id: str
    message: bytes
    def __init__(self, conn_id: _Optional[int] = ..., pane_id: _Optional[str] = ..., message: _Optional[bytes] = ...) -> None: ...

class AgentReply(_message.Message):
    __slots__ = ("message",)
    MESSAGE_FIELD_NUMBER: _ClassVar[int]
    message: bytes
    def __init__(self, message: _Optional[bytes] = ...) -> None: ...

class CloseRequest(_message.Message):
    __slots__ = ("conn_id",)
    CONN_ID_FIELD_NUMBER: _ClassVar[int]
    conn_id: int
    def __init__(self, conn_id: _Optional[int] = ...) -> None: ...

class CloseReply(_message.Message):
    __slots__ = ()
    def __init__(self) -> None: ...
