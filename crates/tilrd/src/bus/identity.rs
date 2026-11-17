//! Who is on the bus and what a call carries (design B2, R3; change `bus-broker-core`,
//! frozen contract "Types"). Owner: `bus-broker-core`.

use std::any::Any;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use bytes::Bytes;
use serde::Deserialize;
use tilr_bus::bus::{ConnectionError, Hello};
use tokio::sync::Notify;

/// A participant's identity, set by the broker (B2): the `from` of everything it
/// sends and `HelloAck.identity`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Identity {
    /// tilrd's own core, in process.
    Tilrd,
    /// The owner: the Unix socket (peer uid) or TCP with the access token.
    Owner,
    /// An instance of `bus.toml`, by name.
    Instance(String),
    /// An outside participant with a token, by the token's participant name.
    Token(String),
}

impl Identity {
    /// The text form back: `tilrd`, `owner`, `instance:<name>`, `token:<name>`.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "tilrd" => Some(Self::Tilrd),
            "owner" => Some(Self::Owner),
            _ => {
                let (kind, name) = text.split_once(':')?;
                if name.is_empty() {
                    return None;
                }
                match kind {
                    "instance" => Some(Self::Instance(name.to_owned())),
                    "token" => Some(Self::Token(name.to_owned())),
                    _ => None,
                }
            }
        }
    }

    /// Whether this is the owner or tilrd, which hold every right.
    pub fn is_privileged(&self) -> bool {
        matches!(self, Self::Tilrd | Self::Owner)
    }
}

impl std::fmt::Display for Identity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Tilrd => f.write_str("tilrd"),
            Self::Owner => f.write_str("owner"),
            Self::Instance(name) => write!(f, "instance:{name}"),
            Self::Token(name) => write!(f, "token:{name}"),
        }
    }
}

/// The rights of R3 as written in `[instances.<n>.rights]` (and `tilr bus token`):
/// the strings, parsed by `bus-instances`, minted from by `bus-rights`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rights {
    #[serde(default)]
    pub call: Vec<String>,
    #[serde(default)]
    pub provide: Vec<String>,
    #[serde(default)]
    pub emit: Vec<String>,
    #[serde(default)]
    pub subscribe: Vec<String>,
    #[serde(default)]
    pub control: Vec<String>,
    #[serde(default)]
    pub pane: Vec<String>,
}

/// How a connection reaches the broker.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transport {
    /// A WebSocket on the server's Unix socket.
    UnixSocket,
    /// A WebSocket on the TCP port.
    Tcp,
    /// An instance's descriptor 3 (`TILR_BUS_FD`).
    InstanceFd,
    /// An instance socket (`TILR_BUS_SOCK`).
    InstanceSocket,
}

/// Where a connection comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Peer {
    Tcp(SocketAddr),
    /// The Unix socket; the client's pid, if the system says.
    Unix {
        pid: Option<i32>,
    },
    /// An instance tilrd started.
    Child {
        pid: Option<u32>,
    },
}

impl std::fmt::Display for Peer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Tcp(addr) => write!(f, "{addr}"),
            Self::Unix { pid: Some(pid) } => write!(f, "unix (pid {pid})"),
            Self::Unix { pid: None } => write!(f, "unix"),
            Self::Child { pid: Some(pid) } => write!(f, "child (pid {pid})"),
            Self::Child { pid: None } => write!(f, "child"),
        }
    }
}

/// Who a connection is, as the authorizer made it at `Hello`; shared by every call
/// the connection makes ([`CallCtx::principal`]).
#[derive(Clone)]
pub struct Principal {
    pub identity: Identity,
    /// An instance's rights (R3); `None` for the owner and tilrd, which hold all.
    pub rights: Option<Arc<Rights>>,
    /// The verified token (`bus-rights`); `None` before it.
    pub credential: Option<Arc<dyn Any + Send + Sync>>,
    /// The WebSocket `Origin`; empty without.
    pub origin: String,
    pub transport: Transport,
    pub client_id: String,
    pub client_name: String,
    pub connection_id: u64,
}

