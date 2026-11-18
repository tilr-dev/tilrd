from tilr_bus.proto._gen import bus_pb2 as _bus_pb2
from google.protobuf.internal import containers as _containers
from google.protobuf import descriptor as _descriptor
from google.protobuf import message as _message
from collections.abc import Iterable as _Iterable, Mapping as _Mapping
from typing import ClassVar as _ClassVar, Optional as _Optional, Union as _Union

DESCRIPTOR: _descriptor.FileDescriptor

class PingRequest(_message.Message):
    __slots__ = ("nonce",)
    NONCE_FIELD_NUMBER: _ClassVar[int]
    nonce: int
    def __init__(self, nonce: _Optional[int] = ...) -> None: ...

class PingReply(_message.Message):
    __slots__ = ("nonce",)
    NONCE_FIELD_NUMBER: _ClassVar[int]
    nonce: int
    def __init__(self, nonce: _Optional[int] = ...) -> None: ...

class InstancesRequest(_message.Message):
    __slots__ = ()
    def __init__(self) -> None: ...

class InstancesReply(_message.Message):
    __slots__ = ("instances", "participants")
    INSTANCES_FIELD_NUMBER: _ClassVar[int]
    PARTICIPANTS_FIELD_NUMBER: _ClassVar[int]
    instances: _containers.RepeatedCompositeFieldContainer[InstanceInfo]
    participants: _containers.RepeatedCompositeFieldContainer[ParticipantInfo]
    def __init__(self, instances: _Optional[_Iterable[_Union[InstanceInfo, _Mapping]]] = ..., participants: _Optional[_Iterable[_Union[ParticipantInfo, _Mapping]]] = ...) -> None: ...

class InstanceInfo(_message.Message):
    __slots__ = ("name", "module", "transport", "state", "pid", "restarts", "provides")
    NAME_FIELD_NUMBER: _ClassVar[int]
    MODULE_FIELD_NUMBER: _ClassVar[int]
    TRANSPORT_FIELD_NUMBER: _ClassVar[int]
    STATE_FIELD_NUMBER: _ClassVar[int]
    PID_FIELD_NUMBER: _ClassVar[int]
    RESTARTS_FIELD_NUMBER: _ClassVar[int]
    PROVIDES_FIELD_NUMBER: _ClassVar[int]
    name: str
    module: str
    transport: str
    state: str
    pid: int
    restarts: int
    provides: _containers.RepeatedScalarFieldContainer[str]
    def __init__(self, name: _Optional[str] = ..., module: _Optional[str] = ..., transport: _Optional[str] = ..., state: _Optional[str] = ..., pid: _Optional[int] = ..., restarts: _Optional[int] = ..., provides: _Optional[_Iterable[str]] = ...) -> None: ...

class ParticipantInfo(_message.Message):
    __slots__ = ("identity", "client_name", "client_id", "connection_id", "address", "origin", "provides")
    IDENTITY_FIELD_NUMBER: _ClassVar[int]
    CLIENT_NAME_FIELD_NUMBER: _ClassVar[int]
    CLIENT_ID_FIELD_NUMBER: _ClassVar[int]
    CONNECTION_ID_FIELD_NUMBER: _ClassVar[int]
    ADDRESS_FIELD_NUMBER: _ClassVar[int]
    ORIGIN_FIELD_NUMBER: _ClassVar[int]
    PROVIDES_FIELD_NUMBER: _ClassVar[int]
    identity: str
    client_name: str
    client_id: str
    connection_id: int
    address: str
    origin: str
    provides: _containers.RepeatedScalarFieldContainer[str]
    def __init__(self, identity: _Optional[str] = ..., client_name: _Optional[str] = ..., client_id: _Optional[str] = ..., connection_id: _Optional[int] = ..., address: _Optional[str] = ..., origin: _Optional[str] = ..., provides: _Optional[_Iterable[str]] = ...) -> None: ...

class MethodsRequest(_message.Message):
    __slots__ = ()
    def __init__(self) -> None: ...

class MethodsReply(_message.Message):
    __slots__ = ("methods",)
    METHODS_FIELD_NUMBER: _ClassVar[int]
    methods: _containers.RepeatedCompositeFieldContainer[MethodInfo]
    def __init__(self, methods: _Optional[_Iterable[_Union[MethodInfo, _Mapping]]] = ...) -> None: ...

