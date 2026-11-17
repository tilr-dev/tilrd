//! `pane.*` (T2, T4, T5, T9): create, list, close, resize, input, output, read, with
//! `PaneInfo.created_by` stamped; the pane events.
//!
//! Owner: `bus-core-panes`. A stub of `bus-broker-core` until then: every `pane.*`
//! method is `not_found`.

use crate::bus::core::{Core, Registry};

/// Add `pane.*` and the observer of the pane events; nothing yet.
pub fn register(_registry: &mut Registry, _core: &Core) {}
