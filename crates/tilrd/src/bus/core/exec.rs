//! `exec.*` (T2), feature `exec`.
//!
//! Owner: `bus-core-services`. A stub of `bus-broker-core` until then.

use crate::bus::core::{Core, Registry};

/// Add `exec.*` when `core.features.exec`; nothing yet.
pub fn register(_registry: &mut Registry, _core: &Core) {}