class MethodInfo(_message.Message):
    __slots__ = ("method", "kind", "lane", "select", "providers")
    METHOD_FIELD_NUMBER: _ClassVar[int]
    KIND_FIELD_NUMBER: _ClassVar[int]
    LANE_FIELD_NUMBER: _ClassVar[int]
    SELECT_FIELD_NUMBER: _ClassVar[int]
    PROVIDERS_FIELD_NUMBER: _ClassVar[int]
    method: str
    kind: _bus_pb2.Kind
    lane: _bus_pb2.Lane
    select: _bus_pb2.Select
    providers: _containers.RepeatedScalarFieldContainer[str]
    def __init__(self, method: _Optional[str] = ..., kind: _Optional[_Union[_bus_pb2.Kind, str]] = ..., lane: _Optional[_Union[_bus_pb2.Lane, str]] = ..., select: _Optional[_Union[_bus_pb2.Select, str]] = ..., providers: _Optional[_Iterable[str]] = ...) -> None: ...

class StartRequest(_message.Message):
    __slots__ = ("instance",)
    INSTANCE_FIELD_NUMBER: _ClassVar[int]
    instance: str
    def __init__(self, instance: _Optional[str] = ...) -> None: ...

class StartReply(_message.Message):
    __slots__ = ("pid",)
    PID_FIELD_NUMBER: _ClassVar[int]
    pid: int
    def __init__(self, pid: _Optional[int] = ...) -> None: ...

class StopRequest(_message.Message):
    __slots__ = ("instance", "drain_ms")
    INSTANCE_FIELD_NUMBER: _ClassVar[int]
    DRAIN_MS_FIELD_NUMBER: _ClassVar[int]
    instance: str
    drain_ms: int
    def __init__(self, instance: _Optional[str] = ..., drain_ms: _Optional[int] = ...) -> None: ...

class StopReply(_message.Message):
    __slots__ = ()
    def __init__(self) -> None: ...

class ReloadRequest(_message.Message):
    __slots__ = ("instance", "module", "all")
    INSTANCE_FIELD_NUMBER: _ClassVar[int]
    MODULE_FIELD_NUMBER: _ClassVar[int]
    ALL_FIELD_NUMBER: _ClassVar[int]
    instance: str
    module: str
    all: bool
    def __init__(self, instance: _Optional[str] = ..., module: _Optional[str] = ..., all: _Optional[bool] = ...) -> None: ...

class ReloadReply(_message.Message):
    __slots__ = ("reloaded",)
    RELOADED_FIELD_NUMBER: _ClassVar[int]
    reloaded: _containers.RepeatedCompositeFieldContainer[Reloaded]
    def __init__(self, reloaded: _Optional[_Iterable[_Union[Reloaded, _Mapping]]] = ...) -> None: ...

class Reloaded(_message.Message):
    __slots__ = ("instance", "old_pid", "new_pid")
    INSTANCE_FIELD_NUMBER: _ClassVar[int]
    OLD_PID_FIELD_NUMBER: _ClassVar[int]
    NEW_PID_FIELD_NUMBER: _ClassVar[int]
    instance: str
    old_pid: int
    new_pid: int
    def __init__(self, instance: _Optional[str] = ..., old_pid: _Optional[int] = ..., new_pid: _Optional[int] = ...) -> None: ...

class ReloadConfigRequest(_message.Message):
    __slots__ = ()
    def __init__(self) -> None: ...

class ReloadConfigReply(_message.Message):
    __slots__ = ("started", "stopped", "reloaded", "kept")
    STARTED_FIELD_NUMBER: _ClassVar[int]
    STOPPED_FIELD_NUMBER: _ClassVar[int]
    RELOADED_FIELD_NUMBER: _ClassVar[int]
    KEPT_FIELD_NUMBER: _ClassVar[int]
    started: _containers.RepeatedScalarFieldContainer[str]
    stopped: _containers.RepeatedScalarFieldContainer[str]
    reloaded: _containers.RepeatedScalarFieldContainer[str]
    kept: _containers.RepeatedScalarFieldContainer[str]
    def __init__(self, started: _Optional[_Iterable[str]] = ..., stopped: _Optional[_Iterable[str]] = ..., reloaded: _Optional[_Iterable[str]] = ..., kept: _Optional[_Iterable[str]] = ...) -> None: ...