impl Principal {
    /// tilrd's core itself.
    pub fn tilrd() -> Self {
        Self {
            identity: Identity::Tilrd,
            rights: None,
            credential: None,
            origin: String::new(),
            transport: Transport::UnixSocket,
            client_id: String::new(),
            client_name: "tilrd".into(),
            connection_id: 0,
        }
    }
}

impl std::fmt::Debug for Principal {
    /// The credential is never shown.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Principal")
            .field("identity", &self.identity)
            .field("rights", &self.rights)
            .field(
                "credential",
                &self.credential.as_ref().map(|_| "<verified>"),
            )
            .field("origin", &self.origin)
            .field("transport", &self.transport)
            .field("client_id", &self.client_id)
            .field("client_name", &self.client_name)
            .field("connection_id", &self.connection_id)
            .finish()
    }
}

/// A check of an instance's `Hello` its runner asks for (the one-time
/// `TILR_BUS_TOKEN` on an instance socket), run before the authorizer.
pub type HelloCheck = Box<dyn FnOnce(&Hello) -> Result<(), ConnectionError> + Send>;

/// What a runner knows of an instance whose stream it attaches
/// ([`crate::bus::BrokerHandle::attach_stream`]): its identity is the descriptor's.
pub struct Seat {
    /// `Identity::Instance(<name>)`.
    pub identity: Identity,
    pub rights: Option<Arc<Rights>>,
    /// `InstanceFd` or `InstanceSocket`.
    pub transport: Transport,
    pub peer: Peer,
    pub check: Option<HelloCheck>,
}

/// Cancellation of one call, shared by the broker and the handler
/// (`tokio_util::sync::CancellationToken`'s surface, without the dependency).
#[derive(Clone, Default)]
pub struct CancelToken(Arc<CancelState>);

#[derive(Default)]
struct CancelState {
    cancelled: AtomicBool,
    notify: Notify,
}

impl CancelToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        if !self.0.cancelled.swap(true, Ordering::SeqCst) {
            self.0.notify.notify_waiters();
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.cancelled.load(Ordering::SeqCst)
    }

    /// Returns once [`cancel`](Self::cancel) was called.
    pub async fn cancelled(&self) {
        loop {
            let notified = self.0.notify.notified();
            if self.is_cancelled() {
                return;
            }
            notified.await;
        }
    }
}

impl std::fmt::Debug for CancelToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("CancelToken")
            .field(&self.is_cancelled())
            .finish()
    }
}

/// One call as a native handler gets it.
#[derive(Clone, Debug)]
pub struct CallCtx {
    /// The caller's id of the call on its connection.
    pub id: u64,
    pub method: String,
    /// `Call.target` as sent; empty when routed by method.
    pub target: String,
    /// The caller's identity, set by the broker.
    pub from: Identity,
    pub principal: Arc<Principal>,
    /// `Call.credential`: authorizes this call alone (R4); `None` when empty.
    pub call_credential: Option<Bytes>,
    /// `None`: no deadline (`bus-streams-events` sets it).
    pub deadline: Option<Instant>,
    /// Cancelled when the caller cancels or its connection ends.
    pub cancel: CancelToken,
    /// Empty means `application/x-protobuf`.
    pub content_type: String,
    pub trace_id: String,
}

impl CallCtx {
    /// Whether the caller asked for JSON (proto3 canonical mapping, B9).
    pub fn wants_json(&self) -> bool {
        self.content_type == tilr_bus::CONTENT_TYPE_JSON
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_identity_reads_back_from_its_text() {
        for identity in [
            Identity::Tilrd,
            Identity::Owner,
            Identity::Instance("t-1".into()),
            Identity::Token("web:p".into()),
        ] {
            assert_eq!(Identity::parse(&identity.to_string()), Some(identity));
        }
        assert_eq!(Identity::parse("instance:"), None);
        assert_eq!(Identity::parse("someone"), None);
    }
}
