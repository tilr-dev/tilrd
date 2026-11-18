from google.protobuf.internal import containers as _containers
from google.protobuf.internal import enum_type_wrapper as _enum_type_wrapper
from google.protobuf import descriptor as _descriptor
from google.protobuf import message as _message
from collections.abc import Iterable as _Iterable, Mapping as _Mapping
from typing import ClassVar as _ClassVar, Optional as _Optional, Union as _Union

DESCRIPTOR: _descriptor.FileDescriptor

class Underline(int, metaclass=_enum_type_wrapper.EnumTypeWrapper):
    __slots__ = ()
    UNDERLINE_NONE: _ClassVar[Underline]
    UNDERLINE_SINGLE: _ClassVar[Underline]
    UNDERLINE_DOUBLE: _ClassVar[Underline]
    UNDERLINE_CURLY: _ClassVar[Underline]
    UNDERLINE_DOTTED: _ClassVar[Underline]
    UNDERLINE_DASHED: _ClassVar[Underline]

class CursorShape(int, metaclass=_enum_type_wrapper.EnumTypeWrapper):
    __slots__ = ()
    CURSOR_SHAPE_BLOCK: _ClassVar[CursorShape]
    CURSOR_SHAPE_BAR: _ClassVar[CursorShape]
    CURSOR_SHAPE_UNDERLINE: _ClassVar[CursorShape]
    CURSOR_SHAPE_BLOCK_HOLLOW: _ClassVar[CursorShape]
UNDERLINE_NONE: Underline
UNDERLINE_SINGLE: Underline
UNDERLINE_DOUBLE: Underline
UNDERLINE_CURLY: Underline
UNDERLINE_DOTTED: Underline
UNDERLINE_DASHED: Underline
CURSOR_SHAPE_BLOCK: CursorShape
CURSOR_SHAPE_BAR: CursorShape
CURSOR_SHAPE_UNDERLINE: CursorShape
CURSOR_SHAPE_BLOCK_HOLLOW: CursorShape

class PaneOutput(_message.Message):
    __slots__ = ("frame", "images")
    FRAME_FIELD_NUMBER: _ClassVar[int]
    IMAGES_FIELD_NUMBER: _ClassVar[int]
    frame: Frame
    images: FrameImages
    def __init__(self, frame: _Optional[_Union[Frame, _Mapping]] = ..., images: _Optional[_Union[FrameImages, _Mapping]] = ...) -> None: ...

class FrameImages(_message.Message):
    __slots__ = ("images",)
    IMAGES_FIELD_NUMBER: _ClassVar[int]
    images: _containers.RepeatedCompositeFieldContainer[Image]
    def __init__(self, images: _Optional[_Iterable[_Union[Image, _Mapping]]] = ...) -> None: ...

class Color(_message.Message):
    __slots__ = ("palette", "rgb")
    PALETTE_FIELD_NUMBER: _ClassVar[int]
    RGB_FIELD_NUMBER: _ClassVar[int]
    palette: int
    rgb: int
    def __init__(self, palette: _Optional[int] = ..., rgb: _Optional[int] = ...) -> None: ...

class Style(_message.Message):
    __slots__ = ("fg", "bg", "underline_color", "attrs", "underline")
    FG_FIELD_NUMBER: _ClassVar[int]
    BG_FIELD_NUMBER: _ClassVar[int]
    UNDERLINE_COLOR_FIELD_NUMBER: _ClassVar[int]
    ATTRS_FIELD_NUMBER: _ClassVar[int]
    UNDERLINE_FIELD_NUMBER: _ClassVar[int]
    fg: Color
    bg: Color
    underline_color: Color
    attrs: int
    underline: Underline
    def __init__(self, fg: _Optional[_Union[Color, _Mapping]] = ..., bg: _Optional[_Union[Color, _Mapping]] = ..., underline_color: _Optional[_Union[Color, _Mapping]] = ..., attrs: _Optional[int] = ..., underline: _Optional[_Union[Underline, str]] = ...) -> None: ...

class Run(_message.Message):
    __slots__ = ("style", "text", "wide", "cluster", "link")
    STYLE_FIELD_NUMBER: _ClassVar[int]
    TEXT_FIELD_NUMBER: _ClassVar[int]
    WIDE_FIELD_NUMBER: _ClassVar[int]
    CLUSTER_FIELD_NUMBER: _ClassVar[int]
    LINK_FIELD_NUMBER: _ClassVar[int]
    style: int
    text: str
    wide: bool
    cluster: bool
    link: int
    def __init__(self, style: _Optional[int] = ..., text: _Optional[str] = ..., wide: _Optional[bool] = ..., cluster: _Optional[bool] = ..., link: _Optional[int] = ...) -> None: ...

