//! Names of the core's methods and events (design T2) with their kind and lane,
//! and of the methods the core calls on clients. The payload of each is named in
//! the header of its schema file in `proto/tilr/<ns>/v1/`.

use crate::bus::{Kind, Lane};

/// A method the bus knows by name: its kind and the lane of its answers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Method {
    pub name: &'static str,
    pub kind: Kind,
    pub lane: Lane,
}

const fn unary(name: &'static str) -> Method {
    Method {
        name,
        kind: Kind::Unary,
        lane: Lane::Interactive,
    }
}

const fn stream(name: &'static str, lane: Lane) -> Method {
    Method {
        name,
        kind: Kind::Stream,
        lane,
    }
}

pub const BUS_PING: Method = unary("bus.ping");
pub const BUS_INSTANCES: Method = unary("bus.instances");
pub const BUS_METHODS: Method = unary("bus.methods");
pub const BUS_START: Method = unary("bus.start");
pub const BUS_STOP: Method = unary("bus.stop");
pub const BUS_RELOAD: Method = unary("bus.reload");
pub const BUS_RELOAD_CONFIG: Method = unary("bus.reload_config");
pub const BUS_TOKEN: Method = unary("bus.token");
pub const BUS_RENEW: Method = unary("bus.renew");
pub const BUS_REVOKE: Method = unary("bus.revoke");
pub const BUS_ROTATE_KEY: Method = unary("bus.rotate_key");

pub const PANE_CREATE: Method = unary("pane.create");
pub const PANE_LIST: Method = unary("pane.list");
pub const PANE_CLOSE: Method = unary("pane.close");
pub const PANE_RESIZE: Method = unary("pane.resize");
pub const PANE_INPUT: Method = unary("pane.input");
pub const PANE_APPLIED: Method = unary("pane.applied");
pub const PANE_INPUT_FINALIZE: Method = unary("pane.input_finalize");
pub const PANE_READ: Method = unary("pane.read");
pub const PANE_PROFILES: Method = unary("pane.profiles");
pub const PANE_OUTPUT: Method = stream("pane.output", Lane::Interactive);

pub const SERVER_INFO: Method = unary("server.info");
pub const SERVER_RELOAD: Method = unary("server.reload");
pub const SERVER_UPDATE: Method = stream("server.update", Lane::Bulk);
pub const SERVER_COMMAND: Method = stream("server.command", Lane::Bulk);

pub const FS_LIST: Method = unary("fs.list");
pub const FS_STAT: Method = unary("fs.stat");
pub const FS_READ: Method = stream("fs.read", Lane::Bulk);
pub const FS_WRITE: Method = unary("fs.write");
pub const FS_MKDIR: Method = unary("fs.mkdir");
pub const FS_RENAME: Method = unary("fs.rename");
pub const FS_DELETE: Method = unary("fs.delete");
pub const FS_WATCH: Method = unary("fs.watch");
pub const FS_UNWATCH: Method = unary("fs.unwatch");

pub const EXEC_RUN: Method = stream("exec.run", Lane::Bulk);
pub const EXEC_INPUT: Method = unary("exec.input");
pub const EXEC_SIGNAL: Method = unary("exec.signal");

pub const CLIPBOARD_PUT: Method = unary("clipboard.put");
pub const CLIPBOARD_GET: Method = stream("clipboard.get", Lane::Bulk);

/// Provided by clients, called by tilrd on the watchers of a pane.
pub const HOST_OPEN: Method = unary("host.open");
/// Provided by clients, called by tilrd for a pane's `SSH_AUTH_SOCK`.
pub const SSH_AGENT_REQUEST: Method = unary("ssh_agent.request");
pub const SSH_AGENT_CLOSE: Method = unary("ssh_agent.close");

