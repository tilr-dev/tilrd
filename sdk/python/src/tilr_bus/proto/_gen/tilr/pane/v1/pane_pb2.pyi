from tilr_bus.proto._gen.tilr.pane.v1 import frame_pb2 as _frame_pb2
from google.protobuf.internal import containers as _containers
from google.protobuf.internal import enum_type_wrapper as _enum_type_wrapper
from google.protobuf import descriptor as _descriptor
from google.protobuf import message as _message
from collections.abc import Iterable as _Iterable, Mapping as _Mapping
from typing import ClassVar as _ClassVar, Optional as _Optional, Union as _Union

DESCRIPTOR: _descriptor.FileDescriptor

class PaneActivityKind(int, metaclass=_enum_type_wrapper.EnumTypeWrapper):
    __slots__ = ()
    PANE_ACTIVITY_KIND_UNSPECIFIED: _ClassVar[PaneActivityKind]
    PANE_ACTIVITY_KIND_BELL: _ClassVar[PaneActivityKind]
    PANE_ACTIVITY_KIND_COMMAND_FINISHED: _ClassVar[PaneActivityKind]
PANE_ACTIVITY_KIND_UNSPECIFIED: PaneActivityKind
PANE_ACTIVITY_KIND_BELL: PaneActivityKind
PANE_ACTIVITY_KIND_COMMAND_FINISHED: PaneActivityKind

class PaneInfo(_message.Message):
    __slots__ = ("pane_id", "cols", "rows", "command", "title", "cwd", "viewers", "created_at_ms", "pid", "viewed_by", "used_at_ms", "created_by", "sandboxed", "profile")
    PANE_ID_FIELD_NUMBER: _ClassVar[int]
    COLS_FIELD_NUMBER: _ClassVar[int]
    ROWS_FIELD_NUMBER: _ClassVar[int]
    COMMAND_FIELD_NUMBER: _ClassVar[int]
    TITLE_FIELD_NUMBER: _ClassVar[int]
    CWD_FIELD_NUMBER: _ClassVar[int]
    VIEWERS_FIELD_NUMBER: _ClassVar[int]
    CREATED_AT_MS_FIELD_NUMBER: _ClassVar[int]
    PID_FIELD_NUMBER: _ClassVar[int]
    VIEWED_BY_FIELD_NUMBER: _ClassVar[int]
    USED_AT_MS_FIELD_NUMBER: _ClassVar[int]
    CREATED_BY_FIELD_NUMBER: _ClassVar[int]
    SANDBOXED_FIELD_NUMBER: _ClassVar[int]
    PROFILE_FIELD_NUMBER: _ClassVar[int]
    pane_id: str
    cols: int
    rows: int
    command: _containers.RepeatedScalarFieldContainer[str]
    title: str
    cwd: str
    viewers: int
    created_at_ms: int
    pid: int
    viewed_by: _containers.RepeatedCompositeFieldContainer[Viewer]
    used_at_ms: int
    created_by: str
    sandboxed: bool
    profile: str
    def __init__(self, pane_id: _Optional[str] = ..., cols: _Optional[int] = ..., rows: _Optional[int] = ..., command: _Optional[_Iterable[str]] = ..., title: _Optional[str] = ..., cwd: _Optional[str] = ..., viewers: _Optional[int] = ..., created_at_ms: _Optional[int] = ..., pid: _Optional[int] = ..., viewed_by: _Optional[_Iterable[_Union[Viewer, _Mapping]]] = ..., used_at_ms: _Optional[int] = ..., created_by: _Optional[str] = ..., sandboxed: _Optional[bool] = ..., profile: _Optional[str] = ...) -> None: ...

