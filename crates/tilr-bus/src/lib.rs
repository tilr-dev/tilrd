//! The tilr bus, protocol `tilr.bus.v1`.
//!
//! - [`bus`]: the envelopes (`proto/bus.proto`) and the payloads of the broker's own
//!   `bus.*` methods (`proto/tilr/bus/v1/`).
//! - [`pane`], [`server`], [`fs`], [`exec`], [`clipboard`], [`host`], [`agent`]: the
//!   payload schemas of the core's methods, events and streams
//!   (`proto/tilr/<ns>/v1/`); [`methods`] names them.
//! - [`codec`]: framing for WebSocket messages and stream sockets, the 16 MiB limit.
//! - [`names`]: method, subject and pattern syntax and matching.
//! - [`client`]: the async client ([`Bus`]).
//! - [`codes`], [`close`]: error codes and WebSocket close codes.
//!
//! No server serves the bus yet; nothing in `tilrd` or `tilr` uses this crate.
//!
//! Framing: a WebSocket message is a flag byte (`0` plain; `1` zstd, for an
//! envelope of 256 bytes or more that compresses, never for a chunk whose payload
//! its producer compressed) and one envelope; a stream socket carries a u32
//! big-endian length and one envelope; 16 MiB per envelope both ways. Types taken
//! from `tilr.proto` keep their field numbers, so a `tilr.v1` `Frame` decodes as a
//! `tilr.pane.v1` `Frame`; the schemas are frozen (fields are added, never
//! renumbered).
//!
//! The client connects with [`Bus::connect`] over an inherited descriptor
//! (`TILR_BUS_FD`), an instance socket (`TILR_BUS_SOCK`, `TILR_BUS_TOKEN`) or a
//! WebSocket (TCP with the access token or a credential, or the server's Unix
//! socket), as [`Transport::from_env`] reads them. Methods are provided in
//! [`ConnectOptions`] and answered by their handlers, one task per call.
//!
//! `examples/test_runtime.rs` is the participant the bus tests start (`test.echo`,
//! `test.count`, ...; its header lists them). It is built with the tests
//! (`cargo build -p tilr-bus --examples`) and is not part of a release.

pub mod client;
pub mod codec;
/// `Credential::attenuate` (`bus-rights`; feature `attenuate`).
#[cfg(feature = "attenuate")]
mod credential;
pub mod methods;
pub mod names;

pub use client::{
    Bus, BusError, CallOptions, Chunk, ChunkStream, CloseInfo, ConnectOptions, Credential, Event,
    Handler, Reply, Request, StreamOptions, StreamWriter, Subscription, Transport,
};

/// Generated code, laid out as the protobuf packages.
pub mod proto {
    pub mod tilr {
        pub mod bus {
            pub mod v1 {
                include!(concat!(env!("OUT_DIR"), "/tilr.bus.v1.rs"));
            }
        }
        pub mod pane {
            pub mod v1 {
                include!(concat!(env!("OUT_DIR"), "/tilr.pane.v1.rs"));

                /// Modifier bits used in [`KeyInput::mods`] and [`MouseInput::mods`].
                pub mod mods {
                    pub const SHIFT: u32 = 1;
                    pub const ALT: u32 = 2;
                    pub const CTRL: u32 = 4;
                    pub const SUPER: u32 = 8;
                }

