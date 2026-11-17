//! Biscuit tokens (R1-R8): the root key, minting, verification with its caches,
//! revocation, and `bus.token`, `bus.renew`, `bus.revoke`, `bus.rotate_key`;
//! `bus-web` adds the origin rules (W2).
//!
//! Owner: `bus-rights`. A stub of `bus-broker-core` until then (the authorizer is
//! the stub of `authz.rs`).

use crate::bus::core::{Core, Registry};

/// Add the token methods; nothing yet.
pub fn register(_registry: &mut Registry, _core: &Core) {}
