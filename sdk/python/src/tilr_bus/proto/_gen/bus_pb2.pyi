from google.protobuf.internal import containers as _containers
from google.protobuf.internal import enum_type_wrapper as _enum_type_wrapper
from google.protobuf import descriptor as _descriptor
from google.protobuf import message as _message
from collections.abc import Iterable as _Iterable, Mapping as _Mapping
from typing import ClassVar as _ClassVar, Optional as _Optional, Union as _Union

DESCRIPTOR: _descriptor.FileDescriptor

class Kind(int, metaclass=_enum_type_wrapper.EnumTypeWrapper):
    __slots__ = ()
    UNARY: _ClassVar[Kind]
    STREAM: _ClassVar[Kind]

class Lane(int, metaclass=_enum_type_wrapper.EnumTypeWrapper):
    __slots__ = ()
    INTERACTIVE: _ClassVar[Lane]
    BULK: _ClassVar[Lane]

class Select(int, metaclass=_enum_type_wrapper.EnumTypeWrapper):
    __slots__ = ()
    ROUND_ROBIN: _ClassVar[Select]
    LAST_ACTIVE: _ClassVar[Select]
UNARY: Kind
STREAM: Kind
INTERACTIVE: Lane
BULK: Lane
ROUND_ROBIN: Select
LAST_ACTIVE: Select

class Envelope(_message.Message):
    __slots__ = ("hello", "hello_ack", "call", "reply", "cancel", "event", "subscribe", "unsubscribe", "chunk", "end", "credit", "credential_update", "activate", "connection_error")
    HELLO_FIELD_NUMBER: _ClassVar[int]
    HELLO_ACK_FIELD_NUMBER: _ClassVar[int]
    CALL_FIELD_NUMBER: _ClassVar[int]
    REPLY_FIELD_NUMBER: _ClassVar[int]
    CANCEL_FIELD_NUMBER: _ClassVar[int]
    EVENT_FIELD_NUMBER: _ClassVar[int]
    SUBSCRIBE_FIELD_NUMBER: _ClassVar[int]
    UNSUBSCRIBE_FIELD_NUMBER: _ClassVar[int]
    CHUNK_FIELD_NUMBER: _ClassVar[int]
    END_FIELD_NUMBER: _ClassVar[int]
    CREDIT_FIELD_NUMBER: _ClassVar[int]
    CREDENTIAL_UPDATE_FIELD_NUMBER: _ClassVar[int]
    ACTIVATE_FIELD_NUMBER: _ClassVar[int]
    CONNECTION_ERROR_FIELD_NUMBER: _ClassVar[int]
    hello: Hello
    hello_ack: HelloAck
    call: Call
    reply: Reply
    cancel: Cancel
    event: Event
    subscribe: Subscribe
    unsubscribe: Unsubscribe
    chunk: Chunk
    end: End
    credit: Credit
    credential_update: CredentialUpdate
    activate: Activate
    connection_error: ConnectionError
    def __init__(self, hello: _Optional[_Union[Hello, _Mapping]] = ..., hello_ack: _Optional[_Union[HelloAck, _Mapping]] = ..., call: _Optional[_Union[Call, _Mapping]] = ..., reply: _Optional[_Union[Reply, _Mapping]] = ..., cancel: _Optional[_Union[Cancel, _Mapping]] = ..., event: _Optional[_Union[Event, _Mapping]] = ..., subscribe: _Optional[_Union[Subscribe, _Mapping]] = ..., unsubscribe: _Optional[_Union[Unsubscribe, _Mapping]] = ..., chunk: _Optional[_Union[Chunk, _Mapping]] = ..., end: _Optional[_Union[End, _Mapping]] = ..., credit: _Optional[_Union[Credit, _Mapping]] = ..., credential_update: _Optional[_Union[CredentialUpdate, _Mapping]] = ..., activate: _Optional[_Union[Activate, _Mapping]] = ..., connection_error: _Optional[_Union[ConnectionError, _Mapping]] = ...) -> None: ...

