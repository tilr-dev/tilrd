//! The `bus.*` control methods (B10): `bus.instances`, `bus.methods`, `bus.start`,
//! `bus.stop`, `bus.reload`, `bus.reload_config`, serialized in one queue.
//!
//! Owner: `bus-instances`. A stub of `bus-broker-core` until then.

use crate::bus::core::{Core, Registry};

/// Add the control methods; nothing yet.
pub fn register(_registry: &mut Registry, _core: &Core) {}