class Viewer(_message.Message):
    __slots__ = ("client_id", "client_name", "address", "identity")
    CLIENT_ID_FIELD_NUMBER: _ClassVar[int]
    CLIENT_NAME_FIELD_NUMBER: _ClassVar[int]
    ADDRESS_FIELD_NUMBER: _ClassVar[int]
    IDENTITY_FIELD_NUMBER: _ClassVar[int]
    client_id: str
    client_name: str
    address: str
    identity: str
    def __init__(self, client_id: _Optional[str] = ..., client_name: _Optional[str] = ..., address: _Optional[str] = ..., identity: _Optional[str] = ...) -> None: ...

class CreateRequest(_message.Message):
    __slots__ = ("cols", "rows", "command", "cwd", "profile")
    COLS_FIELD_NUMBER: _ClassVar[int]
    ROWS_FIELD_NUMBER: _ClassVar[int]
    COMMAND_FIELD_NUMBER: _ClassVar[int]
    CWD_FIELD_NUMBER: _ClassVar[int]
    PROFILE_FIELD_NUMBER: _ClassVar[int]
    cols: int
    rows: int
    command: _containers.RepeatedScalarFieldContainer[str]
    cwd: str
    profile: str
    def __init__(self, cols: _Optional[int] = ..., rows: _Optional[int] = ..., command: _Optional[_Iterable[str]] = ..., cwd: _Optional[str] = ..., profile: _Optional[str] = ...) -> None: ...

class ListRequest(_message.Message):
    __slots__ = ()
    def __init__(self) -> None: ...

class ListReply(_message.Message):
    __slots__ = ("panes",)
    PANES_FIELD_NUMBER: _ClassVar[int]
    panes: _containers.RepeatedCompositeFieldContainer[PaneInfo]
    def __init__(self, panes: _Optional[_Iterable[_Union[PaneInfo, _Mapping]]] = ...) -> None: ...

class CloseRequest(_message.Message):
    __slots__ = ("pane_id",)
    PANE_ID_FIELD_NUMBER: _ClassVar[int]
    pane_id: str
    def __init__(self, pane_id: _Optional[str] = ...) -> None: ...

class CloseReply(_message.Message):
    __slots__ = ("exit_code",)
    EXIT_CODE_FIELD_NUMBER: _ClassVar[int]
    exit_code: int
    def __init__(self, exit_code: _Optional[int] = ...) -> None: ...

class OutputRequest(_message.Message):
    __slots__ = ("pane_id", "cell_width_px", "cell_height_px")
    PANE_ID_FIELD_NUMBER: _ClassVar[int]
    CELL_WIDTH_PX_FIELD_NUMBER: _ClassVar[int]
    CELL_HEIGHT_PX_FIELD_NUMBER: _ClassVar[int]
    pane_id: str
    cell_width_px: int
    cell_height_px: int
    def __init__(self, pane_id: _Optional[str] = ..., cell_width_px: _Optional[int] = ..., cell_height_px: _Optional[int] = ...) -> None: ...

class PaneExited(_message.Message):
    __slots__ = ("pane_id", "exit_code")
    PANE_ID_FIELD_NUMBER: _ClassVar[int]
    EXIT_CODE_FIELD_NUMBER: _ClassVar[int]
    pane_id: str
    exit_code: int
    def __init__(self, pane_id: _Optional[str] = ..., exit_code: _Optional[int] = ...) -> None: ...

class PaneActivity(_message.Message):
    __slots__ = ("pane_id", "kind", "at_ms")
    PANE_ID_FIELD_NUMBER: _ClassVar[int]
    KIND_FIELD_NUMBER: _ClassVar[int]
    AT_MS_FIELD_NUMBER: _ClassVar[int]
    pane_id: str
    kind: PaneActivityKind
    at_ms: int
    def __init__(self, pane_id: _Optional[str] = ..., kind: _Optional[_Union[PaneActivityKind, str]] = ..., at_ms: _Optional[int] = ...) -> None: ...

