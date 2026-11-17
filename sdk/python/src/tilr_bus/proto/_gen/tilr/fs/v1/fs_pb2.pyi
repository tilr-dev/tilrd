from google.protobuf.internal import containers as _containers
from google.protobuf.internal import enum_type_wrapper as _enum_type_wrapper
from google.protobuf import descriptor as _descriptor
from google.protobuf import message as _message
from collections.abc import Iterable as _Iterable, Mapping as _Mapping
from typing import ClassVar as _ClassVar, Optional as _Optional, Union as _Union

DESCRIPTOR: _descriptor.FileDescriptor

class FileKind(int, metaclass=_enum_type_wrapper.EnumTypeWrapper):
    __slots__ = ()
    FILE_KIND_FILE: _ClassVar[FileKind]
    FILE_KIND_DIR: _ClassVar[FileKind]
    FILE_KIND_LINK: _ClassVar[FileKind]
    FILE_KIND_OTHER: _ClassVar[FileKind]

class FsChange(int, metaclass=_enum_type_wrapper.EnumTypeWrapper):
    __slots__ = ()
    FS_CHANGE_CREATE: _ClassVar[FsChange]
    FS_CHANGE_MODIFY: _ClassVar[FsChange]
    FS_CHANGE_REMOVE: _ClassVar[FsChange]
FILE_KIND_FILE: FileKind
FILE_KIND_DIR: FileKind
FILE_KIND_LINK: FileKind
FILE_KIND_OTHER: FileKind
FS_CHANGE_CREATE: FsChange
FS_CHANGE_MODIFY: FsChange
FS_CHANGE_REMOVE: FsChange

class ListRequest(_message.Message):
    __slots__ = ("path", "path_bytes")
    PATH_FIELD_NUMBER: _ClassVar[int]
    PATH_BYTES_FIELD_NUMBER: _ClassVar[int]
    path: str
    path_bytes: bytes
    def __init__(self, path: _Optional[str] = ..., path_bytes: _Optional[bytes] = ...) -> None: ...

class StatRequest(_message.Message):
    __slots__ = ("path", "path_bytes")
    PATH_FIELD_NUMBER: _ClassVar[int]
    PATH_BYTES_FIELD_NUMBER: _ClassVar[int]
    path: str
    path_bytes: bytes
    def __init__(self, path: _Optional[str] = ..., path_bytes: _Optional[bytes] = ...) -> None: ...

class ReadRequest(_message.Message):
    __slots__ = ("path", "offset", "length", "path_bytes")
    PATH_FIELD_NUMBER: _ClassVar[int]
    OFFSET_FIELD_NUMBER: _ClassVar[int]
    LENGTH_FIELD_NUMBER: _ClassVar[int]
    PATH_BYTES_FIELD_NUMBER: _ClassVar[int]
    path: str
    offset: int
    length: int
    path_bytes: bytes
    def __init__(self, path: _Optional[str] = ..., offset: _Optional[int] = ..., length: _Optional[int] = ..., path_bytes: _Optional[bytes] = ...) -> None: ...

class WriteRequest(_message.Message):
    __slots__ = ("transfer_id", "path", "total_size", "create", "truncate", "data", "last", "path_bytes")
    TRANSFER_ID_FIELD_NUMBER: _ClassVar[int]
    PATH_FIELD_NUMBER: _ClassVar[int]
    TOTAL_SIZE_FIELD_NUMBER: _ClassVar[int]
    CREATE_FIELD_NUMBER: _ClassVar[int]
    TRUNCATE_FIELD_NUMBER: _ClassVar[int]
    DATA_FIELD_NUMBER: _ClassVar[int]
    LAST_FIELD_NUMBER: _ClassVar[int]
    PATH_BYTES_FIELD_NUMBER: _ClassVar[int]
    transfer_id: int
    path: str
    total_size: int
    create: bool
    truncate: bool
    data: bytes
    last: bool
    path_bytes: bytes
    def __init__(self, transfer_id: _Optional[int] = ..., path: _Optional[str] = ..., total_size: _Optional[int] = ..., create: _Optional[bool] = ..., truncate: _Optional[bool] = ..., data: _Optional[bytes] = ..., last: _Optional[bool] = ..., path_bytes: _Optional[bytes] = ...) -> None: ...

class MkdirRequest(_message.Message):
    __slots__ = ("path", "path_bytes")
    PATH_FIELD_NUMBER: _ClassVar[int]
    PATH_BYTES_FIELD_NUMBER: _ClassVar[int]
    path: str
    path_bytes: bytes
    def __init__(self, path: _Optional[str] = ..., path_bytes: _Optional[bytes] = ...) -> None: ...

class RenameRequest(_message.Message):
    __slots__ = ("to", "from_bytes", "to_bytes")
    FROM_FIELD_NUMBER: _ClassVar[int]
    TO_FIELD_NUMBER: _ClassVar[int]
    FROM_BYTES_FIELD_NUMBER: _ClassVar[int]
    TO_BYTES_FIELD_NUMBER: _ClassVar[int]
    to: str
    from_bytes: bytes
    to_bytes: bytes
    def __init__(self, to: _Optional[str] = ..., from_bytes: _Optional[bytes] = ..., to_bytes: _Optional[bytes] = ..., **kwargs) -> None: ...

