//! The pane rights of T7: `pane.create` by profile and override, pane access by
//! creator (`AuthzRequest::PaneCreate`, `AuthzRequest::PaneAccess`).
//!
//! Owner: `bus-pane-rights`. A stub of `bus-broker-core` until then: the stub
//! authorizer allows every pane request.

use crate::bus::core::{Core, Registry};

/// Add what the pane rights need; nothing yet.
pub fn register(_registry: &mut Registry, _core: &Core) {}
