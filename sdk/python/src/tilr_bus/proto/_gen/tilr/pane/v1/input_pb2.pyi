from google.protobuf.internal import containers as _containers
from google.protobuf.internal import enum_type_wrapper as _enum_type_wrapper
from google.protobuf import descriptor as _descriptor
from google.protobuf import message as _message
from collections.abc import Iterable as _Iterable, Mapping as _Mapping
from typing import ClassVar as _ClassVar, Optional as _Optional, Union as _Union

DESCRIPTOR: _descriptor.FileDescriptor

class KeyAction(int, metaclass=_enum_type_wrapper.EnumTypeWrapper):
    __slots__ = ()
    KEY_ACTION_PRESS: _ClassVar[KeyAction]
    KEY_ACTION_RELEASE: _ClassVar[KeyAction]
    KEY_ACTION_REPEAT: _ClassVar[KeyAction]

class NamedKey(int, metaclass=_enum_type_wrapper.EnumTypeWrapper):
    __slots__ = ()
    NAMED_KEY_UNSPECIFIED: _ClassVar[NamedKey]
    NAMED_KEY_ENTER: _ClassVar[NamedKey]
    NAMED_KEY_TAB: _ClassVar[NamedKey]
    NAMED_KEY_BACKSPACE: _ClassVar[NamedKey]
    NAMED_KEY_ESCAPE: _ClassVar[NamedKey]
    NAMED_KEY_SPACE: _ClassVar[NamedKey]
    NAMED_KEY_UP: _ClassVar[NamedKey]
    NAMED_KEY_DOWN: _ClassVar[NamedKey]
    NAMED_KEY_LEFT: _ClassVar[NamedKey]
    NAMED_KEY_RIGHT: _ClassVar[NamedKey]
    NAMED_KEY_HOME: _ClassVar[NamedKey]
    NAMED_KEY_END: _ClassVar[NamedKey]
    NAMED_KEY_PAGE_UP: _ClassVar[NamedKey]
    NAMED_KEY_PAGE_DOWN: _ClassVar[NamedKey]
    NAMED_KEY_INSERT: _ClassVar[NamedKey]
    NAMED_KEY_DELETE: _ClassVar[NamedKey]
    NAMED_KEY_F1: _ClassVar[NamedKey]
    NAMED_KEY_F2: _ClassVar[NamedKey]
    NAMED_KEY_F3: _ClassVar[NamedKey]
    NAMED_KEY_F4: _ClassVar[NamedKey]
    NAMED_KEY_F5: _ClassVar[NamedKey]
    NAMED_KEY_F6: _ClassVar[NamedKey]
    NAMED_KEY_F7: _ClassVar[NamedKey]
    NAMED_KEY_F8: _ClassVar[NamedKey]
    NAMED_KEY_F9: _ClassVar[NamedKey]
    NAMED_KEY_F10: _ClassVar[NamedKey]
    NAMED_KEY_F11: _ClassVar[NamedKey]
    NAMED_KEY_F12: _ClassVar[NamedKey]

class MouseAction(int, metaclass=_enum_type_wrapper.EnumTypeWrapper):
    __slots__ = ()
    MOUSE_ACTION_PRESS: _ClassVar[MouseAction]
    MOUSE_ACTION_RELEASE: _ClassVar[MouseAction]
    MOUSE_ACTION_MOTION: _ClassVar[MouseAction]

class MouseButton(int, metaclass=_enum_type_wrapper.EnumTypeWrapper):
    __slots__ = ()
    MOUSE_BUTTON_NONE: _ClassVar[MouseButton]
    MOUSE_BUTTON_LEFT: _ClassVar[MouseButton]
    MOUSE_BUTTON_RIGHT: _ClassVar[MouseButton]
    MOUSE_BUTTON_MIDDLE: _ClassVar[MouseButton]
    MOUSE_BUTTON_WHEEL_UP: _ClassVar[MouseButton]
    MOUSE_BUTTON_WHEEL_DOWN: _ClassVar[MouseButton]
