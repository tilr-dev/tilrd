//! The bus broker, protocol `tilr.bus.v1` (`crates/tilr-bus`; shared design
//! `openspec/design/unified-bus.md`, change `bus-broker-core`).
//!
//! Every participant talks to the broker: tilrd's own core (the native participant
//! `tilrd`, [`core`]), the owner and outside participants over WebSocket
//! ([`transport`]), instances over a stream socket ([`BrokerHandle::attach_stream`]).
//! The broker routes calls by target or method, sets `from`, keeps the order of the
//! calls between two connections, relays streams and delivers events to the
//! subscribers connected now ([`broker`], [`streams`], [`events`]). Who may do what
//! is the [`authz::Authorizer`]'s.
//!
//! This module is declared inside `supervisor` (`supervisor.rs`, `mod bus`) and
//! re-exported as `crate::bus`: the core's methods in [`core`] run closures on the
//! supervisor's own task ([`core::SupervisorHandle`]) and use its private fields and
//! paths, so a lane adds its code in its own file and not in `supervisor.rs`.
//!
//! Module tree and owners (change `bus-broker-core`, design "Module tree"): only the
//! owner edits a file; this file and every `mod` line under `bus/` are
//! `bus-broker-core`'s.
//!
//! # Rules for the changes built on this one
//!
//! - **Free functions, not methods.** A lane reaches the supervisor with free
//!   functions in its own module, `fn x(s: &mut Supervisor, …)` (or `&Supervisor`),
//!   never `impl Supervisor { … }` blocks: methods of parallel lanes share one
//!   namespace and would collide. Only `bus/core/mod.rs` (`on_core`) is an exception.
//! - **Matches with a catch-all.** Outside the module that owns it, a `match` on
//!   [`core::CoreCmd`], [`core::PaneEvent`] or [`authz::AuthzRequest`] ends with
//!   `_ => …`: another lane may append a variant.
//! - **`broker.rs` and `transport.rs`** are edited after this change only by
//!   `bus-streams-events`, then by `bus-reload`, strictly one after the other.
//!   Every other lane uses their public API ([`BrokerHandle`], the [`authz::Authorizer`],
//!   [`core::Registry`], [`core::BusObserver`]) and does not open them; a hook it
//!   misses is asked of the change that owns the file at that time.

// Remove once the lanes use the extension points and `tilr.v1` is gone (change
// `remove-tilr-v1`): then what is still unused is dead.
#![allow(
    dead_code,
    reason = "the extension points of bus-broker-core are frozen for the later changes, which use them; removed with remove-tilr-v1"
)]

/// Biscuit tokens and `bus.token`, `bus.renew`, `bus.revoke`, `bus.rotate_key`
/// (`bus-rights`; `bus-web`: origin rules).
pub mod acl;
/// `Authorizer`, `AuthzRequest`, the stub (`bus-broker-core`; `bus-rights`: the body
/// of `default_authorizer`).
pub mod authz;
/// Routing table, connections, the native participant's dispatch (`bus-broker-core`,
/// then `bus-streams-events`, then `bus-reload`).
pub mod broker;
/// `bus.toml` (`bus-instances`).
pub mod config;
/// The `bus.*` control methods (`bus-instances`).
pub mod control;
/// The native participant `tilrd`: its method registry and the core's namespaces.
pub mod core;
/// Subscriptions and event delivery (`bus-broker-core`, then `bus-streams-events`).
pub mod events;
/// `Identity`, `Rights`, `Principal`, `CallCtx` (`bus-broker-core`).
pub mod identity;
/// Spawning and supervising instances (`bus-instances`).
pub mod runner;
/// `bus-state 1`, what crosses a reload (`bus-reload`).
pub mod state;
/// Pending calls and streams; the provider side of a native stream (`bus-broker-core`,
/// then `bus-streams-events`: credits, lanes, deadlines).
pub mod streams;
/// WebSocket and stream-socket transports, Hello (`bus-broker-core`, then
/// `bus-streams-events`, then `bus-reload`; the Origin is the authorizer's).
pub mod transport;

/// The names the later changes use, at `crate::bus::`.
#[allow(
    unused_imports,
    reason = "re-exports for the later changes, which use them"
)]
mod names {
    pub use super::authz::{
        Authorizer, AuthzConfig, AuthzRequest, Denied, HelloContext, default_authorizer,
    };
    pub use super::broker::{Attached, BrokerHandle, ConnId, ConnInfo, ConnState, Ended};
    pub use super::core::{BusObserver, Core, CoreCmd, PaneEvent, Registry, SupervisorHandle};
    pub use super::identity::{
        CallCtx, CancelToken, Identity, Peer, Principal, Rights, Seat, Transport,
    };
    pub use super::streams::{Chunk, NativeHandler, NativeMethod, StreamOut, TrySendError};
}
#[allow(
    unused_imports,
    reason = "re-exports for the later changes, which use them"
)]
pub use names::*;