class Copied(_message.Message):
    __slots__ = ("pane_id", "transfer_id", "mime", "total_size")
    PANE_ID_FIELD_NUMBER: _ClassVar[int]
    TRANSFER_ID_FIELD_NUMBER: _ClassVar[int]
    MIME_FIELD_NUMBER: _ClassVar[int]
    TOTAL_SIZE_FIELD_NUMBER: _ClassVar[int]
    pane_id: str
    transfer_id: int
    mime: str
    total_size: int
    def __init__(self, pane_id: _Optional[str] = ..., transfer_id: _Optional[int] = ..., mime: _Optional[str] = ..., total_size: _Optional[int] = ...) -> None: ...

class ReadRequest(_message.Message):
    __slots__ = ("pane_id", "since_seq", "wait_ms", "history_lines")
    PANE_ID_FIELD_NUMBER: _ClassVar[int]
    SINCE_SEQ_FIELD_NUMBER: _ClassVar[int]
    WAIT_MS_FIELD_NUMBER: _ClassVar[int]
    HISTORY_LINES_FIELD_NUMBER: _ClassVar[int]
    pane_id: str
    since_seq: int
    wait_ms: int
    history_lines: int
    def __init__(self, pane_id: _Optional[str] = ..., since_seq: _Optional[int] = ..., wait_ms: _Optional[int] = ..., history_lines: _Optional[int] = ...) -> None: ...

class ReadReply(_message.Message):
    __slots__ = ("seq", "cols", "rows", "lines", "cursor", "title", "cwd", "alive", "exit_code", "history")
    SEQ_FIELD_NUMBER: _ClassVar[int]
    COLS_FIELD_NUMBER: _ClassVar[int]
    ROWS_FIELD_NUMBER: _ClassVar[int]
    LINES_FIELD_NUMBER: _ClassVar[int]
    CURSOR_FIELD_NUMBER: _ClassVar[int]
    TITLE_FIELD_NUMBER: _ClassVar[int]
    CWD_FIELD_NUMBER: _ClassVar[int]
    ALIVE_FIELD_NUMBER: _ClassVar[int]
    EXIT_CODE_FIELD_NUMBER: _ClassVar[int]
    HISTORY_FIELD_NUMBER: _ClassVar[int]
    seq: int
    cols: int
    rows: int
    lines: _containers.RepeatedScalarFieldContainer[str]
    cursor: _frame_pb2.Cursor
    title: str
    cwd: str
    alive: bool
    exit_code: int
    history: _containers.RepeatedScalarFieldContainer[str]
    def __init__(self, seq: _Optional[int] = ..., cols: _Optional[int] = ..., rows: _Optional[int] = ..., lines: _Optional[_Iterable[str]] = ..., cursor: _Optional[_Union[_frame_pb2.Cursor, _Mapping]] = ..., title: _Optional[str] = ..., cwd: _Optional[str] = ..., alive: _Optional[bool] = ..., exit_code: _Optional[int] = ..., history: _Optional[_Iterable[str]] = ...) -> None: ...

class ProfilesRequest(_message.Message):
    __slots__ = ()
    def __init__(self) -> None: ...

class ProfilesReply(_message.Message):
    __slots__ = ("profiles",)
    PROFILES_FIELD_NUMBER: _ClassVar[int]
    profiles: _containers.RepeatedCompositeFieldContainer[ProfileInfo]
    def __init__(self, profiles: _Optional[_Iterable[_Union[ProfileInfo, _Mapping]]] = ...) -> None: ...

class ProfileInfo(_message.Message):
    __slots__ = ("name", "description", "sandboxed", "error")
    NAME_FIELD_NUMBER: _ClassVar[int]
    DESCRIPTION_FIELD_NUMBER: _ClassVar[int]
    SANDBOXED_FIELD_NUMBER: _ClassVar[int]
    ERROR_FIELD_NUMBER: _ClassVar[int]
    name: str
    description: str
    sandboxed: bool
    error: str
    def __init__(self, name: _Optional[str] = ..., description: _Optional[str] = ..., sandboxed: _Optional[bool] = ..., error: _Optional[str] = ...) -> None: ...
