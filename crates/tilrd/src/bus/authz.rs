//! Who may do what on the bus (design R2-R7; change `bus-broker-core`, "Traits").
//!
//! Owner: `bus-broker-core`; `bus-rights` replaces the body of
//! [`default_authorizer`] (and adds its own types in `acl.rs`), nothing else.
//!
//! [`default_authorizer`] is Biscuit's ([`super::acl::Acl`]). The stub authorizer of
//! `bus-broker-core` ([`StubAuthorizer`]) stays for its tests: it allows `tilrd` and
//! `owner` everything, an instance what its [`Rights`] name as plain patterns (exact,
//! `ns.*` prefix, `*` without `bus.*`; `control` as `action[:instance]`), every pane
//! request, and refuses every credential at `Hello`: fail closed.

use std::path::PathBuf;
use std::sync::Arc;

use bytes::Bytes;
use tilr_bus::bus::{ConnectionError, Hello};
use tilr_bus::codes;

use super::identity::{Identity, Peer, Principal, Rights, Seat, Transport};

/// What is to be authorized, with the facts the broker supplies (R6).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthzRequest<'a> {
    /// A call or a stream (a stream is a call); `target` empty when routed by method.
    Call { method: &'a str, target: &'a str },
    /// Providing a method in `Hello`.
    Provide { method: &'a str },
    /// Emitting an event.
    Emit { subject: &'a str },
    /// Subscribing to a pattern.
    Subscribe { pattern: &'a str },
    /// A `bus.*` control action, on an instance or on all (`None`).
    Control {
        action: &'a str,
        instance: Option<&'a str>,
    },
    /// Delivering an event to this subscriber; `scope` is `pane:<id>` or empty.
    Event { subject: &'a str, scope: &'a str },
    /// `pane.create` with a profile (empty: none) and whether `command` or `cwd`
    /// override it (T7).
    PaneCreate { profile: &'a str, overrides: bool },
    /// Any access to a pane made by `creator` (T7).
    PaneAccess { pane_id: &'a str, creator: &'a str },
}

/// A refusal: the `denied` message, naming the missing right.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Denied {
    pub reason: String,
}

impl Denied {
    pub fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
        }
    }
}

impl std::fmt::Display for Denied {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.reason)
    }
}

impl From<Denied> for tilr_bus::BusError {
    fn from(denied: Denied) -> Self {
        tilr_bus::BusError::new(codes::DENIED, denied.reason)
    }
}

/// What the authorizer is told of a connection saying `Hello`.
pub struct HelloContext<'a> {
    pub transport: Transport,
    pub peer: Peer,
    /// The WebSocket `Origin`; empty without.
    pub origin: &'a str,
    /// The broker's id of the connection (`HelloAck.connection_id`).
    pub connection_id: u64,
    /// An instance's seat, when its runner attached the stream.
    pub seat: Option<&'a Seat>,
}