/// Every method `tilrd` provides with every feature on, in the order of the
/// `bus-terminal` spec, then the broker's own.
pub const CORE: &[Method] = &[
    PANE_CREATE,
    PANE_LIST,
    PANE_CLOSE,
    PANE_RESIZE,
    PANE_INPUT,
    PANE_READ,
    PANE_APPLIED,
    PANE_INPUT_FINALIZE,
    PANE_PROFILES,
    PANE_OUTPUT,
    SERVER_INFO,
    SERVER_RELOAD,
    SERVER_UPDATE,
    SERVER_COMMAND,
    FS_LIST,
    FS_STAT,
    FS_READ,
    FS_WRITE,
    FS_MKDIR,
    FS_RENAME,
    FS_DELETE,
    FS_WATCH,
    FS_UNWATCH,
    EXEC_RUN,
    EXEC_INPUT,
    EXEC_SIGNAL,
    CLIPBOARD_PUT,
    CLIPBOARD_GET,
    BUS_PING,
    BUS_INSTANCES,
    BUS_METHODS,
    BUS_START,
    BUS_STOP,
    BUS_RELOAD,
    BUS_RELOAD_CONFIG,
    BUS_TOKEN,
    BUS_RENEW,
    BUS_REVOKE,
    BUS_ROTATE_KEY,
];

/// Methods clients provide and the core calls.
pub const CLIENT: &[Method] = &[HOST_OPEN, SSH_AGENT_REQUEST, SSH_AGENT_CLOSE];

/// Event subjects the core emits; each with scope `pane:<id>` but `fs.changed.*`.
pub mod events {
    pub const PANE_CREATED: &str = "pane.created";
    pub const PANE_CHANGED: &str = "pane.changed";
    pub const PANE_BELL: &str = "pane.bell";
    pub const PANE_COMMAND_FINISHED: &str = "pane.command_finished";
    pub const PANE_COPIED: &str = "pane.copied";
    pub const PANE_EXITED: &str = "pane.exited";
    /// Prefix of `fs.changed.<watch_id>`.
    pub const FS_CHANGED_PREFIX: &str = "fs.changed.";

    /// The subject of a watch's events.
    pub fn fs_changed(watch_id: u64) -> String {
        format!("{FS_CHANGED_PREFIX}{watch_id}")
    }

    /// The event scope of a pane: `pane:<id>`.
    pub fn pane_scope(pane_id: &str) -> String {
        format!("pane:{pane_id}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::names;

    #[test]
    fn every_core_method_is_a_valid_name_in_a_reserved_namespace() {
        for method in CORE {
            assert!(names::valid_method(method.name), "{}", method.name);
            assert!(names::is_reserved(method.name), "{}", method.name);
        }
        for method in CLIENT {
            assert!(names::valid_method(method.name), "{}", method.name);
            assert!(!names::is_reserved(method.name), "{}", method.name);
        }
        let mut seen = std::collections::HashSet::new();
        assert!(CORE.iter().chain(CLIENT).all(|m| seen.insert(m.name)));
    }

    #[test]
    fn bulk_streams_are_the_ones_design_t3_names() {
        let bulk: Vec<&str> = CORE
            .iter()
            .filter(|m| m.lane == Lane::Bulk)
            .map(|m| m.name)
            .collect();
        assert_eq!(
            bulk,
            [
                "server.update",
                "server.command",
                "fs.read",
                "exec.run",
                "clipboard.get"
            ]
        );
        assert_eq!(PANE_OUTPUT.kind, Kind::Stream);
        assert_eq!(PANE_OUTPUT.lane, Lane::Interactive);
    }

    #[test]
    fn event_subjects_are_valid() {
        for subject in [
            events::PANE_CREATED,
            events::PANE_CHANGED,
            events::PANE_BELL,
            events::PANE_COMMAND_FINISHED,
            events::PANE_COPIED,
            events::PANE_EXITED,
        ] {
            assert!(names::valid_subject(subject), "{subject}");
        }
        assert!(names::valid_subject(&events::fs_changed(42)));
        assert!(names::matches("fs.changed.*", &events::fs_changed(42)));
    }
}
