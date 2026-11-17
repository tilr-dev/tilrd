//! `clipboard.*` and the event `pane.copied` (T6), feature `clipboard`.
//!
//! Owner: `bus-core-services`. A stub of `bus-broker-core` until then.

use crate::bus::core::{Core, Registry};

/// Add `clipboard.*` when `core.features.clipboard`; nothing yet.
pub fn register(_registry: &mut Registry, _core: &Core) {}
