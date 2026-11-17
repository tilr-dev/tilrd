//! `host.open`, called by tilrd on the watchers of a pane that provide it (T6),
//! feature `host-events`.
//!
//! Owner: `bus-core-services`. A stub of `bus-broker-core` until then.

use crate::bus::core::{Core, Registry};

/// Add what the host events need; nothing yet.
pub fn register(_registry: &mut Registry, _core: &Core) {}