class Row(_message.Message):
    __slots__ = ("y", "runs")
    Y_FIELD_NUMBER: _ClassVar[int]
    RUNS_FIELD_NUMBER: _ClassVar[int]
    y: int
    runs: _containers.RepeatedCompositeFieldContainer[Run]
    def __init__(self, y: _Optional[int] = ..., runs: _Optional[_Iterable[_Union[Run, _Mapping]]] = ...) -> None: ...

class Cursor(_message.Message):
    __slots__ = ("visible", "x", "y", "shape", "blinking")
    VISIBLE_FIELD_NUMBER: _ClassVar[int]
    X_FIELD_NUMBER: _ClassVar[int]
    Y_FIELD_NUMBER: _ClassVar[int]
    SHAPE_FIELD_NUMBER: _ClassVar[int]
    BLINKING_FIELD_NUMBER: _ClassVar[int]
    visible: bool
    x: int
    y: int
    shape: CursorShape
    blinking: bool
    def __init__(self, visible: _Optional[bool] = ..., x: _Optional[int] = ..., y: _Optional[int] = ..., shape: _Optional[_Union[CursorShape, str]] = ..., blinking: _Optional[bool] = ...) -> None: ...

class Frame(_message.Message):
    __slots__ = ("links", "pane_id", "seq", "full", "cols", "rows", "styles", "changed", "cursor", "title", "mouse_tracking", "alt_screen", "placements_changed", "placements", "images", "cell_width_px", "cell_height_px", "viewport_top", "last_input_seq", "timing")
    LINKS_FIELD_NUMBER: _ClassVar[int]
    PANE_ID_FIELD_NUMBER: _ClassVar[int]
    SEQ_FIELD_NUMBER: _ClassVar[int]
    FULL_FIELD_NUMBER: _ClassVar[int]
    COLS_FIELD_NUMBER: _ClassVar[int]
    ROWS_FIELD_NUMBER: _ClassVar[int]
    STYLES_FIELD_NUMBER: _ClassVar[int]
    CHANGED_FIELD_NUMBER: _ClassVar[int]
    CURSOR_FIELD_NUMBER: _ClassVar[int]
    TITLE_FIELD_NUMBER: _ClassVar[int]
    MOUSE_TRACKING_FIELD_NUMBER: _ClassVar[int]
    ALT_SCREEN_FIELD_NUMBER: _ClassVar[int]
    PLACEMENTS_CHANGED_FIELD_NUMBER: _ClassVar[int]
    PLACEMENTS_FIELD_NUMBER: _ClassVar[int]
    IMAGES_FIELD_NUMBER: _ClassVar[int]
    CELL_WIDTH_PX_FIELD_NUMBER: _ClassVar[int]
    CELL_HEIGHT_PX_FIELD_NUMBER: _ClassVar[int]
    VIEWPORT_TOP_FIELD_NUMBER: _ClassVar[int]
    LAST_INPUT_SEQ_FIELD_NUMBER: _ClassVar[int]
    TIMING_FIELD_NUMBER: _ClassVar[int]
    links: _containers.RepeatedScalarFieldContainer[str]
    pane_id: str
    seq: int
    full: bool
    cols: int
    rows: int
    styles: _containers.RepeatedCompositeFieldContainer[Style]
    changed: _containers.RepeatedCompositeFieldContainer[Row]
    cursor: Cursor
    title: str
    mouse_tracking: bool
    alt_screen: bool
    placements_changed: bool
    placements: _containers.RepeatedCompositeFieldContainer[Placement]
    images: _containers.RepeatedCompositeFieldContainer[Image]
    cell_width_px: int
    cell_height_px: int
    viewport_top: int
    last_input_seq: int
    timing: FrameTiming
    def __init__(self, links: _Optional[_Iterable[str]] = ..., pane_id: _Optional[str] = ..., seq: _Optional[int] = ..., full: _Optional[bool] = ..., cols: _Optional[int] = ..., rows: _Optional[int] = ..., styles: _Optional[_Iterable[_Union[Style, _Mapping]]] = ..., changed: _Optional[_Iterable[_Union[Row, _Mapping]]] = ..., cursor: _Optional[_Union[Cursor, _Mapping]] = ..., title: _Optional[str] = ..., mouse_tracking: _Optional[bool] = ..., alt_screen: _Optional[bool] = ..., placements_changed: _Optional[bool] = ..., placements: _Optional[_Iterable[_Union[Placement, _Mapping]]] = ..., images: _Optional[_Iterable[_Union[Image, _Mapping]]] = ..., cell_width_px: _Optional[int] = ..., cell_height_px: _Optional[int] = ..., viewport_top: _Optional[int] = ..., last_input_seq: _Optional[int] = ..., timing: _Optional[_Union[FrameTiming, _Mapping]] = ...) -> None: ...

