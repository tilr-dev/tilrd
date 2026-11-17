//! `server.*` (T2): `server.info`, `server.reload`, `server.update`,
//! `server.command`.
//!
//! Owner: `bus-core-services`. A stub of `bus-broker-core` until then.

use crate::bus::core::{Core, Registry};

/// Add `server.*`; nothing yet.
pub fn register(_registry: &mut Registry, _core: &Core) {}