class DeleteRequest(_message.Message):
    __slots__ = ("path", "recursive", "path_bytes")
    PATH_FIELD_NUMBER: _ClassVar[int]
    RECURSIVE_FIELD_NUMBER: _ClassVar[int]
    PATH_BYTES_FIELD_NUMBER: _ClassVar[int]
    path: str
    recursive: bool
    path_bytes: bytes
    def __init__(self, path: _Optional[str] = ..., recursive: _Optional[bool] = ..., path_bytes: _Optional[bytes] = ...) -> None: ...

class WatchRequest(_message.Message):
    __slots__ = ("path", "path_bytes")
    PATH_FIELD_NUMBER: _ClassVar[int]
    PATH_BYTES_FIELD_NUMBER: _ClassVar[int]
    path: str
    path_bytes: bytes
    def __init__(self, path: _Optional[str] = ..., path_bytes: _Optional[bytes] = ...) -> None: ...

class WatchReply(_message.Message):
    __slots__ = ("watch_id",)
    WATCH_ID_FIELD_NUMBER: _ClassVar[int]
    watch_id: int
    def __init__(self, watch_id: _Optional[int] = ...) -> None: ...

class UnwatchRequest(_message.Message):
    __slots__ = ("watch_id",)
    WATCH_ID_FIELD_NUMBER: _ClassVar[int]
    watch_id: int
    def __init__(self, watch_id: _Optional[int] = ...) -> None: ...

class FsAck(_message.Message):
    __slots__ = ()
    def __init__(self) -> None: ...

class FsEntry(_message.Message):
    __slots__ = ("name", "kind", "size", "mtime_ms", "mode", "target", "name_bytes")
    NAME_FIELD_NUMBER: _ClassVar[int]
    KIND_FIELD_NUMBER: _ClassVar[int]
    SIZE_FIELD_NUMBER: _ClassVar[int]
    MTIME_MS_FIELD_NUMBER: _ClassVar[int]
    MODE_FIELD_NUMBER: _ClassVar[int]
    TARGET_FIELD_NUMBER: _ClassVar[int]
    NAME_BYTES_FIELD_NUMBER: _ClassVar[int]
    name: str
    kind: FileKind
    size: int
    mtime_ms: int
    mode: int
    target: FileKind
    name_bytes: bytes
    def __init__(self, name: _Optional[str] = ..., kind: _Optional[_Union[FileKind, str]] = ..., size: _Optional[int] = ..., mtime_ms: _Optional[int] = ..., mode: _Optional[int] = ..., target: _Optional[_Union[FileKind, str]] = ..., name_bytes: _Optional[bytes] = ...) -> None: ...

class FsEntries(_message.Message):
    __slots__ = ("path", "entries", "path_bytes")
    PATH_FIELD_NUMBER: _ClassVar[int]
    ENTRIES_FIELD_NUMBER: _ClassVar[int]
    PATH_BYTES_FIELD_NUMBER: _ClassVar[int]
    path: str
    entries: _containers.RepeatedCompositeFieldContainer[FsEntry]
    path_bytes: bytes
    def __init__(self, path: _Optional[str] = ..., entries: _Optional[_Iterable[_Union[FsEntry, _Mapping]]] = ..., path_bytes: _Optional[bytes] = ...) -> None: ...

class FsInfo(_message.Message):
    __slots__ = ("path", "entry", "path_bytes")
    PATH_FIELD_NUMBER: _ClassVar[int]
    ENTRY_FIELD_NUMBER: _ClassVar[int]
    PATH_BYTES_FIELD_NUMBER: _ClassVar[int]
    path: str
    entry: FsEntry
    path_bytes: bytes
    def __init__(self, path: _Optional[str] = ..., entry: _Optional[_Union[FsEntry, _Mapping]] = ..., path_bytes: _Optional[bytes] = ...) -> None: ...

class FsData(_message.Message):
    __slots__ = ("total_size", "data", "file_size")
    TOTAL_SIZE_FIELD_NUMBER: _ClassVar[int]
    DATA_FIELD_NUMBER: _ClassVar[int]
    FILE_SIZE_FIELD_NUMBER: _ClassVar[int]
    total_size: int
    data: bytes
    file_size: int
    def __init__(self, total_size: _Optional[int] = ..., data: _Optional[bytes] = ..., file_size: _Optional[int] = ...) -> None: ...

class FsChanged(_message.Message):
    __slots__ = ("watch", "path", "kind", "path_bytes", "watch_id")
    WATCH_FIELD_NUMBER: _ClassVar[int]
    PATH_FIELD_NUMBER: _ClassVar[int]
    KIND_FIELD_NUMBER: _ClassVar[int]
    PATH_BYTES_FIELD_NUMBER: _ClassVar[int]
    WATCH_ID_FIELD_NUMBER: _ClassVar[int]
    watch: str
    path: str
    kind: FsChange
    path_bytes: bytes
    watch_id: int
    def __init__(self, watch: _Optional[str] = ..., path: _Optional[str] = ..., kind: _Optional[_Union[FsChange, str]] = ..., path_bytes: _Optional[bytes] = ..., watch_id: _Optional[int] = ...) -> None: ...
