from google.protobuf.internal import containers as _containers
from google.protobuf import descriptor as _descriptor
from google.protobuf import message as _message
from collections.abc import Iterable as _Iterable
from typing import ClassVar as _ClassVar, Optional as _Optional

DESCRIPTOR: _descriptor.FileDescriptor

class InfoRequest(_message.Message):
    __slots__ = ()
    def __init__(self) -> None: ...

class InfoReply(_message.Message):
    __slots__ = ("server_version", "features", "clipboard_max_bytes", "server_id", "fs_max_bytes", "update_channel")
    SERVER_VERSION_FIELD_NUMBER: _ClassVar[int]
    FEATURES_FIELD_NUMBER: _ClassVar[int]
    CLIPBOARD_MAX_BYTES_FIELD_NUMBER: _ClassVar[int]
    SERVER_ID_FIELD_NUMBER: _ClassVar[int]
    FS_MAX_BYTES_FIELD_NUMBER: _ClassVar[int]
    UPDATE_CHANNEL_FIELD_NUMBER: _ClassVar[int]
    server_version: str
    features: _containers.RepeatedScalarFieldContainer[str]
    clipboard_max_bytes: int
    server_id: str
    fs_max_bytes: int
    update_channel: str
    def __init__(self, server_version: _Optional[str] = ..., features: _Optional[_Iterable[str]] = ..., clipboard_max_bytes: _Optional[int] = ..., server_id: _Optional[str] = ..., fs_max_bytes: _Optional[int] = ..., update_channel: _Optional[str] = ...) -> None: ...

class ReloadRequest(_message.Message):
    __slots__ = ()
    def __init__(self) -> None: ...

class ReloadReply(_message.Message):
    __slots__ = ()
    def __init__(self) -> None: ...

class UpdateRequest(_message.Message):
    __slots__ = ()
    def __init__(self) -> None: ...

class CommandRequest(_message.Message):
    __slots__ = ("argv", "cwd", "env", "stdin", "timeout_ms")
    ARGV_FIELD_NUMBER: _ClassVar[int]
    CWD_FIELD_NUMBER: _ClassVar[int]
    ENV_FIELD_NUMBER: _ClassVar[int]
    STDIN_FIELD_NUMBER: _ClassVar[int]
    TIMEOUT_MS_FIELD_NUMBER: _ClassVar[int]
    argv: _containers.RepeatedScalarFieldContainer[str]
    cwd: str
    env: _containers.RepeatedScalarFieldContainer[str]
    stdin: bool
    timeout_ms: int
    def __init__(self, argv: _Optional[_Iterable[str]] = ..., cwd: _Optional[str] = ..., env: _Optional[_Iterable[str]] = ..., stdin: _Optional[bool] = ..., timeout_ms: _Optional[int] = ...) -> None: ...
