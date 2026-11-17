//! `ssh_agent.request` and `ssh_agent.close`, called by tilrd on the watcher of a
//! pane that provides it and subscribed last (T6), feature `agent`.
//!
//! Owner: `bus-core-services`. A stub of `bus-broker-core` until then.

use crate::bus::core::{Core, Registry};

/// Add what the SSH agent needs; nothing yet.
pub fn register(_registry: &mut Registry, _core: &Core) {}