class TokenRequest(_message.Message):
    __slots__ = ("name", "call", "provide", "emit", "subscribe", "control", "pane", "origin", "ttl_ms")
    NAME_FIELD_NUMBER: _ClassVar[int]
    CALL_FIELD_NUMBER: _ClassVar[int]
    PROVIDE_FIELD_NUMBER: _ClassVar[int]
    EMIT_FIELD_NUMBER: _ClassVar[int]
    SUBSCRIBE_FIELD_NUMBER: _ClassVar[int]
    CONTROL_FIELD_NUMBER: _ClassVar[int]
    PANE_FIELD_NUMBER: _ClassVar[int]
    ORIGIN_FIELD_NUMBER: _ClassVar[int]
    TTL_MS_FIELD_NUMBER: _ClassVar[int]
    name: str
    call: _containers.RepeatedScalarFieldContainer[str]
    provide: _containers.RepeatedScalarFieldContainer[str]
    emit: _containers.RepeatedScalarFieldContainer[str]
    subscribe: _containers.RepeatedScalarFieldContainer[str]
    control: _containers.RepeatedScalarFieldContainer[str]
    pane: _containers.RepeatedScalarFieldContainer[str]
    origin: str
    ttl_ms: int
    def __init__(self, name: _Optional[str] = ..., call: _Optional[_Iterable[str]] = ..., provide: _Optional[_Iterable[str]] = ..., emit: _Optional[_Iterable[str]] = ..., subscribe: _Optional[_Iterable[str]] = ..., control: _Optional[_Iterable[str]] = ..., pane: _Optional[_Iterable[str]] = ..., origin: _Optional[str] = ..., ttl_ms: _Optional[int] = ...) -> None: ...

class TokenReply(_message.Message):
    __slots__ = ("credential", "expires_at_ms", "revocation_id")
    CREDENTIAL_FIELD_NUMBER: _ClassVar[int]
    EXPIRES_AT_MS_FIELD_NUMBER: _ClassVar[int]
    REVOCATION_ID_FIELD_NUMBER: _ClassVar[int]
    credential: bytes
    expires_at_ms: int
    revocation_id: str
    def __init__(self, credential: _Optional[bytes] = ..., expires_at_ms: _Optional[int] = ..., revocation_id: _Optional[str] = ...) -> None: ...

class RenewRequest(_message.Message):
    __slots__ = ("credential",)
    CREDENTIAL_FIELD_NUMBER: _ClassVar[int]
    credential: bytes
    def __init__(self, credential: _Optional[bytes] = ...) -> None: ...

class RenewReply(_message.Message):
    __slots__ = ("credential", "expires_at_ms", "revocation_id")
    CREDENTIAL_FIELD_NUMBER: _ClassVar[int]
    EXPIRES_AT_MS_FIELD_NUMBER: _ClassVar[int]
    REVOCATION_ID_FIELD_NUMBER: _ClassVar[int]
    credential: bytes
    expires_at_ms: int
    revocation_id: str
    def __init__(self, credential: _Optional[bytes] = ..., expires_at_ms: _Optional[int] = ..., revocation_id: _Optional[str] = ...) -> None: ...

class RevokeRequest(_message.Message):
    __slots__ = ("credential", "revocation_id")
    CREDENTIAL_FIELD_NUMBER: _ClassVar[int]
    REVOCATION_ID_FIELD_NUMBER: _ClassVar[int]
    credential: bytes
    revocation_id: str
    def __init__(self, credential: _Optional[bytes] = ..., revocation_id: _Optional[str] = ...) -> None: ...

class RevokeReply(_message.Message):
    __slots__ = ()
    def __init__(self) -> None: ...

class RotateKeyRequest(_message.Message):
    __slots__ = ()
    def __init__(self) -> None: ...

class RotateKeyReply(_message.Message):
    __slots__ = ()
    def __init__(self) -> None: ...