/// The broker's authorization; one per server, shared by every connection.
pub trait Authorizer: Send + Sync {
    /// Who the connection saying `hello` is; a refusal is sent as
    /// `ConnectionError` (code `denied`) and the connection closed. An instance's
    /// seat check ([`Seat::check`]) has passed when this is called.
    fn hello(&self, hello: &Hello, conn: &HelloContext<'_>) -> Result<Principal, ConnectionError>;

    /// Whether `principal` may do `request`; `call_credential` (R4), when given,
    /// authorizes a call alone. Any error is a denial (fail closed).
    fn authorize(
        &self,
        principal: &Principal,
        call_credential: Option<&[u8]>,
        request: &AuthzRequest<'_>,
    ) -> Result<(), Denied>;

    /// `HelloAck.credential`: the token minted for an instance at its start.
    fn hello_ack_credential(&self, principal: &Principal) -> Option<Bytes>;

    /// A `CredentialUpdate` (renewal, R5): the connection's principal from now on.
    /// A refusal is sent as `ConnectionError` and the connection closed (fail
    /// closed). Before `bus-rights` no credential is accepted.
    fn credential_update(
        &self,
        principal: &Principal,
        credential: &[u8],
    ) -> Result<Principal, ConnectionError> {
        let _ = (principal, credential);
        Err(denied("tokens are not supported by this build"))
    }
}

/// What the authorizer is made from.
#[derive(Clone, Debug, Default)]
pub struct AuthzConfig {
    /// The access token the owner shows on TCP; `None`: the server asks for none.
    pub access_token: Option<String>,
    /// `server.info.server_id`, the `server(...)` fact of R2.
    pub server_id: String,
    /// Where `bus-rights` keeps `bus-root.key` and `bus-revoked`.
    pub state_dir: Option<PathBuf>,
}

/// The server's authorizer: Biscuit's (`bus-rights`, `acl.rs`).
pub fn default_authorizer(cfg: &AuthzConfig) -> Arc<dyn Authorizer> {
    super::acl::authorizer(cfg)
}

/// The authorizer before `bus-rights` (module docs); kept for its tests.
pub struct StubAuthorizer {
    access_token: Option<String>,
}

fn denied(message: impl Into<String>) -> ConnectionError {
    ConnectionError {
        code: codes::DENIED.into(),
        message: message.into(),
    }
}

impl Authorizer for StubAuthorizer {
    fn hello(&self, hello: &Hello, conn: &HelloContext<'_>) -> Result<Principal, ConnectionError> {
        if !hello.credential.is_empty() {
            return Err(denied("tokens are not supported by this build"));
        }
        let (identity, rights) = match conn.seat {
            Some(seat) => (seat.identity.clone(), seat.rights.clone()),
            None if !hello.instance_token.is_empty() => {
                return Err(denied(
                    "an instance token is taken on an instance socket only",
                ));
            }
            None => match (conn.transport, &self.access_token) {
                // The Unix socket is this user's alone: no token there.
                (Transport::UnixSocket, _) | (Transport::Tcp, None) => (Identity::Owner, None),
                (Transport::Tcp, Some(_)) if hello.access_token.is_empty() => {
                    return Err(denied("this server asks for its access token"));
                }
                (Transport::Tcp, Some(token)) => {
                    if !crate::auth::matches(token, &hello.access_token) {
                        return Err(denied("wrong access token"));
                    }
                    (Identity::Owner, None)
                }
                (Transport::InstanceFd | Transport::InstanceSocket, _) => {
                    return Err(denied("an instance stream needs its seat"));
                }
            },
        };
        Ok(Principal {
            identity,
            rights,
            credential: None,
            origin: conn.origin.to_owned(),
            transport: conn.transport,
            client_id: hello.client_id.clone(),
            client_name: hello.client_name.clone(),
            connection_id: conn.connection_id,
        })
    }

    fn authorize(
        &self,
        principal: &Principal,
        call_credential: Option<&[u8]>,
        request: &AuthzRequest<'_>,
    ) -> Result<(), Denied> {
        if call_credential.is_some_and(|credential| !credential.is_empty()) {
            return Err(Denied::new("tokens are not supported by this build"));
        }
        let who = &principal.identity;
        if who.is_privileged() {
            return Ok(());
        }
        let Identity::Instance(_) = who else {
            return Err(Denied::new(format!("{who} has no rights in this build")));
        };
        let none = Rights::default();
        let rights = principal.rights.as_deref().unwrap_or(&none);
        let (allowed, what) = match *request {
            AuthzRequest::Call { method, .. } => {
                (name_allowed(&rights.call, method), format!("call {method}"))
            }
            AuthzRequest::Provide { method } => (
                name_allowed(&rights.provide, method),
                format!("provide {method}"),
            ),
            AuthzRequest::Emit { subject } => (
                name_allowed(&rights.emit, subject),
                format!("emit {subject}"),
            ),
            AuthzRequest::Subscribe { pattern } => (
                pattern_allowed(&rights.subscribe, pattern),
                format!("subscribe to {pattern}"),
            ),
            AuthzRequest::Control { action, instance } => (
                control_allowed(&rights.control, action, instance),
                match instance {
                    Some(instance) => format!("control {action}:{instance}"),
                    None => format!("control {action}"),
                },
            ),
            // Per event and pane rights come with bus-rights and bus-pane-rights.
            AuthzRequest::Event { .. }
            | AuthzRequest::PaneCreate { .. }
            | AuthzRequest::PaneAccess { .. } => (true, String::new()),
        };
        if allowed {
            Ok(())
        } else {
            Err(Denied::new(format!("{who} has no right to {what}")))
        }
    }

