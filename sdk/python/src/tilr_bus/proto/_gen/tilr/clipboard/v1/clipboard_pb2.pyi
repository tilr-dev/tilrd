from google.protobuf import descriptor as _descriptor
from google.protobuf import message as _message
from typing import ClassVar as _ClassVar, Optional as _Optional

DESCRIPTOR: _descriptor.FileDescriptor

class PutRequest(_message.Message):
    __slots__ = ("transfer_id", "mime", "total_size", "data", "last")
    TRANSFER_ID_FIELD_NUMBER: _ClassVar[int]
    MIME_FIELD_NUMBER: _ClassVar[int]
    TOTAL_SIZE_FIELD_NUMBER: _ClassVar[int]
    DATA_FIELD_NUMBER: _ClassVar[int]
    LAST_FIELD_NUMBER: _ClassVar[int]
    transfer_id: int
    mime: str
    total_size: int
    data: bytes
    last: bool
    def __init__(self, transfer_id: _Optional[int] = ..., mime: _Optional[str] = ..., total_size: _Optional[int] = ..., data: _Optional[bytes] = ..., last: _Optional[bool] = ...) -> None: ...

class PutReply(_message.Message):
    __slots__ = ("transfer_id",)
    TRANSFER_ID_FIELD_NUMBER: _ClassVar[int]
    transfer_id: int
    def __init__(self, transfer_id: _Optional[int] = ...) -> None: ...

class GetRequest(_message.Message):
    __slots__ = ("transfer_id",)
    TRANSFER_ID_FIELD_NUMBER: _ClassVar[int]
    transfer_id: int
    def __init__(self, transfer_id: _Optional[int] = ...) -> None: ...

class ClipboardData(_message.Message):
    __slots__ = ("transfer_id", "pane_id", "mime", "total_size", "data")
    TRANSFER_ID_FIELD_NUMBER: _ClassVar[int]
    PANE_ID_FIELD_NUMBER: _ClassVar[int]
    MIME_FIELD_NUMBER: _ClassVar[int]
    TOTAL_SIZE_FIELD_NUMBER: _ClassVar[int]
    DATA_FIELD_NUMBER: _ClassVar[int]
    transfer_id: int
    pane_id: str
    mime: str
    total_size: int
    data: bytes
    def __init__(self, transfer_id: _Optional[int] = ..., pane_id: _Optional[str] = ..., mime: _Optional[str] = ..., total_size: _Optional[int] = ..., data: _Optional[bytes] = ...) -> None: ...
