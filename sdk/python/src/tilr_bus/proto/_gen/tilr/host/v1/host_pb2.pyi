from google.protobuf.internal import containers as _containers
from google.protobuf import descriptor as _descriptor
from google.protobuf import message as _message
from collections.abc import Iterable as _Iterable
from typing import ClassVar as _ClassVar, Optional as _Optional

DESCRIPTOR: _descriptor.FileDescriptor

class OpenRequest(_message.Message):
    __slots__ = ("pane_id", "name", "kind", "path", "args")
    PANE_ID_FIELD_NUMBER: _ClassVar[int]
    NAME_FIELD_NUMBER: _ClassVar[int]
    KIND_FIELD_NUMBER: _ClassVar[int]
    PATH_FIELD_NUMBER: _ClassVar[int]
    ARGS_FIELD_NUMBER: _ClassVar[int]
    pane_id: str
    name: str
    kind: str
    path: str
    args: _containers.RepeatedScalarFieldContainer[str]
    def __init__(self, pane_id: _Optional[str] = ..., name: _Optional[str] = ..., kind: _Optional[str] = ..., path: _Optional[str] = ..., args: _Optional[_Iterable[str]] = ...) -> None: ...

class OpenReply(_message.Message):
    __slots__ = ()
    def __init__(self) -> None: ...