    fn hello_ack_credential(&self, _: &Principal) -> Option<Bytes> {
        None
    }
}

/// A method or subject against `call`, `provide` or `emit` rights: exact, `ns.*`
/// (anything under `ns.`), `*` (anything but `bus.*`).
fn name_allowed(rights: &[String], name: &str) -> bool {
    rights.iter().any(|right| match right.as_str() {
        "*" => tilr_bus::names::namespace(name) != "bus",
        right => match right.strip_suffix('*') {
            Some(prefix) if prefix.ends_with('.') => name.starts_with(prefix),
            _ => right == name,
        },
    })
}

/// A subscription pattern against `subscribe` rights: one equal to a right, or
/// inside a right's prefix (`pane.*` allows `pane.*`, `pane.bell`, `pane.>`), or
/// anything under `*`.
fn pattern_allowed(rights: &[String], pattern: &str) -> bool {
    rights.iter().any(|right| match right.as_str() {
        "*" => true,
        right => match right.strip_suffix('*') {
            Some(prefix) if prefix.ends_with('.') => {
                pattern.starts_with(prefix) && pattern.len() > prefix.len()
            }
            _ => right == pattern,
        },
    })
}

/// A control action against `control` rights `action` (on every instance) or
/// `action:<instance>` (`t-*` for a prefix); `*` as the action allows them all.
fn control_allowed(rights: &[String], action: &str, instance: Option<&str>) -> bool {
    rights.iter().any(|right| {
        let (right_action, right_instance) = match right.split_once(':') {
            Some((a, i)) => (a, Some(i)),
            None => (right.as_str(), None),
        };
        if right_action != "*" && right_action != action {
            return false;
        }
        match (right_instance, instance) {
            (None, _) => true,
            (Some(_), None) => false,
            (Some(pattern), Some(instance)) => match pattern.strip_suffix('*') {
                Some(prefix) => instance.starts_with(prefix),
                None => pattern == instance,
            },
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn instance(rights: Rights) -> Principal {
        Principal {
            identity: Identity::Instance("t-1".into()),
            rights: Some(Arc::new(rights)),
            ..Principal::tilrd()
        }
    }

    fn strings(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    fn stub(cfg: &AuthzConfig) -> StubAuthorizer {
        StubAuthorizer {
            access_token: cfg.access_token.clone(),
        }
    }

    #[test]
    fn the_stub_allows_what_the_rights_name() {
        let authz = stub(&AuthzConfig::default());
        let p = instance(Rights {
            call: strings(&["math.*", "pane.read", "*"]),
            provide: strings(&["ui.*"]),
            emit: strings(&["deno.*"]),
            subscribe: strings(&["pane.*"]),
            control: strings(&["list", "reload:t-*"]),
            pane: strings(&["profile:agent"]),
        });
        let ok = |request: AuthzRequest<'_>| authz.authorize(&p, None, &request).is_ok();
        assert!(ok(AuthzRequest::Call {
            method: "math.add",
            target: ""
        }));
        assert!(ok(AuthzRequest::Call {
            method: "math.x.y",
            target: ""
        }));
        assert!(ok(AuthzRequest::Call {
            method: "pane.read",
            target: ""
        }));
        // `*` is everything but bus.*.
        assert!(ok(AuthzRequest::Call {
            method: "other.x",
            target: ""
        }));
        assert!(!ok(AuthzRequest::Call {
            method: "bus.stop",
            target: ""
        }));
        assert!(ok(AuthzRequest::Provide {
            method: "ui.notify"
        }));
        assert!(!ok(AuthzRequest::Provide { method: "math.add" }));
        assert!(ok(AuthzRequest::Emit {
            subject: "deno.ready"
        }));
        assert!(!ok(AuthzRequest::Emit {
            subject: "denox.ready"
        }));
        assert!(ok(AuthzRequest::Subscribe { pattern: "pane.*" }));
        assert!(ok(AuthzRequest::Subscribe {
            pattern: "pane.bell"
        }));
        assert!(ok(AuthzRequest::Subscribe { pattern: "pane.>" }));
        assert!(!ok(AuthzRequest::Subscribe { pattern: "*.>" }));
        assert!(ok(AuthzRequest::Control {
            action: "list",
            instance: None
        }));
        assert!(ok(AuthzRequest::Control {
            action: "reload",
            instance: Some("t-2")
        }));
        assert!(!ok(AuthzRequest::Control {
            action: "reload",
            instance: Some("u-1")
        }));
        assert!(!ok(AuthzRequest::Control {
            action: "reload",
            instance: None
        }));
        assert!(!ok(AuthzRequest::Control {
            action: "stop",
            instance: Some("t-2")
        }));
        // Pane rights are bus-pane-rights'.
        assert!(ok(AuthzRequest::PaneCreate {
            profile: "",
            overrides: true
        }));

        let denied = authz
            .authorize(&p, None, &AuthzRequest::Provide { method: "pane.x" })
            .unwrap_err();
        assert_eq!(denied.reason, "instance:t-1 has no right to provide pane.x");

        // Without rights an instance may do nothing; owner and tilrd everything.
        let bare = instance(Rights::default());
        assert!(
            authz
                .authorize(
                    &bare,
                    None,
                    &AuthzRequest::Call {
                        method: "math.add",
                        target: ""
                    }
                )
                .is_err()
        );
        for privileged in [Identity::Owner, Identity::Tilrd] {
            let p = Principal {
                identity: privileged,
                ..Principal::tilrd()
            };
            assert!(
                authz
                    .authorize(
                        &p,
                        None,
                        &AuthzRequest::Call {
                            method: "bus.stop",
                            target: ""
                        }
                    )
                    .is_ok()
            );
            // A per-call credential cannot be verified: fail closed.
            assert!(
                authz
                    .authorize(
                        &p,
                        Some(b"x"),
                        &AuthzRequest::Call {
                            method: "a.b",
                            target: ""
                        }
                    )
                    .is_err()
            );
        }
    }

    #[test]
    fn the_stub_refuses_every_credential_at_hello() {
        let authz = stub(&AuthzConfig {
            access_token: Some("secret".into()),
            ..AuthzConfig::default()
        });
        let conn = |transport| HelloContext {
            transport,
            peer: Peer::Unix { pid: None },
            origin: "",
            connection_id: 9,
            seat: None,
        };
        let hello = |token: &str, credential: &[u8]| Hello {
            protocol_version: 1,
            access_token: token.into(),
            credential: Bytes::copy_from_slice(credential),
            ..Hello::default()
        };
        let refused =
            |hello: Hello, transport| authz.hello(&hello, &conn(transport)).unwrap_err().message;
        assert_eq!(
            refused(hello("", b"token"), Transport::UnixSocket),
            "tokens are not supported by this build"
        );
        assert_eq!(
            refused(hello("", b""), Transport::Tcp),
            "this server asks for its access token"
        );
        assert_eq!(
            refused(hello("nope", b""), Transport::Tcp),
            "wrong access token"
        );
        let owner = authz
            .hello(&hello("secret", b""), &conn(Transport::Tcp))
            .unwrap();
        assert_eq!(owner.identity, Identity::Owner);
        assert_eq!(owner.connection_id, 9);
        let local = authz
            .hello(&hello("", b""), &conn(Transport::UnixSocket))
            .unwrap();
        assert_eq!(local.identity, Identity::Owner);
    }
}
