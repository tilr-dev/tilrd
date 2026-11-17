from google.protobuf.internal import containers as _containers
from google.protobuf import descriptor as _descriptor
from google.protobuf import message as _message
from collections.abc import Iterable as _Iterable, Mapping as _Mapping
from typing import ClassVar as _ClassVar, Optional as _Optional, Union as _Union

DESCRIPTOR: _descriptor.FileDescriptor

class RunRequest(_message.Message):
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

class RunChunk(_message.Message):
    __slots__ = ("started", "output")
    STARTED_FIELD_NUMBER: _ClassVar[int]
    OUTPUT_FIELD_NUMBER: _ClassVar[int]
    started: ExecStarted
    output: ExecOutput
    def __init__(self, started: _Optional[_Union[ExecStarted, _Mapping]] = ..., output: _Optional[_Union[ExecOutput, _Mapping]] = ...) -> None: ...

class ExecStarted(_message.Message):
    __slots__ = ("exec_id", "pid")
    EXEC_ID_FIELD_NUMBER: _ClassVar[int]
    PID_FIELD_NUMBER: _ClassVar[int]
    exec_id: int
    pid: int
    def __init__(self, exec_id: _Optional[int] = ..., pid: _Optional[int] = ...) -> None: ...

class ExecOutput(_message.Message):
    __slots__ = ("exec_id", "stream", "data")
    EXEC_ID_FIELD_NUMBER: _ClassVar[int]
    STREAM_FIELD_NUMBER: _ClassVar[int]
    DATA_FIELD_NUMBER: _ClassVar[int]
    exec_id: int
    stream: int
    data: bytes
    def __init__(self, exec_id: _Optional[int] = ..., stream: _Optional[int] = ..., data: _Optional[bytes] = ...) -> None: ...

class ExecExit(_message.Message):
    __slots__ = ("exec_id", "code", "signal", "timed_out", "cancelled", "reason")
    EXEC_ID_FIELD_NUMBER: _ClassVar[int]
    CODE_FIELD_NUMBER: _ClassVar[int]
    SIGNAL_FIELD_NUMBER: _ClassVar[int]
    TIMED_OUT_FIELD_NUMBER: _ClassVar[int]
    CANCELLED_FIELD_NUMBER: _ClassVar[int]
    REASON_FIELD_NUMBER: _ClassVar[int]
    exec_id: int
    code: int
    signal: int
    timed_out: bool
    cancelled: bool
    reason: str
    def __init__(self, exec_id: _Optional[int] = ..., code: _Optional[int] = ..., signal: _Optional[int] = ..., timed_out: _Optional[bool] = ..., cancelled: _Optional[bool] = ..., reason: _Optional[str] = ...) -> None: ...

class InputRequest(_message.Message):
    __slots__ = ("exec_id", "data", "eof")
    EXEC_ID_FIELD_NUMBER: _ClassVar[int]
    DATA_FIELD_NUMBER: _ClassVar[int]
    EOF_FIELD_NUMBER: _ClassVar[int]
    exec_id: int
    data: bytes
    eof: bool
    def __init__(self, exec_id: _Optional[int] = ..., data: _Optional[bytes] = ..., eof: _Optional[bool] = ...) -> None: ...

class InputReply(_message.Message):
    __slots__ = ("exec_id", "taken")
    EXEC_ID_FIELD_NUMBER: _ClassVar[int]
    TAKEN_FIELD_NUMBER: _ClassVar[int]
    exec_id: int
    taken: int
    def __init__(self, exec_id: _Optional[int] = ..., taken: _Optional[int] = ...) -> None: ...

class SignalRequest(_message.Message):
    __slots__ = ("exec_id", "signal")
    EXEC_ID_FIELD_NUMBER: _ClassVar[int]
    SIGNAL_FIELD_NUMBER: _ClassVar[int]
    exec_id: int
    signal: int
    def __init__(self, exec_id: _Optional[int] = ..., signal: _Optional[int] = ...) -> None: ...

class SignalReply(_message.Message):
    __slots__ = ()
    def __init__(self) -> None: ...