                /// Attribute bits used in [`Style::attrs`].
                pub mod attrs {
                    pub const BOLD: u32 = 1;
                    pub const ITALIC: u32 = 2;
                    pub const FAINT: u32 = 4;
                    pub const BLINK: u32 = 8;
                    pub const INVERSE: u32 = 16;
                    pub const INVISIBLE: u32 = 32;
                    pub const STRIKETHROUGH: u32 = 64;
                    pub const OVERLINE: u32 = 128;
                }
            }
        }
        pub mod server {
            pub mod v1 {
                include!(concat!(env!("OUT_DIR"), "/tilr.server.v1.rs"));
            }
        }
        pub mod fs {
            pub mod v1 {
                include!(concat!(env!("OUT_DIR"), "/tilr.fs.v1.rs"));
            }
        }
        pub mod exec {
            pub mod v1 {
                include!(concat!(env!("OUT_DIR"), "/tilr.exec.v1.rs"));
            }
        }
        pub mod clipboard {
            pub mod v1 {
                include!(concat!(env!("OUT_DIR"), "/tilr.clipboard.v1.rs"));
            }
        }
        pub mod host {
            pub mod v1 {
                include!(concat!(env!("OUT_DIR"), "/tilr.host.v1.rs"));
            }
        }
        pub mod agent {
            pub mod v1 {
                include!(concat!(env!("OUT_DIR"), "/tilr.agent.v1.rs"));
            }
        }
    }
}

pub use proto::tilr::agent::v1 as agent;
/// `tilr.bus.v1`: envelopes and the `bus.*` payloads.
pub use proto::tilr::bus::v1 as bus;
pub use proto::tilr::clipboard::v1 as clipboard;
pub use proto::tilr::exec::v1 as exec;
pub use proto::tilr::fs::v1 as fs;
pub use proto::tilr::host::v1 as host;
pub use proto::tilr::pane::v1 as pane;
pub use proto::tilr::server::v1 as server;

/// The version of `tilr.bus.v1` this crate speaks (`Hello.protocol_version`).
pub const PROTOCOL_VERSION: u32 = 1;
/// The WebSocket subprotocol (`Sec-WebSocket-Protocol`).
pub const SUBPROTOCOL: &str = "tilr.bus.v1";

/// `content_type` of protobuf payloads; an empty `content_type` means it too.
pub const CONTENT_TYPE_PROTOBUF: &str = "application/x-protobuf";
/// `content_type` of proto3 canonical JSON payloads; the core answers JSON to JSON.
pub const CONTENT_TYPE_JSON: &str = "application/json";

/// Limits and defaults the broker announces in `HelloAck` (design B5, B7, B8).
pub mod limits {
    /// `HelloAck.max_frame`.
    pub const MAX_FRAME: u32 = crate::codec::MAX_FRAME as u32;
    /// `HelloAck.max_in_flight`: calls and streams pending per connection.
    pub const MAX_IN_FLIGHT: u32 = 256;
    /// `HelloAck.stream_window`: a stream's credit when `Call.window` is 0.
    pub const STREAM_WINDOW: u32 = 1 << 20;
    /// Largest `Call.window`.
    pub const MAX_STREAM_WINDOW: u32 = 16 << 20;
    /// `HelloAck.connection_window`: uncredited bulk bytes to one connection.
    pub const CONNECTION_WINDOW: u32 = 1 << 20;
    /// A call's deadline when `Call.timeout_ms` is 0 (bus.toml `call_timeout_ms`).
    pub const CALL_TIMEOUT_MS: u32 = 10_000;
    /// The window clients open `pane.output` with (frames with images).
    pub const PANE_OUTPUT_WINDOW: u32 = 4 << 20;
}

/// `Error.code` and `ConnectionError.code` values (design B5, the `component-bus`
/// and `protocol` specs). The set is closed for the broker and open for handlers.
pub mod codes {
    pub const DENIED: &str = "denied";
    pub const NOT_FOUND: &str = "not_found";
    pub const UNAVAILABLE: &str = "unavailable";
    pub const TIMEOUT: &str = "timeout";
    pub const BUSY: &str = "busy";
    pub const TOO_LARGE: &str = "too_large";
    pub const BAD_REQUEST: &str = "bad_request";
    pub const HANDLER_ERROR: &str = "handler_error";
    pub const BAD_REPLY: &str = "bad_reply";
    pub const NOT_SUPPORTED: &str = "not_supported";
    pub const CANCELLED: &str = "cancelled";
    pub const SPAWN_FAILED: &str = "spawn_failed";
    pub const START_FAILED: &str = "start_failed";
    pub const START_TIMEOUT: &str = "start_timeout";
    pub const BAD_CONFIG: &str = "bad_config";