class Hello(_message.Message):
    __slots__ = ("protocol_version", "client_name", "client_id", "access_token", "credential", "instance_token", "provides")
    PROTOCOL_VERSION_FIELD_NUMBER: _ClassVar[int]
    CLIENT_NAME_FIELD_NUMBER: _ClassVar[int]
    CLIENT_ID_FIELD_NUMBER: _ClassVar[int]
    ACCESS_TOKEN_FIELD_NUMBER: _ClassVar[int]
    CREDENTIAL_FIELD_NUMBER: _ClassVar[int]
    INSTANCE_TOKEN_FIELD_NUMBER: _ClassVar[int]
    PROVIDES_FIELD_NUMBER: _ClassVar[int]
    protocol_version: int
    client_name: str
    client_id: str
    access_token: str
    credential: bytes
    instance_token: str
    provides: _containers.RepeatedCompositeFieldContainer[MethodSpec]
    def __init__(self, protocol_version: _Optional[int] = ..., client_name: _Optional[str] = ..., client_id: _Optional[str] = ..., access_token: _Optional[str] = ..., credential: _Optional[bytes] = ..., instance_token: _Optional[str] = ..., provides: _Optional[_Iterable[_Union[MethodSpec, _Mapping]]] = ...) -> None: ...

class MethodSpec(_message.Message):
    __slots__ = ("method", "kind", "lane", "select")
    METHOD_FIELD_NUMBER: _ClassVar[int]
    KIND_FIELD_NUMBER: _ClassVar[int]
    LANE_FIELD_NUMBER: _ClassVar[int]
    SELECT_FIELD_NUMBER: _ClassVar[int]
    method: str
    kind: Kind
    lane: Lane
    select: Select
    def __init__(self, method: _Optional[str] = ..., kind: _Optional[_Union[Kind, str]] = ..., lane: _Optional[_Union[Lane, str]] = ..., select: _Optional[_Union[Select, str]] = ...) -> None: ...

class HelloAck(_message.Message):
    __slots__ = ("protocol_version", "identity", "connection_id", "max_frame", "max_in_flight", "stream_window", "connection_window", "credential", "server_id")
    PROTOCOL_VERSION_FIELD_NUMBER: _ClassVar[int]
    IDENTITY_FIELD_NUMBER: _ClassVar[int]
    CONNECTION_ID_FIELD_NUMBER: _ClassVar[int]
    MAX_FRAME_FIELD_NUMBER: _ClassVar[int]
    MAX_IN_FLIGHT_FIELD_NUMBER: _ClassVar[int]
    STREAM_WINDOW_FIELD_NUMBER: _ClassVar[int]
    CONNECTION_WINDOW_FIELD_NUMBER: _ClassVar[int]
    CREDENTIAL_FIELD_NUMBER: _ClassVar[int]
    SERVER_ID_FIELD_NUMBER: _ClassVar[int]
    protocol_version: int
    identity: str
    connection_id: int
    max_frame: int
    max_in_flight: int
    stream_window: int
    connection_window: int
    credential: bytes
    server_id: str
    def __init__(self, protocol_version: _Optional[int] = ..., identity: _Optional[str] = ..., connection_id: _Optional[int] = ..., max_frame: _Optional[int] = ..., max_in_flight: _Optional[int] = ..., stream_window: _Optional[int] = ..., connection_window: _Optional[int] = ..., credential: _Optional[bytes] = ..., server_id: _Optional[str] = ...) -> None: ...

class Call(_message.Message):
    __slots__ = ("id", "target", "method", "payload", "trace_id", "timeout_ms", "content_type", "credential", "window")
    ID_FIELD_NUMBER: _ClassVar[int]
    TARGET_FIELD_NUMBER: _ClassVar[int]
    METHOD_FIELD_NUMBER: _ClassVar[int]
    PAYLOAD_FIELD_NUMBER: _ClassVar[int]
    FROM_FIELD_NUMBER: _ClassVar[int]
    TRACE_ID_FIELD_NUMBER: _ClassVar[int]
    TIMEOUT_MS_FIELD_NUMBER: _ClassVar[int]
    CONTENT_TYPE_FIELD_NUMBER: _ClassVar[int]
    CREDENTIAL_FIELD_NUMBER: _ClassVar[int]
    WINDOW_FIELD_NUMBER: _ClassVar[int]
    id: int
    target: str
    method: str
    payload: bytes
    trace_id: str
    timeout_ms: int
    content_type: str
    credential: bytes
    window: int
    def __init__(self, id: _Optional[int] = ..., target: _Optional[str] = ..., method: _Optional[str] = ..., payload: _Optional[bytes] = ..., trace_id: _Optional[str] = ..., timeout_ms: _Optional[int] = ..., content_type: _Optional[str] = ..., credential: _Optional[bytes] = ..., window: _Optional[int] = ..., **kwargs) -> None: ...