KEY_ACTION_PRESS: KeyAction
KEY_ACTION_RELEASE: KeyAction
KEY_ACTION_REPEAT: KeyAction
NAMED_KEY_UNSPECIFIED: NamedKey
NAMED_KEY_ENTER: NamedKey
NAMED_KEY_TAB: NamedKey
NAMED_KEY_BACKSPACE: NamedKey
NAMED_KEY_ESCAPE: NamedKey
NAMED_KEY_SPACE: NamedKey
NAMED_KEY_UP: NamedKey
NAMED_KEY_DOWN: NamedKey
NAMED_KEY_LEFT: NamedKey
NAMED_KEY_RIGHT: NamedKey
NAMED_KEY_HOME: NamedKey
NAMED_KEY_END: NamedKey
NAMED_KEY_PAGE_UP: NamedKey
NAMED_KEY_PAGE_DOWN: NamedKey
NAMED_KEY_INSERT: NamedKey
NAMED_KEY_DELETE: NamedKey
NAMED_KEY_F1: NamedKey
NAMED_KEY_F2: NamedKey
NAMED_KEY_F3: NamedKey
NAMED_KEY_F4: NamedKey
NAMED_KEY_F5: NamedKey
NAMED_KEY_F6: NamedKey
NAMED_KEY_F7: NamedKey
NAMED_KEY_F8: NamedKey
NAMED_KEY_F9: NamedKey
NAMED_KEY_F10: NamedKey
NAMED_KEY_F11: NamedKey
NAMED_KEY_F12: NamedKey
MOUSE_ACTION_PRESS: MouseAction
MOUSE_ACTION_RELEASE: MouseAction
MOUSE_ACTION_MOTION: MouseAction
MOUSE_BUTTON_NONE: MouseButton
MOUSE_BUTTON_LEFT: MouseButton
MOUSE_BUTTON_RIGHT: MouseButton
MOUSE_BUTTON_MIDDLE: MouseButton
MOUSE_BUTTON_WHEEL_UP: MouseButton
MOUSE_BUTTON_WHEEL_DOWN: MouseButton

class InputRequest(_message.Message):
    __slots__ = ("pane_id", "key", "mouse", "paste", "raw", "scroll", "clear", "seq", "input_seq")
    PANE_ID_FIELD_NUMBER: _ClassVar[int]
    KEY_FIELD_NUMBER: _ClassVar[int]
    MOUSE_FIELD_NUMBER: _ClassVar[int]
    PASTE_FIELD_NUMBER: _ClassVar[int]
    RAW_FIELD_NUMBER: _ClassVar[int]
    SCROLL_FIELD_NUMBER: _ClassVar[int]
    CLEAR_FIELD_NUMBER: _ClassVar[int]
    SEQ_FIELD_NUMBER: _ClassVar[int]
    INPUT_SEQ_FIELD_NUMBER: _ClassVar[int]
    pane_id: str
    key: KeyInput
    mouse: MouseInput
    paste: Paste
    raw: RawInput
    scroll: Scroll
    clear: ClearPane
    seq: int
    input_seq: int
    def __init__(self, pane_id: _Optional[str] = ..., key: _Optional[_Union[KeyInput, _Mapping]] = ..., mouse: _Optional[_Union[MouseInput, _Mapping]] = ..., paste: _Optional[_Union[Paste, _Mapping]] = ..., raw: _Optional[_Union[RawInput, _Mapping]] = ..., scroll: _Optional[_Union[Scroll, _Mapping]] = ..., clear: _Optional[_Union[ClearPane, _Mapping]] = ..., seq: _Optional[int] = ..., input_seq: _Optional[int] = ...) -> None: ...

class ResizeRequest(_message.Message):
    __slots__ = ("pane_id", "cols", "rows", "cell_width_px", "cell_height_px", "seq", "input_seq")
    PANE_ID_FIELD_NUMBER: _ClassVar[int]
    COLS_FIELD_NUMBER: _ClassVar[int]
    ROWS_FIELD_NUMBER: _ClassVar[int]
    CELL_WIDTH_PX_FIELD_NUMBER: _ClassVar[int]
    CELL_HEIGHT_PX_FIELD_NUMBER: _ClassVar[int]
    SEQ_FIELD_NUMBER: _ClassVar[int]
    INPUT_SEQ_FIELD_NUMBER: _ClassVar[int]
    pane_id: str
    cols: int
    rows: int
    cell_width_px: int
    cell_height_px: int
    seq: int
    input_seq: int
    def __init__(self, pane_id: _Optional[str] = ..., cols: _Optional[int] = ..., rows: _Optional[int] = ..., cell_width_px: _Optional[int] = ..., cell_height_px: _Optional[int] = ..., seq: _Optional[int] = ..., input_seq: _Optional[int] = ...) -> None: ...