    /// The broker's error codes of calls, in the order design B5 lists them.
    pub const ALL: &[&str] = &[
        DENIED,
        NOT_FOUND,
        UNAVAILABLE,
        TIMEOUT,
        BUSY,
        TOO_LARGE,
        BAD_REQUEST,
        HANDLER_ERROR,
        BAD_REPLY,
        NOT_SUPPORTED,
        CANCELLED,
        SPAWN_FAILED,
        START_FAILED,
        START_TIMEOUT,
        BAD_CONFIG,
    ];

    /// `ConnectionError.code`: something other than `Hello` came first.
    pub const EXPECTED_HELLO: &str = "expected_hello";
    /// `ConnectionError.code`: `Hello.protocol_version` is not this server's.
    pub const UNSUPPORTED_VERSION: &str = "unsupported_version";
    /// `ConnectionError.code`: a `Hello` provides a method of a reserved namespace.
    pub const RESERVED: &str = "reserved";
    /// `ConnectionError.code`: a WebSocket message did not decode; the connection stays.
    pub const BAD_FRAME: &str = "bad_frame";

    /// The `ConnectionError` codes; `denied` (a wrong token or credential) is one too.
    pub const CONNECTION: &[&str] = &[
        EXPECTED_HELLO,
        UNSUPPORTED_VERSION,
        DENIED,
        RESERVED,
        BAD_FRAME,
    ];
}

/// WebSocket close codes the server ends a connection with, besides the standard
/// ones; the close frame's reason is the text given here. Part of version 1.
pub mod close {
    /// A newer connection of the same principal and `Hello.client_id` took over.
    /// The client does not reconnect this one.
    pub const SUPERSEDED: u16 = 4000;
    pub const SUPERSEDED_REASON: &str = "superseded";
    /// The participant did not take what it was sent (1024 envelopes queued).
    pub const TOO_SLOW: u16 = 4001;
    pub const TOO_SLOW_REASON: &str = "too slow";
    /// The participant went silent, or stopped answering the server's pings.
    pub const TIMEOUT: u16 = 4002;
    pub const TIMEOUT_REASON: &str = "timeout";
    /// The connection's token expired, was revoked or rotated away without a
    /// `CredentialUpdate`.
    pub const CREDENTIAL: u16 = 4003;
    pub const CREDENTIAL_REASON: &str = "credential";
    /// The standard code a client of the old protocol (`tilr.v1`) is closed with.
    pub const PROTOCOL_ERROR: u16 = 1002;
}

/// Environment variables an instance is started with (design B10, K2).
pub mod env {
    /// The instance's bus connection is this descriptor (always 3).
    pub const BUS_FD: &str = "TILR_BUS_FD";
    /// The instance socket to connect to, with [`BUS_TOKEN`].
    pub const BUS_SOCK: &str = "TILR_BUS_SOCK";
    /// The one-time token for `Hello.instance_token` on [`BUS_SOCK`].
    pub const BUS_TOKEN: &str = "TILR_BUS_TOKEN";
    /// A WebSocket URL of the bus, for outside participants (with [`BUS_CREDENTIAL`]).
    pub const BUS_URL: &str = "TILR_BUS_URL";
    /// A credential for [`BUS_URL`], base64url as `tilr bus token` prints it.
    pub const BUS_CREDENTIAL: &str = "TILR_BUS_CREDENTIAL";
    /// The instance's name in bus.toml.
    pub const INSTANCE: &str = "TILR_INSTANCE";
    /// The server's configuration directory.
    pub const CONFIG_DIR: &str = "TILR_CONFIG_DIR";
}