class Reply(_message.Message):
    __slots__ = ("id", "payload", "error", "content_type")
    ID_FIELD_NUMBER: _ClassVar[int]
    PAYLOAD_FIELD_NUMBER: _ClassVar[int]
    ERROR_FIELD_NUMBER: _ClassVar[int]
    CONTENT_TYPE_FIELD_NUMBER: _ClassVar[int]
    id: int
    payload: bytes
    error: Error
    content_type: str
    def __init__(self, id: _Optional[int] = ..., payload: _Optional[bytes] = ..., error: _Optional[_Union[Error, _Mapping]] = ..., content_type: _Optional[str] = ...) -> None: ...

class Chunk(_message.Message):
    __slots__ = ("id", "payload", "content_encoding")
    ID_FIELD_NUMBER: _ClassVar[int]
    PAYLOAD_FIELD_NUMBER: _ClassVar[int]
    CONTENT_ENCODING_FIELD_NUMBER: _ClassVar[int]
    id: int
    payload: bytes
    content_encoding: str
    def __init__(self, id: _Optional[int] = ..., payload: _Optional[bytes] = ..., content_encoding: _Optional[str] = ...) -> None: ...

class End(_message.Message):
    __slots__ = ("id", "payload")
    ID_FIELD_NUMBER: _ClassVar[int]
    PAYLOAD_FIELD_NUMBER: _ClassVar[int]
    id: int
    payload: bytes
    def __init__(self, id: _Optional[int] = ..., payload: _Optional[bytes] = ...) -> None: ...

class Credit(_message.Message):
    __slots__ = ("id", "bytes")
    ID_FIELD_NUMBER: _ClassVar[int]
    BYTES_FIELD_NUMBER: _ClassVar[int]
    id: int
    bytes: int
    def __init__(self, id: _Optional[int] = ..., bytes: _Optional[int] = ...) -> None: ...

class Cancel(_message.Message):
    __slots__ = ("id", "reason")
    ID_FIELD_NUMBER: _ClassVar[int]
    REASON_FIELD_NUMBER: _ClassVar[int]
    id: int
    reason: str
    def __init__(self, id: _Optional[int] = ..., reason: _Optional[str] = ...) -> None: ...

class Event(_message.Message):
    __slots__ = ("subject", "payload", "content_type", "scope")
    SUBJECT_FIELD_NUMBER: _ClassVar[int]
    PAYLOAD_FIELD_NUMBER: _ClassVar[int]
    CONTENT_TYPE_FIELD_NUMBER: _ClassVar[int]
    FROM_FIELD_NUMBER: _ClassVar[int]
    SCOPE_FIELD_NUMBER: _ClassVar[int]
    subject: str
    payload: bytes
    content_type: str
    scope: str
    def __init__(self, subject: _Optional[str] = ..., payload: _Optional[bytes] = ..., content_type: _Optional[str] = ..., scope: _Optional[str] = ..., **kwargs) -> None: ...

class Subscribe(_message.Message):
    __slots__ = ("id", "pattern")
    ID_FIELD_NUMBER: _ClassVar[int]
    PATTERN_FIELD_NUMBER: _ClassVar[int]
    id: int
    pattern: str
    def __init__(self, id: _Optional[int] = ..., pattern: _Optional[str] = ...) -> None: ...

class Unsubscribe(_message.Message):
    __slots__ = ("id",)
    ID_FIELD_NUMBER: _ClassVar[int]
    id: int
    def __init__(self, id: _Optional[int] = ...) -> None: ...

class CredentialUpdate(_message.Message):
    __slots__ = ("credential",)
    CREDENTIAL_FIELD_NUMBER: _ClassVar[int]
    credential: bytes
    def __init__(self, credential: _Optional[bytes] = ...) -> None: ...

class Activate(_message.Message):
    __slots__ = ()
    def __init__(self) -> None: ...

class ConnectionError(_message.Message):
    __slots__ = ("code", "message")
    CODE_FIELD_NUMBER: _ClassVar[int]
    MESSAGE_FIELD_NUMBER: _ClassVar[int]
    code: str
    message: str
    def __init__(self, code: _Optional[str] = ..., message: _Optional[str] = ...) -> None: ...

class Error(_message.Message):
    __slots__ = ("code", "message")
    CODE_FIELD_NUMBER: _ClassVar[int]
    MESSAGE_FIELD_NUMBER: _ClassVar[int]
    code: str
    message: str
    def __init__(self, code: _Optional[str] = ..., message: _Optional[str] = ...) -> None: ...