class FrameTiming(_message.Message):
    __slots__ = ("wait_us", "parse_us", "build_us", "encode_us", "output_bytes", "reads", "input_us")
    WAIT_US_FIELD_NUMBER: _ClassVar[int]
    PARSE_US_FIELD_NUMBER: _ClassVar[int]
    BUILD_US_FIELD_NUMBER: _ClassVar[int]
    ENCODE_US_FIELD_NUMBER: _ClassVar[int]
    OUTPUT_BYTES_FIELD_NUMBER: _ClassVar[int]
    READS_FIELD_NUMBER: _ClassVar[int]
    INPUT_US_FIELD_NUMBER: _ClassVar[int]
    wait_us: int
    parse_us: int
    build_us: int
    encode_us: int
    output_bytes: int
    reads: int
    input_us: int
    def __init__(self, wait_us: _Optional[int] = ..., parse_us: _Optional[int] = ..., build_us: _Optional[int] = ..., encode_us: _Optional[int] = ..., output_bytes: _Optional[int] = ..., reads: _Optional[int] = ..., input_us: _Optional[int] = ...) -> None: ...

class Placement(_message.Message):
    __slots__ = ("image_id", "image_generation", "col", "row", "cols", "rows", "source_x", "source_y", "source_width", "source_height", "x_offset", "y_offset", "z", "pixel_width", "pixel_height")
    IMAGE_ID_FIELD_NUMBER: _ClassVar[int]
    IMAGE_GENERATION_FIELD_NUMBER: _ClassVar[int]
    COL_FIELD_NUMBER: _ClassVar[int]
    ROW_FIELD_NUMBER: _ClassVar[int]
    COLS_FIELD_NUMBER: _ClassVar[int]
    ROWS_FIELD_NUMBER: _ClassVar[int]
    SOURCE_X_FIELD_NUMBER: _ClassVar[int]
    SOURCE_Y_FIELD_NUMBER: _ClassVar[int]
    SOURCE_WIDTH_FIELD_NUMBER: _ClassVar[int]
    SOURCE_HEIGHT_FIELD_NUMBER: _ClassVar[int]
    X_OFFSET_FIELD_NUMBER: _ClassVar[int]
    Y_OFFSET_FIELD_NUMBER: _ClassVar[int]
    Z_FIELD_NUMBER: _ClassVar[int]
    PIXEL_WIDTH_FIELD_NUMBER: _ClassVar[int]
    PIXEL_HEIGHT_FIELD_NUMBER: _ClassVar[int]
    image_id: int
    image_generation: int
    col: int
    row: int
    cols: int
    rows: int
    source_x: int
    source_y: int
    source_width: int
    source_height: int
    x_offset: int
    y_offset: int
    z: int
    pixel_width: int
    pixel_height: int
    def __init__(self, image_id: _Optional[int] = ..., image_generation: _Optional[int] = ..., col: _Optional[int] = ..., row: _Optional[int] = ..., cols: _Optional[int] = ..., rows: _Optional[int] = ..., source_x: _Optional[int] = ..., source_y: _Optional[int] = ..., source_width: _Optional[int] = ..., source_height: _Optional[int] = ..., x_offset: _Optional[int] = ..., y_offset: _Optional[int] = ..., z: _Optional[int] = ..., pixel_width: _Optional[int] = ..., pixel_height: _Optional[int] = ...) -> None: ...

class Image(_message.Message):
    __slots__ = ("id", "generation", "width", "height", "png")
    ID_FIELD_NUMBER: _ClassVar[int]
    GENERATION_FIELD_NUMBER: _ClassVar[int]
    WIDTH_FIELD_NUMBER: _ClassVar[int]
    HEIGHT_FIELD_NUMBER: _ClassVar[int]
    PNG_FIELD_NUMBER: _ClassVar[int]
    id: int
    generation: int
    width: int
    height: int
    png: bytes
    def __init__(self, id: _Optional[int] = ..., generation: _Optional[int] = ..., width: _Optional[int] = ..., height: _Optional[int] = ..., png: _Optional[bytes] = ...) -> None: ...