class InputAck(_message.Message):
    __slots__ = ("pane_id", "seq")
    PANE_ID_FIELD_NUMBER: _ClassVar[int]
    SEQ_FIELD_NUMBER: _ClassVar[int]
    pane_id: str
    seq: int
    def __init__(self, pane_id: _Optional[str] = ..., seq: _Optional[int] = ...) -> None: ...

class AppliedRequest(_message.Message):
    __slots__ = ()
    def __init__(self) -> None: ...

class AppliedReply(_message.Message):
    __slots__ = ("applied",)
    APPLIED_FIELD_NUMBER: _ClassVar[int]
    applied: _containers.RepeatedCompositeFieldContainer[InputAck]
    def __init__(self, applied: _Optional[_Iterable[_Union[InputAck, _Mapping]]] = ...) -> None: ...

class InputFinalizeRequest(_message.Message):
    __slots__ = ()
    def __init__(self) -> None: ...

class InputFinalizeReply(_message.Message):
    __slots__ = ()
    def __init__(self) -> None: ...

class KeyInput(_message.Message):
    __slots__ = ("action", "mods", "named", "codepoint", "text", "code")
    ACTION_FIELD_NUMBER: _ClassVar[int]
    MODS_FIELD_NUMBER: _ClassVar[int]
    NAMED_FIELD_NUMBER: _ClassVar[int]
    CODEPOINT_FIELD_NUMBER: _ClassVar[int]
    TEXT_FIELD_NUMBER: _ClassVar[int]
    CODE_FIELD_NUMBER: _ClassVar[int]
    action: KeyAction
    mods: int
    named: NamedKey
    codepoint: int
    text: str
    code: str
    def __init__(self, action: _Optional[_Union[KeyAction, str]] = ..., mods: _Optional[int] = ..., named: _Optional[_Union[NamedKey, str]] = ..., codepoint: _Optional[int] = ..., text: _Optional[str] = ..., code: _Optional[str] = ...) -> None: ...

class MouseInput(_message.Message):
    __slots__ = ("action", "button", "mods", "x", "y")
    ACTION_FIELD_NUMBER: _ClassVar[int]
    BUTTON_FIELD_NUMBER: _ClassVar[int]
    MODS_FIELD_NUMBER: _ClassVar[int]
    X_FIELD_NUMBER: _ClassVar[int]
    Y_FIELD_NUMBER: _ClassVar[int]
    action: MouseAction
    button: MouseButton
    mods: int
    x: int
    y: int
    def __init__(self, action: _Optional[_Union[MouseAction, str]] = ..., button: _Optional[_Union[MouseButton, str]] = ..., mods: _Optional[int] = ..., x: _Optional[int] = ..., y: _Optional[int] = ...) -> None: ...

class Paste(_message.Message):
    __slots__ = ("text",)
    TEXT_FIELD_NUMBER: _ClassVar[int]
    text: str
    def __init__(self, text: _Optional[str] = ...) -> None: ...

class RawInput(_message.Message):
    __slots__ = ("data",)
    DATA_FIELD_NUMBER: _ClassVar[int]
    data: bytes
    def __init__(self, data: _Optional[bytes] = ...) -> None: ...

class Scroll(_message.Message):
    __slots__ = ("delta", "to_bottom")
    DELTA_FIELD_NUMBER: _ClassVar[int]
    TO_BOTTOM_FIELD_NUMBER: _ClassVar[int]
    delta: int
    to_bottom: bool
    def __init__(self, delta: _Optional[int] = ..., to_bottom: _Optional[bool] = ...) -> None: ...

class ClearPane(_message.Message):
    __slots__ = ("history",)
    HISTORY_FIELD_NUMBER: _ClassVar[int]
    history: bool
    def __init__(self, history: _Optional[bool] = ...) -> None: ...
