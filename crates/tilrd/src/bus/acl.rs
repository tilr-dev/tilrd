//! Biscuit tokens (R1-R8): the root key, minting, verification with its caches,
//! revocation, and `bus.token`, `bus.renew`, `bus.revoke`, `bus.rotate_key`;
//! `bus-web` adds the origin rules (W2).
//!
//! Owner: `bus-rights`.
//!
//! # What is checked where
//!
//! - **The root key** is Ed25519 in `<state dir>/bus-root.key` (hex, 0600), made on
//!   the first start and replaced only by `bus.rotate_key`. A key file that cannot be
//!   read, is not the user's or is open to others makes the server refuse every token
//!   (and mint none): fail closed, with an error in the log.
//! - **`Hello`** ([`Acl::hello`]): an instance's seat gets a token minted from its
//!   rights for this connection (sent in `HelloAck.credential`); a `credential`
//!   makes the participant `token:<name>` of that token once it verifies (signature
//!   of this server's key, at most 16 KiB and 8 blocks, no third-party block, not
//!   revoked, not expired, its origin); no credential is the owner, as before.
//! - **Every request** ([`Acl::authorize`]): `server.command` is the owner's on the
//!   Unix socket only, whatever a token says; otherwise tilrd and the owner hold every
//!   right; a `Call.credential` authorizes its call alone (R4), and so the `Control`
//!   checks a control method makes for that call (it passes `CallCtx::call_credential`);
//!   anything else is decided by the connection's token: first the checks that need
//!   no Datalog (key epoch, revocation of the token and of its minters, the
//!   instance's connection, expiry, origin), then the decision cache (1 s), then a
//!   fresh Biscuit authorizer with the request's facts, the deterministic limits of
//!   1000 facts and 100 iterations (a wall-clock limit of 50 ms is a backstop only,
//!   and a decision it ends is never cached), and the policies of [`POLICIES`], which
//!   read the authority block only: an appended block adds checks, never rights. Any
//!   error is a denial.
//! - **Expiry and renewal**: tokens live at most 24 h. A sweeper renews an instance's
//!   token 12 h after it was minted (the connection's principal is replaced and the
//!   instance gets the new token in a `CredentialUpdate`) and
//!   closes a connection whose token expired with 4003; `bus.renew` gives an outside
//!   participant a fresh token for an unattenuated one, which it presents with
//!   `CredentialUpdate` ([`Acl::credential_update`]), in the second half of its life
//!   and never past the ceiling a minter other than the owner set.
//! - **Revocation**: `bus.revoke` and the end of an instance's connection put the
//!   token's revocation id in a set kept until the token expires, in memory and in
//!   `<state dir>/bus-revoked` (0600, written whole by one writer at a time, never
//!   while the file on disk is broken); connections that said `Hello` with it are
//!   closed with 4003. A token minted by a participant other than the owner names
//!   its minters' revocation ids and dies with them. `bus.rotate_key` makes a new
//!   key: every earlier token is refused from that moment, instances get new tokens,
//!   outside participants are closed with 4003 (the sweeper catches a connection
//!   that said `Hello` during the rotation).
//! - **File I/O** (`bus.revoke`, `bus.renew`, `bus.rotate_key`, an ended instance's
//!   revocation) runs on a blocking thread, never on the envelope path; the methods
//!   answer once the revocation is in memory and on disk.
//!
//! Token bytes and keys are never logged; audit lines name the first 16 hex digits
//! of a token's revocation id.
//!
//! The authorizer reaches the broker (for the methods above, the sweeper, and to see
//! whether an instance's connection is still open) through [`register`], which finds
//! the [`Acl`] the broker was made with by its address in a process-wide list.

use std::any::Any;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, RwLock, Weak};
use std::time::{Duration, Instant, UNIX_EPOCH};

use biscuit_auth::builder::{self as datalog, Fact, Term};
use biscuit_auth::error::{Logic, Token as TokenError};
use biscuit_auth::{
    Algorithm, AuthorizerBuilder, AuthorizerLimits, Biscuit, KeyPair, PrivateKey, UnverifiedBiscuit,
};
use bytes::Bytes;
use prost::Message as _;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tilr_bus::bus::{ConnectionError, Hello};
use tilr_bus::proto::tilr::bus::v1 as wire;
use tilr_bus::{BusError, Reply, codes, names};
use zeroize::Zeroizing;

use super::authz::{Authorizer, AuthzConfig, AuthzRequest, Denied, ForCall, HelloContext};
use super::broker::{BrokerHandle, ConnId};
use super::core::{Core, Registry};
use super::identity::{CallCtx, Identity, Principal, Rights, Transport};
use super::streams::NativeMethod;

/// The root key's file in the state directory.
pub const ROOT_KEY_FILE: &str = "bus-root.key";
/// The revoked tokens' file in the state directory.
pub const REVOKED_FILE: &str = "bus-revoked";
/// The audit log target (R8).
const AUDIT: &str = "tilrd::bus::audit";

/// A token's longest life (R5).
pub const MAX_TTL: Duration = Duration::from_secs(24 * 3600);
/// An instance's token is renewed this long after it was minted (R5).
pub const INSTANCE_RENEWAL: Duration = Duration::from_secs(12 * 3600);
/// The largest token accepted (R7).
pub const MAX_TOKEN_BYTES: usize = 16 * 1024;
/// The most blocks a token may have (R7).
pub const MAX_BLOCKS: usize = 8;
/// The most checks an appended block may hold, counting each `or` alternative.
pub const MAX_BLOCK_CHECKS: usize = 16;
/// The most predicates in the body of one check of an appended block.
const MAX_CHECK_PREDICATES: usize = 16;
/// The most expression operations an appended block may hold, all checks together.
const MAX_BLOCK_OPS: usize = 64;
/// The longest `Call.target` decided: a target names a participant
/// (`token:<name>`, names up to 128 bytes), so a longer one is denied unread.
pub const MAX_TARGET_BYTES: usize = 512;
/// How much of a target an audit line shows.
const AUDIT_TARGET_BYTES: usize = 128;
/// Parsed tokens kept (R7).
const PARSED_CACHE: usize = 1024;
/// How long a decision is reused (R7).
const DECISION_TTL: Duration = Duration::from_secs(1);
/// Decisions kept before the cache is emptied.
const DECISION_CACHE: usize = 8192;
/// How often expiry, renewal and ended instances are looked at.
const SWEEP: Duration = Duration::from_millis(200);
/// An instance token whose connection was never seen open is revoked after this.
const UNSEEN_INSTANCE: Duration = Duration::from_secs(30);
/// How long an instance's token presented on another connection is taken while its
/// own connection is not registered yet: the broker registers it right after it
/// queued `HelloAck` (which carries the token), so only a connection that ended
/// before the sweeper saw it open is older than this and not open.
const UNREGISTERED_GRACE: Duration = Duration::from_secs(1);
/// The most rights facts a token's authority block may hold: with the request's
/// facts it stays under the run limit of 1000 facts.
const MAX_RIGHTS_FACTS: usize = 512;
/// The wall-clock backstop of one decision; the deterministic limits of
/// [`run_limits`] are the real bound (R7).
const MAX_TIME: Duration = Duration::from_millis(50);
/// The control actions (R3, `tilr.bus.v1` `bus.*` header); `server.reload` and
/// `server.update` are their own actions. `server.command` is no right at all: it is
/// the owner's on the Unix socket only.
pub const CONTROL_ACTIONS: &[&str] = &[
    "list",
    "start",
    "stop",
    "reload",
    "reload_config",
    "token",
    "revoke",
    "rotate_key",
    "server.reload",
    "server.update",
];

/// The authorizer's policies (R6): they read the authority block and the request's
/// facts; an appended block's facts are out of their scope. Rights facts:
/// `call(m)`, `call_prefix(p)`, `call_any(true)`, `provide_method(m)`,
/// `provide_prefix(p)`, `provide_any(true)`, `emit_subject(s)`, `emit_prefix(p)`,
/// `emit_any(true)`, `subscribe_pattern(p)`, `subscribe_prefix(p)`,
/// `subscribe_any(true)`, `control(a, i)`, `control_prefix(a, p)`, `control_any(a)`,
/// `control_all(true)`; the pane facts (`pane_profile(n)`, ...) are minted for
/// `bus-pane-rights` and not read here.
pub const POLICIES: &str = r#"
allow if kind("bus");
allow if kind("call"), method($m), call($m);
allow if kind("call"), method($m), call_prefix($p), $m.starts_with($p);
allow if kind("call"), call_any(true);
allow if kind("provide"), method($m), provide_method($m);
allow if kind("provide"), method($m), provide_prefix($p), $m.starts_with($p);
allow if kind("provide"), provide_any(true);
allow if kind("emit"), subject($s), emit_subject($s);
allow if kind("emit"), subject($s), emit_prefix($p), $s.starts_with($p);
allow if kind("emit"), emit_any(true);
allow if kind("subscribe"), pattern($s), subscribe_pattern($s);
allow if kind("subscribe"), pattern($s), subscribe_prefix($p), $s.starts_with($p), $s.length() > $p.length();
allow if kind("subscribe"), subscribe_any(true);
allow if kind("control"), control_all(true);
allow if kind("control"), action($a), control_any($a);
allow if kind("control"), action($a), instance($i), control($a, $i);
allow if kind("control"), action($a), instance($i), control_prefix($a, $p), $i.starts_with($p);
allow if kind("control"), instance($i), control("*", $i);
allow if kind("control"), instance($i), control_prefix("*", $p), $i.starts_with($p);
allow if kind("control_gate"), control_all(true);
allow if kind("control_gate"), action($a), control_any($a);
allow if kind("control_gate"), action($a), control($a, $i);
allow if kind("control_gate"), action($a), control_prefix($a, $p);
allow if kind("control_gate"), control("*", $i);
allow if kind("control_gate"), control_prefix("*", $p);
allow if kind("event");
allow if kind("pane");
deny if true;
"#;

/// The run limits of one decision (R7): 1000 facts and 100 iterations bound the
/// work the same way on every machine; the time limit only stops a run that the
/// machine's load stretches far beyond them, and its denial is not cached.
fn run_limits() -> AuthorizerLimits {
    AuthorizerLimits {
        max_facts: 1000,
        max_iterations: 100,
        max_time: MAX_TIME,
    }
}

// ---------------------------------------------------------------------------
// Tokens
// ---------------------------------------------------------------------------

/// What the authority block's context says (signed with it): read without Datalog.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
struct Meta {
    /// `instance:<name>` or `token:<name>`.
    participant: String,
    /// The server id it was minted by: checked without Datalog too (Hello,
    /// `CredentialUpdate`, `bus.renew`, minting decide without it).
    #[serde(default)]
    server: String,
    minted_ms: u64,
    expires_ms: u64,
    /// The origin it is bound to; empty: none.
    #[serde(default)]
    origin: String,
    rights: RightsText,
    /// Who minted it, when not the owner or tilrd: its lifetime limit and the
    /// revocation ids it dies with.
    #[serde(default)]
    lineage: Lineage,
    /// The authority revocation id of the token it renews, and of all its earlier
    /// renewals: the first of the chain (empty: this token is the first). Revoking
    /// any token of the chain revokes this id, so a renewal never outlives a
    /// revocation of the token it came from; a renewal supersedes its predecessor's
    /// own id only.
    #[serde(default)]
    family: String,
}

/// What a token minted by a participant other than the owner inherits from its
/// minter: it never lives past the minter's token as minted (`ceiling_ms`, also
/// through renewals) and is refused once any of `parents` (the minter's revocation
/// id and its own parents) is revoked.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
struct Lineage {
    /// Unix ms; 0: none.
    #[serde(default)]
    ceiling_ms: u64,
    #[serde(default)]
    parents: Vec<String>,
}

/// [`Rights`] as minted (the strings of R3).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
struct RightsText {
    #[serde(default)]
    call: Vec<String>,
    #[serde(default)]
    provide: Vec<String>,
    #[serde(default)]
    emit: Vec<String>,
    #[serde(default)]
    subscribe: Vec<String>,
    #[serde(default)]
    control: Vec<String>,
    #[serde(default)]
    pane: Vec<String>,
}

impl From<&Rights> for RightsText {
    fn from(rights: &Rights) -> Self {
        Self {
            call: rights.call.clone(),
            provide: rights.provide.clone(),
            emit: rights.emit.clone(),
            subscribe: rights.subscribe.clone(),
            control: rights.control.clone(),
            pane: rights.pane.clone(),
        }
    }
}

impl From<&RightsText> for Rights {
    fn from(rights: &RightsText) -> Self {
        Self {
            call: rights.call.clone(),
            provide: rights.provide.clone(),
            emit: rights.emit.clone(),
            subscribe: rights.subscribe.clone(),
            control: rights.control.clone(),
            pane: rights.pane.clone(),
        }
    }
}

/// A token that verified against this server's key.
struct Parsed {
    biscuit: Biscuit,
    /// SHA-256 of its bytes.
    hash: [u8; 32],
    meta: Meta,
    identity: Identity,
    /// The revocation id of each block, hex; the first is the authority block's.
    rids: Vec<String>,
    /// The key epoch it verified under.
    epoch: u64,
}

impl Parsed {
    fn attenuated(&self) -> bool {
        self.rids.len() > 1
    }

    /// Every revocation id that ends this token: its blocks', its family's and its
    /// minters' (their families).
    fn ending_ids(&self) -> impl Iterator<Item = &String> {
        self.rids
            .iter()
            .chain(self.family_id())
            .chain(self.meta.lineage.parents.iter())
    }

    /// The id of its chain of renewals ([`Meta::family`]): the first token's
    /// authority revocation id; `None` only for a token with no blocks.
    fn family_id(&self) -> Option<&String> {
        if self.meta.family.is_empty() {
            self.rids.first()
        } else {
            Some(&self.meta.family)
        }
    }

    fn family(&self) -> &str {
        self.family_id().map_or("", String::as_str)
    }

    /// The first 16 hex digits of the authority block's revocation id.
    fn rid16(&self) -> &str {
        self.rids
            .first()
            .map_or("", |rid| &rid[..rid.len().min(16)])
    }
}

impl std::fmt::Debug for Parsed {
    /// Never the token.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Parsed")
            .field("identity", &self.identity)
            .field("rid", &self.rid16())
            .field("expires_ms", &self.meta.expires_ms)
            .finish()
    }
}

/// The verified token of a connection: what [`Principal::credential`] holds.
pub struct Token {
    bytes: Bytes,
    parsed: Arc<Parsed>,
}

impl Token {
    /// The token as Biscuit (for `bus-pane-rights`' second authorization).
    pub fn biscuit(&self) -> &Biscuit {
        &self.parsed.biscuit
    }

    /// The participant it names.
    pub fn identity(&self) -> &Identity {
        &self.parsed.identity
    }

    /// The rights it was minted with.
    pub fn rights(&self) -> Rights {
        Rights::from(&self.parsed.meta.rights)
    }

    /// Unix milliseconds.
    pub fn expires_ms(&self) -> u64 {
        self.parsed.meta.expires_ms
    }

    /// The authority block's revocation id, hex.
    pub fn revocation_id(&self) -> &str {
        self.parsed.rids.first().map_or("", String::as_str)
    }
}

impl std::fmt::Debug for Token {
    /// Never the bytes.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Token")
            .field("identity", &self.parsed.identity)
            .field("rid", &self.parsed.rid16())
            .field("expires_ms", &self.parsed.meta.expires_ms)
            .finish()
    }
}

/// The verified token a principal holds, if any.
pub fn token_of(principal: &Principal) -> Option<&Token> {
    principal
        .credential
        .as_ref()
        .and_then(|credential| credential.downcast_ref::<Token>())
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        })
}

/// `text` cut to at most `max` bytes (on a character boundary), for messages and logs.
fn clip(text: &str, max: usize) -> &str {
    if text.len() <= max {
        return text;
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

fn is_hex(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= 256
        && text
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

fn date(ms: u64) -> Term {
    datalog::date(&(UNIX_EPOCH + Duration::from_secs(ms / 1000)))
}

fn fact(name: &str, terms: &[Term]) -> Fact {
    datalog::fact(name, terms)
}

fn s(text: &str) -> Term {
    datalog::string(text)
}

/// The authority facts of R3's rights. Strings no rule reads grant nothing.
fn rights_facts(rights: &RightsText) -> Vec<Fact> {
    let mut facts = Vec::new();
    let mut names = |list: &[String], exact: &str, prefix: &str, any: &str| {
        for right in list {
            match right.as_str() {
                "*" => facts.push(fact(any, &[datalog::boolean(true)])),
                right => match right.strip_suffix('*') {
                    Some(p) if p.ends_with('.') => facts.push(fact(prefix, &[s(p)])),
                    _ => facts.push(fact(exact, &[s(right)])),
                },
            }
        }
    };
    names(&rights.call, "call", "call_prefix", "call_any");
    names(
        &rights.provide,
        "provide_method",
        "provide_prefix",
        "provide_any",
    );
    names(&rights.emit, "emit_subject", "emit_prefix", "emit_any");
    names(
        &rights.subscribe,
        "subscribe_pattern",
        "subscribe_prefix",
        "subscribe_any",
    );
    for right in &rights.control {
        // `<action>:*` is over every instance: the unscoped `<action>` (`*:*` is `*`).
        match right.split_once(':') {
            None if right == "*" => facts.push(fact("control_all", &[datalog::boolean(true)])),
            Some(("*", "*")) => facts.push(fact("control_all", &[datalog::boolean(true)])),
            None => facts.push(fact("control_any", &[s(right)])),
            Some((action, "*")) => facts.push(fact("control_any", &[s(action)])),
            Some((action, instance)) => match instance.strip_suffix('*') {
                Some(p) => facts.push(fact("control_prefix", &[s(action), s(p)])),
                None => facts.push(fact("control", &[s(action), s(instance)])),
            },
        }
    }
    for right in &rights.pane {
        match right.as_str() {
            "profile:*" => facts.push(fact("pane_profile_any", &[datalog::boolean(true)])),
            "unrestricted" => facts.push(fact("pane_unrestricted", &[datalog::boolean(true)])),
            "override" => facts.push(fact("pane_override", &[datalog::boolean(true)])),
            right => {
                if let Some(name) = right.strip_prefix("profile:") {
                    facts.push(fact("pane_profile", &[s(name)]));
                } else if let Some(access) = right.strip_prefix("access:") {
                    facts.push(fact("pane_access", &[s(access)]));
                }
            }
        }
    }
    facts
}

/// The facts the authorizer adds for a request ([`request_facts`] and
/// [`Acl::datalog_run`]): all a check of an appended block may read. Each holds one
/// fact per request at most, so a check binds its variables once, whatever it joins.
pub const REQUEST_FACTS: &[&str] = &[
    "kind",
    "caller",
    "server",
    "origin",
    "time",
    "method",
    "namespace",
    "target",
    "subject",
    "pattern",
    "action",
    "instance",
    "scope",
    "pane_profile_requested",
    "pane_overrides",
    "pane",
    "pane_creator",
];

/// Whether every appended block of `biscuit` is one a decision stays cheap with
/// (R7), read from the block's signed bytes as the authorizer reads them, never
/// from printed source: Biscuit prints symbols unescaped, so a name can carry
/// source text that reads as something else.
fn check_appended_blocks(biscuit: &Biscuit) -> Result<(), String> {
    // The token's symbol table as Biscuit makes it (third-party blocks are refused
    // before): the default symbols, then each block's, in order.
    let mut names = Vec::new();
    for index in 0..biscuit.block_count() {
        names.extend(
            biscuit
                .block_symbols(index)
                .map_err(|_| UNREADABLE.to_owned())?,
        );
    }
    let symbols =
        biscuit_auth::datalog::SymbolTable::from(names).map_err(|_| UNREADABLE.to_owned())?;
    let container = biscuit.container().to_proto();
    for signed in &container.blocks {
        check_appended_block(&signed.block, &symbols)?;
    }
    Ok(())
}

const UNREADABLE: &str = "an appended block does not read";

/// Whether the appended block of `data` (its protobuf bytes) holds checks only (no
/// fact, no rule, no scope, no public key), at most [`MAX_BLOCK_CHECKS`] queries,
/// each reading only [`REQUEST_FACTS`] with at most [`MAX_CHECK_PREDICATES`]
/// predicates, and at most [`MAX_BLOCK_OPS`] expression operations, none of them a
/// closure over values (`.all`, `.any`), a regex or an extern function. Biscuit
/// checks its time limit only between checks, so one check must never be costly:
/// with one binding per check and a bounded expression it never is. Anything it does
/// not know is refused: a field the schema does not have, a schema version above
/// biscuit-auth's, an unknown kind or operation.
fn check_appended_block(
    data: &[u8],
    symbols: &biscuit_auth::datalog::SymbolTable,
) -> Result<(), String> {
    use biscuit_auth::builder::{Binary, Check, CheckKind, Convert as _, Op, Unary};
    use biscuit_auth::format::{convert, schema};
    use biscuit_prost::Message as _;
    let unreadable = || UNREADABLE.to_owned();
    let block = schema::Block::decode(data).map_err(|_| unreadable())?;
    // Decoding drops what the schema does not know and fills a missing required
    // field with its default: only a block that is its own canonical encoding, byte
    // for byte, is read whole (a length alone lets an unknown field stand in for a
    // missing one of the same size). biscuit-auth writes its blocks so.
    if block.encode_to_vec() != data {
        return Err(unreadable());
    }
    if !block.facts.is_empty() || !block.rules.is_empty() {
        return Err("an appended block holds checks only, no facts or rules".into());
    }
    if !block.scope.is_empty()
        || !block.public_keys.is_empty()
        || block
            .checks
            .iter()
            .flat_map(|check| &check.queries)
            .any(|query| !query.scope.is_empty())
    {
        return Err("an appended block may not trust other blocks or keys".into());
    }
    if block
        .checks
        .iter()
        .any(|check| !matches!(check.kind, None | Some(0..=2)))
    {
        return Err(unreadable());
    }
    // The version range, check kinds, terms and operations as the authorizer reads them.
    let block = convert::proto_block_to_token_block(&block, None).map_err(|_| unreadable())?;
    let checks = block
        .checks
        .iter()
        .map(|check| Check::convert_from(check, symbols))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| unreadable())?;
    for check in &checks {
        match check.kind {
            CheckKind::One | CheckKind::All | CheckKind::Reject => {}
        }
    }
    let queries: Vec<_> = checks.iter().flat_map(|c| &c.queries).collect();
    if queries.len() > MAX_BLOCK_CHECKS {
        return Err(format!(
            "an appended block holds at most {MAX_BLOCK_CHECKS} checks"
        ));
    }
    fn ops_ok(ops: &[Op], count: &mut usize) -> Result<(), String> {
        for op in ops {
            *count += 1;
            match op {
                Op::Closure(params, _) if !params.is_empty() => {
                    return Err("an appended block may not iterate (.all, .any)".into());
                }
                Op::Closure(_, inner) => ops_ok(inner, count)?,
                Op::Binary(Binary::All | Binary::Any) => {
                    return Err("an appended block may not iterate (.all, .any)".into());
                }
                Op::Binary(Binary::Regex) => {
                    return Err("an appended block may not use .matches".into());
                }
                Op::Binary(Binary::Ffi(_)) | Op::Unary(Unary::Ffi(_)) => {
                    return Err("an appended block may not call extern functions".into());
                }
                // Each linear in its operands, which the block's size bounds.
                Op::Value(_)
                | Op::Unary(Unary::Negate | Unary::Parens | Unary::Length | Unary::TypeOf)
                | Op::Binary(
                    Binary::LessThan
                    | Binary::GreaterThan
                    | Binary::LessOrEqual
                    | Binary::GreaterOrEqual
                    | Binary::Equal
                    | Binary::Contains
                    | Binary::Prefix
                    | Binary::Suffix
                    | Binary::Add
                    | Binary::Sub
                    | Binary::Mul
                    | Binary::Div
                    | Binary::And
                    | Binary::Or
                    | Binary::Intersection
                    | Binary::Union
                    | Binary::BitwiseAnd
                    | Binary::BitwiseOr
                    | Binary::BitwiseXor
                    | Binary::NotEqual
                    | Binary::HeterogeneousEqual
                    | Binary::HeterogeneousNotEqual
                    | Binary::LazyAnd
                    | Binary::LazyOr
                    | Binary::Get
                    | Binary::TryOr,
                ) => {}
            }
        }
        Ok(())
    }
    let mut ops = 0;
    for query in queries {
        if !query.scopes.is_empty() {
            return Err("an appended block may not trust other blocks or keys".into());
        }
        if query.body.len() > MAX_CHECK_PREDICATES {
            return Err(format!(
                "an appended block's check reads at most {MAX_CHECK_PREDICATES} facts"
            ));
        }
        if let Some(predicate) = query
            .body
            .iter()
            .find(|p| !REQUEST_FACTS.contains(&p.name.as_str()))
        {
            return Err(format!(
                "an appended block's checks read the request's facts only, not {:?}",
                clip(&predicate.name, 64)
            ));
        }
        for expression in &query.expressions {
            ops_ok(&expression.ops, &mut ops)?;
        }
    }
    if ops > MAX_BLOCK_OPS {
        return Err(format!(
            "an appended block holds at most {MAX_BLOCK_OPS} operations"
        ));
    }
    Ok(())
}

/// Segments of `[a-z0-9_-]`, dot-separated, at least one.
fn valid_prefix(prefix: &str) -> bool {
    !prefix.is_empty()
        && prefix.split('.').all(|segment| {
            !segment.is_empty()
                && segment
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
        })
}

/// Whether `rights` are well formed (`bus.token`); the first bad one otherwise.
fn check_rights(rights: &RightsText) -> Result<(), String> {
    let names = |list: &[String], valid: fn(&str) -> bool, what: &str| {
        for right in list {
            let ok = match right.as_str() {
                "*" => true,
                right => match right.strip_suffix(".*") {
                    Some(prefix) => valid_prefix(prefix),
                    None => valid(right),
                },
            };
            if !ok {
                return Err(format!("{right:?} is not a {what} right"));
            }
        }
        Ok(())
    };
    names(&rights.call, names::valid_method, "call")?;
    names(&rights.provide, names::valid_method, "provide")?;
    names(&rights.emit, names::valid_subject, "emit")?;
    names(&rights.subscribe, names::valid_pattern, "subscribe")?;
    for right in &rights.control {
        let (action, instance) = match right.split_once(':') {
            Some((a, i)) => (a, Some(i)),
            None => (right.as_str(), None),
        };
        let action_ok = action == "*" || CONTROL_ACTIONS.contains(&action);
        let instance_ok = instance.is_none_or(|i| {
            let i = i.strip_suffix('*').unwrap_or(i);
            (instance.is_some_and(|whole| whole.ends_with('*')) || !i.is_empty())
                && i.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-' || b == b'.')
        });
        if !action_ok || !instance_ok {
            return Err(format!("{right:?} is not a control right"));
        }
    }
    for right in &rights.pane {
        let ok = matches!(
            right.as_str(),
            "profile:*" | "unrestricted" | "override" | "access:own" | "access:any"
        ) || right
            .strip_prefix("profile:")
            .is_some_and(|name| !name.is_empty());
        if !ok {
            return Err(format!("{right:?} is not a pane right"));
        }
    }
    Ok(())
}

/// Whether every right of `wanted` is one of `held` (verbatim, or under a `*` of
/// the same kind): what a participant other than the owner may mint.
fn rights_within(wanted: &RightsText, held: &RightsText) -> bool {
    let within = |wanted: &[String], held: &[String]| {
        wanted
            .iter()
            .all(|right| held.iter().any(|h| h == "*" || h == right))
    };
    within(&wanted.call, &held.call)
        && within(&wanted.provide, &held.provide)
        && within(&wanted.emit, &held.emit)
        && within(&wanted.subscribe, &held.subscribe)
        && within(&wanted.control, &held.control)
        && wanted.pane.iter().all(|right| held.pane.contains(right))
}

/// What a token minted by `minter`, a participant other than the owner, inherits
/// (its [`Lineage`]). Refused: a minter with appended blocks (its narrowing would
/// not reach the child), a name outside the minter's own (`<minter kind>.<minter
/// name>.<name>`: `instance.a.x` for `instance:a`, `token.a.x` for `token:a`, so a
/// child cannot pass for another participant, nor the child of a minter of the other
/// kind with the same name; the child's own `<name>` has no dot, so `instance.a.b.x` is
/// child `x` of `instance:a.b` and never a child of `instance:a`), and an origin other than the one the minter is bound to
/// (a bound minter's child is bound to it too).
fn child_lineage(minter: &Parsed, name: &str, origin: &mut String) -> Result<Lineage, BusError> {
    let refuse = |message: String| BusError::new(codes::DENIED, message);
    if minter.attenuated() {
        return Err(refuse(
            "a token with appended blocks cannot mint: mint with the token it was attenuated from"
                .into(),
        ));
    }
    let (kind, own) = match &minter.identity {
        Identity::Token(own) => ("token", own.as_str()),
        Identity::Instance(own) => ("instance", own.as_str()),
        _ => ("", ""),
    };
    let under = name
        .strip_prefix(kind)
        .and_then(|rest| rest.strip_prefix('.'))
        .and_then(|rest| rest.strip_prefix(own))
        .and_then(|rest| rest.strip_prefix('.'))
        .is_some_and(|rest| !rest.is_empty() && !rest.contains('.'));
    if own.is_empty() || !under {
        return Err(refuse(format!(
            "{} mints tokens named {kind}.{own}.<name> only, <name> without a dot, not {name:?}",
            minter.identity
        )));
    }
    let bound = &minter.meta.origin;
    if !bound.is_empty() {
        if origin.is_empty() {
            origin.clone_from(bound);
        } else if origin != bound {
            return Err(refuse(format!(
                "a token bound to the origin {bound:?} mints tokens for that origin only"
            )));
        }
    }
    let ceiling_ms = match minter.meta.lineage.ceiling_ms {
        0 => minter.meta.expires_ms,
        ceiling => ceiling.min(minter.meta.expires_ms),
    };
    // The minter's family, not its token's own id: renewing the minter keeps its
    // children, revoking it (any token of the chain) ends them.
    let mut parents = minter.meta.lineage.parents.clone();
    parents.extend(minter.family_id().cloned());
    Ok(Lineage {
        ceiling_ms,
        parents,
    })
}

// ---------------------------------------------------------------------------
// The authorizer
// ---------------------------------------------------------------------------

/// The root key and the count of rotations.
struct Keys {
    /// `None`: no usable key (no state directory, or the file is bad): every token
    /// is refused and none is minted.
    pair: Option<KeyPair>,
    /// Why there is none.
    missing: String,
    epoch: u64,
}

/// An instance's token, alive while its connection is.
struct InstanceSeat {
    conn: ConnId,
    /// The connection was seen open (it may be checked before it is registered).
    seen: bool,
    made: Instant,
    expires_ms: u64,
    /// The key epoch it was minted under.
    epoch: u64,
    /// Its family ([`Parsed::family`]), revoked with it when the connection ends.
    family: String,
}

#[derive(Default)]
struct Revoked {
    /// Revocation id → Unix ms until which it is kept: it ends every token it is
    /// one of the [`Parsed::ending_ids`] of.
    ids: HashMap<String, u64>,
    /// The authority revocation ids of renewed tokens → Unix ms until which each is
    /// kept: it ends that token (and those attenuated from it) only, not its family
    /// or the tokens its holder minted.
    renewed: HashMap<String, u64>,
    /// The file could not be read: every token is refused, and the file is never
    /// written (so it stays broken, and refused, after a restart) until the
    /// operator fixes or removes it.
    broken: Option<String>,
}

/// The revocation set and `<state dir>/bus-revoked`.
struct Store {
    dir: Option<PathBuf>,
    revoked: Mutex<Revoked>,
    /// Held from the snapshot of `revoked` to the rename of its file, and across a
    /// rotation: one writer of the state files at a time. Taken before `keys` and
    /// `revoked`, never while holding them.
    files: Mutex<()>,
}

impl Store {
    /// Write the set as it is now. Never while the file is broken.
    fn persist(&self) -> Result<(), String> {
        let Some(dir) = &self.dir else {
            return Ok(());
        };
        let _files = lock(&self.files);
        let snapshot: Vec<(String, u64, bool)> = {
            let revoked = lock(&self.revoked);
            if let Some(reason) = &revoked.broken {
                return Err(format!(
                    "{REVOKED_FILE} is not written while it does not read ({reason}); fix or remove it and restart"
                ));
            }
            let ids = revoked
                .ids
                .iter()
                .map(|(id, until)| (id.clone(), *until, false));
            let renewed = revoked
                .renewed
                .iter()
                .map(|(id, until)| (id.clone(), *until, true));
            ids.chain(renewed).collect()
        };
        write_revoked(&dir.join(REVOKED_FILE), &snapshot).map_err(|err| format!("{err:#}"))
    }
}

/// Parsed tokens by the SHA-256 of their bytes, with the tick of their last use.
type ParsedCache = HashMap<[u8; 32], (Arc<Parsed>, u64)>;

struct Decision {
    at: Instant,
    epoch: u64,
    result: Result<(), String>,
    /// The token's [`Parsed::ending_ids`]: a revocation drops its decisions only.
    ids: Arc<[String]>,
}

/// What the sweeper does with a connection.
#[derive(Debug, PartialEq, Eq)]
enum Sweep {
    /// An instance's token is due (12 h) or of an earlier key: a new one.
    Renew,
    /// An outside participant's token expired or is of an earlier key: 4003.
    Close(&'static str),
}

/// The Biscuit authorizer of the bus (R2-R8). See the module docs.
pub struct Acl {
    access_token: Option<String>,
    server_id: String,
    state_dir: Option<PathBuf>,
    keys: RwLock<Keys>,
    /// The revoked ids and their file; shared with the blocking threads that
    /// write it.
    store: Arc<Store>,
    parsed: Mutex<ParsedCache>,
    parsed_tick: Mutex<u64>,
    /// By [`decision_key`].
    decisions: Mutex<HashMap<[u8; 32], Decision>>,
    instances: Mutex<HashMap<String, InstanceSeat>>,
    broker: OnceLock<BrokerHandle>,
    policies: AuthorizerBuilder,
    /// Unix milliseconds now: the test clock under `failpoints`.
    clock: fn() -> u64,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Every [`Acl`] made in this process, to find the broker's in [`register`].
static MADE: Mutex<Vec<Weak<Acl>>> = Mutex::new(Vec::new());

/// The authorizer of a server (`default_authorizer`'s body).
pub fn authorizer(cfg: &AuthzConfig) -> Arc<Acl> {
    let acl = Arc::new(Acl::new(cfg, crate::time::now_ms));
    let mut made = lock(&MADE);
    made.retain(|weak| weak.strong_count() > 0);
    made.push(Arc::downgrade(&acl));
    acl
}

fn connection_error(code: &str, message: impl Into<String>) -> ConnectionError {
    ConnectionError {
        code: code.into(),
        message: message.into(),
    }
}

fn denied(message: impl Into<String>) -> ConnectionError {
    connection_error(codes::DENIED, message)
}

/// One request, as facts.
struct Request<'a> {
    kind: &'static str,
    /// For messages and audit: `call test.echo`, `subscribe to pane.*`.
    what: String,
    /// `method=...`, `subject=...` for audit.
    audit: String,
    target: &'a str,
    facts: Vec<Fact>,
    /// The parts of the decision cache key besides the token, kind, caller and
    /// origin.
    key: Vec<&'a str>,
}

/// The decision cache's key: SHA-256 of the token's hash and the length-prefixed
/// `parts`, so a key is 32 bytes whatever a request names, and no two requests
/// share one.
fn decision_key(token: &[u8; 32], parts: &[&str]) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(token);
    for part in parts {
        hash.update((part.len() as u64).to_le_bytes());
        hash.update(part.as_bytes());
    }
    hash.finalize().into()
}

/// The facts of the call a follow-up check is made for (`method`, `namespace`,
/// `target`), as [`AuthzRequest::Call`] adds them: an appended check reads the same
/// call in each check made for it.
fn call_facts(facts: &mut Vec<Fact>, call: ForCall<'_>) {
    facts.push(fact("method", &[s(call.method)]));
    facts.push(fact("namespace", &[s(names::namespace(call.method))]));
    facts.push(fact("target", &[s(call.target)]));
}

/// Map a request to its kind and facts (R6); `None`: unknown, denied.
fn request_facts<'a>(request: &AuthzRequest<'a>) -> Result<Request<'a>, Denied> {
    let target = match *request {
        AuthzRequest::Call { target, .. }
        | AuthzRequest::Control {
            call: ForCall { target, .. },
            ..
        }
        | AuthzRequest::PaneCreate {
            call: ForCall { target, .. },
            ..
        }
        | AuthzRequest::PaneAccess {
            call: ForCall { target, .. },
            ..
        } => target,
        _ => "",
    };
    if target.len() > MAX_TARGET_BYTES {
        // Not read further, not cached, not logged.
        return Err(Denied::new(format!(
            "the target is over {MAX_TARGET_BYTES} bytes: no participant has such a name"
        )));
    }
    let mut facts = Vec::new();
    #[allow(
        unreachable_patterns,
        reason = "another lane may append a variant to AuthzRequest (bus/mod.rs); one this authorizer does not know is denied"
    )]
    let request = match *request {
        AuthzRequest::Call { method, target } => {
            let namespace = names::namespace(method);
            facts.push(fact("method", &[s(method)]));
            facts.push(fact("namespace", &[s(namespace)]));
            facts.push(fact("target", &[s(target)]));
            let control = match method {
                "bus.ping" | "bus.renew" => None,
                "bus.instances" | "bus.methods" => Some(("control", "list")),
                "bus.start" => Some(("control_gate", "start")),
                "bus.stop" => Some(("control_gate", "stop")),
                "bus.reload" => Some(("control_gate", "reload")),
                "bus.reload_config" => Some(("control", "reload_config")),
                "bus.token" => Some(("control", "token")),
                "bus.revoke" => Some(("control", "revoke")),
                "bus.rotate_key" => Some(("control", "rotate_key")),
                "server.reload" => Some(("control", "server.reload")),
                "server.update" => Some(("control", "server.update")),
                // Only these exact names are control methods. Any other name, in
                // `bus` too (`bus.absent`, `bus.Token`, `bus.token `), is a plain
                // call: a caller whose `call` rights cover it passes and the broker
                // answers `not_found` (no participant provides in `bus`).
                _ => None,
            };
            let kind = match (method, control) {
                ("bus.ping" | "bus.renew", _) => "bus",
                (_, Some((kind, action))) => {
                    facts.push(fact("action", &[s(action)]));
                    kind
                }
                (_, None) => "call",
            };
            Request {
                kind,
                what: match control {
                    Some((_, action)) => format!("control {action} ({method})"),
                    None => format!("call {method}"),
                },
                audit: match control {
                    Some((_, action)) => format!("method={method} action={action}"),
                    None => format!("method={method}"),
                },
                target,
                facts,
                key: vec![method, target],
            }
        }
        AuthzRequest::Provide { method } => {
            facts.push(fact("method", &[s(method)]));
            facts.push(fact("namespace", &[s(names::namespace(method))]));
            Request {
                kind: "provide",
                what: format!("provide {method}"),
                audit: format!("method={method}"),
                target: "",
                facts,
                key: vec![method],
            }
        }
        AuthzRequest::Emit { subject } => {
            if names::is_reserved(subject) {
                return Err(Denied::new(format!(
                    "only tilrd emits in the namespace {}",
                    names::namespace(subject)
                )));
            }
            facts.push(fact("subject", &[s(subject)]));
            facts.push(fact("namespace", &[s(names::namespace(subject))]));
            Request {
                kind: "emit",
                what: format!("emit {subject}"),
                audit: format!("subject={subject}"),
                target: "",
                facts,
                key: vec![subject],
            }
        }
        AuthzRequest::Subscribe { pattern } => {
            facts.push(fact("pattern", &[s(pattern)]));
            Request {
                kind: "subscribe",
                what: format!("subscribe to {pattern}"),
                audit: format!("pattern={pattern}"),
                target: "",
                facts,
                key: vec![pattern],
            }
        }
        AuthzRequest::Control {
            action,
            instance,
            call,
        } => {
            call_facts(&mut facts, call);
            facts.push(fact("action", &[s(action)]));
            if let Some(instance) = instance {
                facts.push(fact("instance", &[s(instance)]));
            }
            Request {
                kind: "control",
                what: match instance {
                    Some(instance) => format!("control {action}:{instance}"),
                    None => format!("control {action}"),
                },
                audit: format!("action={action} instance={}", instance.unwrap_or("")),
                target: instance.unwrap_or(""),
                facts,
                key: match instance {
                    Some(instance) => vec![call.method, call.target, action, "instance", instance],
                    None => vec![call.method, call.target, action],
                },
            }
        }
        AuthzRequest::Event { subject, scope } => {
            facts.push(fact("subject", &[s(subject)]));
            facts.push(fact("scope", &[s(scope)]));
            Request {
                kind: "event",
                what: format!("receive {subject}"),
                audit: format!("subject={subject}"),
                target: "",
                facts,
                key: vec![subject, scope],
            }
        }
        AuthzRequest::PaneCreate {
            profile,
            overrides,
            call,
        } => {
            call_facts(&mut facts, call);
            facts.push(fact("pane_profile_requested", &[s(profile)]));
            facts.push(fact("pane_overrides", &[datalog::boolean(overrides)]));
            Request {
                kind: "pane",
                what: format!("create a pane of profile {profile:?}"),
                audit: format!("profile={profile}"),
                target: "",
                facts,
                key: vec![
                    "create",
                    call.method,
                    call.target,
                    profile,
                    if overrides { "true" } else { "false" },
                ],
            }
        }
        AuthzRequest::PaneAccess {
            pane_id,
            creator,
            call,
        } => {
            call_facts(&mut facts, call);
            facts.push(fact("pane", &[s(pane_id)]));
            facts.push(fact("pane_creator", &[s(creator)]));
            Request {
                kind: "pane",
                what: format!("use pane {pane_id}"),
                audit: format!("pane={pane_id}"),
                target: "",
                facts,
                key: vec!["access", call.method, call.target, pane_id, creator],
            }
        }
        _ => return Err(Denied::new("a request this server does not know")),
    };
    Ok(request)
}

impl Acl {
    fn new(cfg: &AuthzConfig, clock: fn() -> u64) -> Self {
        let keys = match &cfg.state_dir {
            // As the server id without one: a key that lasts until the server stops.
            None => {
                tracing::warn!(target: "tilrd::bus", "no state directory: bus tokens end when the server stops");
                Keys {
                    pair: Some(KeyPair::new()),
                    missing: String::new(),
                    epoch: 0,
                }
            }
            Some(dir) => match load_or_make_key(dir) {
                Ok(pair) => Keys {
                    pair: Some(pair),
                    missing: String::new(),
                    epoch: 0,
                },
                Err(err) => {
                    tracing::error!(target: "tilrd::bus", "bus tokens are refused: {err:#}");
                    Keys {
                        pair: None,
                        missing: format!("{err:#}"),
                        epoch: 0,
                    }
                }
            },
        };
        let revoked = match &cfg.state_dir {
            None => Revoked::default(),
            Some(dir) => match load_revoked(&dir.join(REVOKED_FILE), clock()) {
                Ok(revoked) => revoked,
                Err(err) => {
                    tracing::error!(target: "tilrd::bus", "bus tokens are refused: {err:#}");
                    Revoked {
                        broken: Some(format!("{err:#}")),
                        ..Revoked::default()
                    }
                }
            },
        };
        let policies = AuthorizerBuilder::new()
            .code(POLICIES)
            .expect("the policies parse")
            .set_limits(run_limits());
        Self {
            access_token: cfg.access_token.clone(),
            server_id: cfg.server_id.clone(),
            state_dir: cfg.state_dir.clone(),
            keys: RwLock::new(keys),
            store: Arc::new(Store {
                dir: cfg.state_dir.clone(),
                revoked: Mutex::new(revoked),
                files: Mutex::new(()),
            }),
            parsed: Mutex::new(HashMap::new()),
            parsed_tick: Mutex::new(0),
            decisions: Mutex::new(HashMap::new()),
            instances: Mutex::new(HashMap::new()),
            broker: OnceLock::new(),
            policies,
            clock,
        }
    }

    fn now_ms(&self) -> u64 {
        (self.clock)()
    }

    fn keys(&self) -> std::sync::RwLockReadGuard<'_, Keys> {
        self.keys
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    // --- minting ---------------------------------------------------------

    /// A token for `participant` with `rights`, bound to this server, `origin`
    /// (empty: none), expiring `ttl` from now (never past `lineage.ceiling_ms`). One
    /// [`Self::verify`] would refuse (over 16 KiB, too many rights facts) is not made.
    fn mint(
        &self,
        participant: &Identity,
        rights: &RightsText,
        origin: &str,
        ttl: Duration,
        lineage: &Lineage,
        family: &str,
    ) -> Result<Token, String> {
        let keys = self.keys();
        let pair = keys.pair.as_ref().ok_or_else(|| keys.missing.clone())?;
        let now = self.now_ms();
        let ttl_ms = u64::try_from(ttl.min(MAX_TTL).as_millis()).unwrap_or(u64::MAX);
        let mut expires_ms = now.saturating_add(ttl_ms);
        if lineage.ceiling_ms != 0 {
            if lineage.ceiling_ms <= now {
                return Err("its minter's token has expired".into());
            }
            expires_ms = expires_ms.min(lineage.ceiling_ms);
        }
        let rights_facts = rights_facts(rights);
        if rights_facts.len() > MAX_RIGHTS_FACTS {
            return Err(format!(
                "a token holds at most {MAX_RIGHTS_FACTS} rights, not {}",
                rights_facts.len()
            ));
        }
        let meta = Meta {
            participant: participant.to_string(),
            server: self.server_id.clone(),
            minted_ms: now,
            expires_ms,
            origin: origin.to_owned(),
            rights: rights.clone(),
            lineage: lineage.clone(),
            family: family.to_owned(),
        };
        let context = serde_json::to_string(&meta).map_err(|err| err.to_string())?;
        let build = || -> Result<Biscuit, TokenError> {
            let mut builder = Biscuit::builder()
                .context(context.clone())
                .fact(fact("participant", &[s(&meta.participant)]))?;
            for right in rights_facts.iter().cloned() {
                builder = builder.fact(right)?;
            }
            let mut params = HashMap::new();
            params.insert("server".to_owned(), s(&self.server_id));
            params.insert("expiry".to_owned(), date(meta.expires_ms));
            let mut checks =
                String::from("check if server({server});\ncheck if time($t), $t <= {expiry};\n");
            if !origin.is_empty() {
                params.insert("origin".to_owned(), s(origin));
                checks.push_str("check if origin({origin});\n");
            }
            builder = builder.code_with_params(checks, params, HashMap::new())?;
            builder.build(pair)
        };
        let biscuit = build().map_err(|err| format!("cannot mint: {err}"))?;
        let bytes = Bytes::from(biscuit.to_vec().map_err(|err| err.to_string())?);
        if bytes.len() > MAX_TOKEN_BYTES {
            return Err(format!(
                "the token would be over {} KiB: fewer or shorter rights",
                MAX_TOKEN_BYTES / 1024
            ));
        }
        let parsed = Arc::new(Parsed {
            hash: Sha256::digest(&bytes).into(),
            rids: biscuit
                .revocation_identifiers()
                .iter()
                .map(|id| hex(id))
                .collect(),
            identity: participant.clone(),
            meta,
            biscuit,
            epoch: keys.epoch,
        });
        Ok(Token { bytes, parsed })
    }

    /// An instance's token for its connection `conn` (R2, R3), of the family
    /// `family` when it renews one (empty: a new family).
    fn mint_instance(
        &self,
        identity: &Identity,
        rights: &RightsText,
        conn: ConnId,
        seen: bool,
        family: &str,
    ) -> Result<Token, String> {
        let token = self.mint(identity, rights, "", MAX_TTL, &Lineage::default(), family)?;
        lock(&self.instances).insert(
            token.revocation_id().to_owned(),
            InstanceSeat {
                conn,
                seen,
                made: Instant::now(),
                expires_ms: token.expires_ms(),
                epoch: token.parsed.epoch,
                family: token.parsed.family().to_owned(),
            },
        );
        Ok(token)
    }

    // --- verifying -------------------------------------------------------

    /// Parse and verify `bytes` (signature of this server's key, size, blocks).
    fn verify(&self, bytes: &[u8]) -> Result<Arc<Parsed>, Denied> {
        if bytes.len() > MAX_TOKEN_BYTES {
            return Err(Denied::new(format!(
                "the token is over {} KiB",
                MAX_TOKEN_BYTES / 1024
            )));
        }
        let hash: [u8; 32] = Sha256::digest(bytes).into();
        let keys = self.keys();
        let Some(pair) = keys.pair.as_ref() else {
            return Err(Denied::new(format!(
                "this server cannot verify tokens: {}",
                keys.missing
            )));
        };
        {
            let mut tick = lock(&self.parsed_tick);
            *tick += 1;
            let mut cache = lock(&self.parsed);
            if let Some((parsed, used)) = cache.get_mut(&hash)
                && parsed.epoch == keys.epoch
            {
                *used = *tick;
                return Ok(Arc::clone(parsed));
            }
        }
        // Counted before any signature is checked: one per block.
        let unverified = UnverifiedBiscuit::from(bytes)
            .map_err(|_| Denied::new("not a token of this server"))?;
        if unverified.block_count() > MAX_BLOCKS {
            return Err(Denied::new(format!(
                "the token has over {MAX_BLOCKS} blocks"
            )));
        }
        if unverified
            .external_public_keys()
            .iter()
            .any(Option::is_some)
        {
            return Err(Denied::new("third-party blocks are not accepted"));
        }
        let biscuit = unverified
            .verify(pair.public())
            .map_err(|_| Denied::new("not a token of this server"))?;
        check_appended_blocks(&biscuit).map_err(Denied::new)?;
        let meta: Meta = biscuit
            .context()
            .first()
            .cloned()
            .flatten()
            .and_then(|context| serde_json::from_str(&context).ok())
            .ok_or_else(|| Denied::new("not a token of this server"))?;
        let identity = Identity::parse(&meta.participant)
            .filter(|identity| matches!(identity, Identity::Instance(_) | Identity::Token(_)))
            .ok_or_else(|| Denied::new("not a token of this server"))?;
        let parsed = Arc::new(Parsed {
            hash,
            rids: biscuit
                .revocation_identifiers()
                .iter()
                .map(|id| hex(id))
                .collect(),
            identity,
            meta,
            biscuit,
            epoch: keys.epoch,
        });
        let tick = *lock(&self.parsed_tick);
        let mut cache = lock(&self.parsed);
        if cache.len() >= PARSED_CACHE
            && let Some(oldest) = cache
                .iter()
                .min_by_key(|(_, (_, used))| *used)
                .map(|(hash, _)| *hash)
        {
            cache.remove(&oldest);
        }
        cache.insert(hash, (Arc::clone(&parsed), tick));
        Ok(parsed)
    }

    /// The checks of a token that need no Datalog, on every use (also on a
    /// decision-cache hit): key epoch, revocation, the instance's connection,
    /// expiry, origin. `conn` is the connection presenting it.
    fn usable(&self, parsed: &Parsed, origin: &str, conn: ConnId) -> Result<(), Denied> {
        if parsed.meta.server != self.server_id {
            return Err(Denied::new(
                "not a token of this server: it was minted by another server id",
            ));
        }
        let epoch = self.keys().epoch;
        if parsed.epoch != epoch {
            return Err(Denied::new(
                "not a token of this server: its root key was rotated",
            ));
        }
        {
            let revoked = lock(&self.store.revoked);
            if let Some(reason) = &revoked.broken {
                return Err(Denied::new(format!(
                    "this server cannot check revocations: {reason}"
                )));
            }
            if parsed.rids.iter().any(|rid| revoked.ids.contains_key(rid))
                || parsed
                    .family_id()
                    .is_some_and(|family| revoked.ids.contains_key(family))
            {
                return Err(Denied::new("revoked: the token was revoked"));
            }
            if parsed
                .rids
                .iter()
                .any(|rid| revoked.renewed.contains_key(rid))
            {
                return Err(Denied::new("revoked: the token was renewed"));
            }
            if parsed
                .meta
                .lineage
                .parents
                .iter()
                .any(|rid| revoked.ids.contains_key(rid))
            {
                return Err(Denied::new("revoked: the token of its minter was revoked"));
            }
        }
        if let Identity::Instance(_) = parsed.identity {
            self.instance_alive(parsed, conn)?;
        }
        let now = self.now_ms();
        if now > parsed.meta.expires_ms {
            return Err(Denied::new("expired: the token expired"));
        }
        if !parsed.meta.origin.is_empty() && parsed.meta.origin != origin {
            return Err(Denied::new("origin: the token is bound to another origin"));
        }
        Ok(())
    }

    /// An instance's token works while the connection it was minted for is open.
    fn instance_alive(&self, parsed: &Parsed, conn: ConnId) -> Result<(), Denied> {
        let rid = parsed.rids.first().map_or("", String::as_str);
        let seat_conn = {
            let mut instances = lock(&self.instances);
            let Some(seat) = instances.get_mut(rid) else {
                return Err(Denied::new("revoked: the instance's token is not issued"));
            };
            if seat.conn == conn {
                // Its own connection is acting: open, whatever the sweeper saw.
                seat.seen = true;
                return Ok(());
            }
            seat.conn
        };
        // Presented by another connection: is the instance's still open? Not
        // called under the broker's lock (the broker authorizes outside it).
        let open = self
            .broker
            .get()
            .map(|broker| broker.principal_of(seat_conn).is_some());
        match open {
            Some(true) => {
                if let Some(seat) = lock(&self.instances).get_mut(rid) {
                    seat.seen = true;
                }
                Ok(())
            }
            Some(false) => {
                let (seen, young) = lock(&self.instances)
                    .get(rid)
                    .map_or((true, false), |seat| {
                        (seat.seen, seat.made.elapsed() < UNREGISTERED_GRACE)
                    });
                if seen {
                    self.end_instance_token(rid);
                    Err(Denied::new("revoked: the instance's process ended"))
                } else if young {
                    // Greeted, and the broker registers it right after `HelloAck`.
                    Ok(())
                } else {
                    // Never seen open and not open now: it ended before the sweeper
                    // saw it (the sweeper revokes it). Fail closed meanwhile.
                    Err(Denied::new(
                        "revoked: the instance's connection is not open",
                    ))
                }
            }
            // No broker to ask: fail closed.
            None => Err(Denied::new(
                "the instance's connection cannot be checked: the server has no broker",
            )),
        }
    }

    /// Revoke the token of an instance whose connection ended, with its family
    /// (its earlier tokens and what the instance minted): at once in memory, on
    /// disk from a blocking thread (this runs on the envelope path and in the
    /// sweeper).
    fn end_instance_token(&self, rid: &str) {
        let Some(seat) = lock(&self.instances).remove(rid) else {
            return;
        };
        tracing::info!(target: AUDIT, "bus audit: revoke rid={} reason=its instance's connection ended", &rid[..rid.len().min(16)]);
        // Every token of the family expires within a day from now.
        let family_until = self.family_until();
        self.revoke_in_memory(&[
            (rid.to_owned(), seat.expires_ms.saturating_add(1000)),
            (seat.family, family_until),
        ]);
        self.persist_off_path();
    }

    /// An instance's token replaced by its renewal: it alone is refused from now
    /// on, not its family (the instance keeps what it minted).
    fn supersede_instance_token(&self, rid: &str) {
        let Some(seat) = lock(&self.instances).remove(rid) else {
            return;
        };
        self.supersede_in_memory(rid, seat.expires_ms.saturating_add(1000));
        self.persist_off_path();
    }

    /// Until when the revocation of a family is kept: every token of it expires
    /// within [`MAX_TTL`] from now, and none is renewed once it is revoked.
    fn family_until(&self) -> u64 {
        let max_ms = u64::try_from(MAX_TTL.as_millis()).unwrap_or(u64::MAX);
        self.now_ms().saturating_add(max_ms).saturating_add(1000)
    }

    /// Write the revocation file from a blocking thread (or here, without a
    /// runtime).
    fn persist_off_path(&self) {
        let store = Arc::clone(&self.store);
        let persist = move || {
            if let Err(err) = store.persist() {
                tracing::error!(target: "tilrd::bus", "an ended instance's token is revoked until the server stops only: {err}");
            }
        };
        match tokio::runtime::Handle::try_current() {
            Ok(runtime) => {
                runtime.spawn_blocking(persist);
            }
            Err(_) => persist(),
        }
    }

    /// Decide `request` for a token held by `principal` (or presented per call).
    fn run(
        &self,
        parsed: &Parsed,
        principal: &Principal,
        request: &Request<'_>,
    ) -> Result<(), Denied> {
        self.usable(parsed, &principal.origin, principal.connection_id)?;
        let caller = principal.identity.to_string();
        let mut parts = vec![request.kind, caller.as_str(), principal.origin.as_str()];
        parts.extend(&request.key);
        let key = decision_key(&parsed.hash, &parts);
        let epoch = self.keys().epoch;
        if let Some(decision) = lock(&self.decisions).get(&key)
            && decision.epoch == epoch
            && decision.at.elapsed() < DECISION_TTL
        {
            return decision.result.clone().map_err(Denied::new);
        }
        let (result, cacheable) = self.datalog(parsed, &caller, &principal.origin, request);
        if cacheable {
            let mut decisions = lock(&self.decisions);
            if decisions.len() >= DECISION_CACHE {
                decisions.clear();
            }
            decisions.insert(
                key,
                Decision {
                    at: Instant::now(),
                    epoch,
                    result: result.clone(),
                    ids: parsed.ending_ids().cloned().collect(),
                },
            );
        }
        result.map_err(Denied::new)
    }

    /// A fresh Biscuit authorizer for one request (R6, R7); whether its result may
    /// be cached (not when the wall-clock backstop ended it: the load of the
    /// moment, not the token, decided).
    fn datalog(
        &self,
        parsed: &Parsed,
        caller: &str,
        origin: &str,
        request: &Request<'_>,
    ) -> (Result<(), String>, bool) {
        let outcome = self.datalog_run(parsed, caller, origin, request);
        let cacheable = !matches!(
            outcome,
            Err(TokenError::RunLimit(biscuit_auth::error::RunLimit::Timeout))
        );
        let result = match outcome {
            Ok(()) => Ok(()),
            Err(TokenError::RunLimit(_)) => Err("authorization limit reached".into()),
            Err(TokenError::FailedLogic(
                Logic::Unauthorized { checks, .. } | Logic::NoMatchingPolicy { checks },
            )) if !checks.is_empty() => {
                Err(format!("a check of the token refuses {}", request.what))
            }
            Err(TokenError::FailedLogic(_)) => Err(format!(
                "{} has no right to {}",
                parsed.identity, request.what
            )),
            Err(TokenError::Execution(_)) => {
                Err("authorization failed: a check of the token does not evaluate".into())
            }
            Err(_) => Err("authorization failed".into()),
        };
        (result, cacheable)
    }

    fn datalog_run(
        &self,
        parsed: &Parsed,
        caller: &str,
        origin: &str,
        request: &Request<'_>,
    ) -> Result<(), TokenError> {
        let now = self.now_ms();
        let build = || -> Result<biscuit_auth::Authorizer, TokenError> {
            let mut builder = self
                .policies
                .clone()
                .fact(fact("kind", &[s(request.kind)]))?
                .fact(fact("caller", &[s(caller)]))?
                .fact(fact("server", &[s(&self.server_id)]))?
                .fact(fact("origin", &[s(origin)]))?
                .fact(fact("time", &[date(now)]))?;
            for f in &request.facts {
                builder = builder.fact(f.clone())?;
            }
            builder.build(&parsed.biscuit)
        };
        build()
            .and_then(|mut authorizer| authorizer.authorize())
            .map(|_| ())
    }

    /// `bearer`: `token` came with the call (R4); the line names both the caller
    /// (`from`) and the bearer token's participant.
    fn audit(
        &self,
        principal: &Principal,
        token: Option<&Parsed>,
        bearer: bool,
        request: Result<&Request<'_>, &Denied>,
        result: &Result<(), Denied>,
    ) {
        let rid = token.map_or("-", Parsed::rid16);
        let from = match token {
            Some(token) if bearer => format!("{} bearer={}", principal.identity, token.identity),
            _ => principal.identity.to_string(),
        };
        let (kind, detail, target) = match request {
            Ok(request) => (
                request.kind,
                request.audit.as_str(),
                clip(request.target, AUDIT_TARGET_BYTES),
            ),
            Err(_) => ("unknown", "", ""),
        };
        match result {
            // Per-event checks are not audited: there is one per subscriber.
            Err(_) if kind == "event" => {}
            Err(denied) => {
                tracing::info!(target: AUDIT, "bus audit: deny from={from} rid={rid} kind={kind} {detail} target={target} reason={denied}");
            }
            Ok(()) if kind.starts_with("control") => {
                tracing::info!(target: AUDIT, "bus audit: control allow from={from} rid={rid} {detail} target={target}");
            }
            Ok(()) => {
                tracing::debug!(target: AUDIT, "bus audit: allow from={from} rid={rid} kind={kind} {detail} target={target}");
            }
        }
    }

    // --- revocation -------------------------------------------------------

    /// Add revocation ids (with the Unix ms until which each is kept) and write the
    /// file (an error: kept in memory only). File I/O: not on the envelope path.
    fn revoke_ids(&self, ids: &[(String, u64)]) -> Result<(), String> {
        self.revoke_in_memory(ids);
        self.store.persist()
    }

    /// Add revocation ids to the set, at once, and drop the cached decisions of the
    /// tokens they end (no other).
    fn revoke_in_memory(&self, ids: &[(String, u64)]) {
        let now = self.now_ms();
        {
            let mut revoked = lock(&self.store.revoked);
            for (id, until) in ids {
                let kept = revoked.ids.entry(id.clone()).or_insert(0);
                *kept = (*kept).max(*until);
            }
            revoked.ids.retain(|_, until| *until >= now);
            revoked.renewed.retain(|_, until| *until >= now);
        }
        self.drop_decisions(ids.iter().map(|(id, _)| id.as_str()));
    }

    /// Mark the renewed token of authority revocation id `rid` superseded (kept
    /// until `until`), at once.
    fn supersede_in_memory(&self, rid: &str, until: u64) {
        let now = self.now_ms();
        {
            let mut revoked = lock(&self.store.revoked);
            let kept = revoked.renewed.entry(rid.to_owned()).or_insert(0);
            *kept = (*kept).max(until);
            revoked.renewed.retain(|_, until| *until >= now);
        }
        self.drop_decisions(std::iter::once(rid));
    }

    /// Drop the cached decisions of the tokens one of `ids` ends.
    fn drop_decisions<'a>(&self, ids: impl Iterator<Item = &'a str> + Clone) {
        lock(&self.decisions).retain(|_, decision| {
            !decision
                .ids
                .iter()
                .any(|rid| ids.clone().any(|id| id == rid))
        });
    }

    /// Close every connection whose token has one of `rids` (4003).
    fn close_holders(&self, rids: &[String]) {
        let Some(broker) = self.broker.get() else {
            return;
        };
        for conn in broker.connections() {
            let Some(principal) = broker.principal_of(conn.connection_id) else {
                continue;
            };
            if let Some(token) = token_of(&principal)
                && token.parsed.ending_ids().any(|rid| rids.contains(rid))
            {
                tracing::info!(target: AUDIT, "bus audit: close from={} rid={} reason=revoked", principal.identity, token.parsed.rid16());
                broker.close_connection(
                    conn.connection_id,
                    tilr_bus::close::CREDENTIAL,
                    tilr_bus::close::CREDENTIAL_REASON,
                );
            }
        }
    }

    // --- rotation ---------------------------------------------------------

    /// A new root key (R5): every earlier token is refused from now on. The key in
    /// memory changes only once the file has it (the keys' lock is held across the
    /// write, and the files' lock against a concurrent rotation). File I/O: not on
    /// the envelope path.
    fn rotate(&self) -> Result<(), String> {
        let _files = lock(&self.store.files);
        let pair = KeyPair::new();
        let epoch = {
            let mut keys = self
                .keys
                .write()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Some(dir) = &self.state_dir {
                write_key(&dir.join(ROOT_KEY_FILE), &pair).map_err(|err| format!("{err:#}"))?;
            }
            keys.pair = Some(pair);
            keys.missing.clear();
            keys.epoch += 1;
            keys.epoch
        };
        lock(&self.parsed).clear();
        lock(&self.decisions).clear();
        // Seats of the old key only: an instance greeted since has a token of the new.
        lock(&self.instances).retain(|_, seat| seat.epoch == epoch);
        // The revoked ids stay until they expire: clearing them here could drop the
        // revocation of a token of the new key made meanwhile.
        Ok(())
    }

    /// After a rotation or for a renewal: a new token for the instance on `conn`,
    /// which becomes the connection's principal and is sent to the instance in a
    /// `CredentialUpdate`.
    fn renew_instance(&self, broker: &BrokerHandle, conn: ConnId, principal: &Principal) {
        let rights = token_of(principal).map(|token| token.parsed.meta.rights.clone());
        let rights = rights.unwrap_or_else(|| {
            principal
                .rights
                .as_deref()
                .map(RightsText::from)
                .unwrap_or_default()
        });
        let old = token_of(principal).map(|token| token.revocation_id().to_owned());
        let family = token_of(principal).map_or("", |token| token.parsed.family());
        match self.mint_instance(&principal.identity, &rights, conn, true, family) {
            Ok(token) => {
                tracing::info!(target: AUDIT, "bus audit: renew from={} rid={} reason=instance token", principal.identity, token.parsed.rid16());
                let bytes = token.bytes.clone();
                broker.update_principal(
                    conn,
                    Principal {
                        credential: Some(Arc::new(token) as Arc<dyn Any + Send + Sync>),
                        ..principal.clone()
                    },
                );
                // After the principal: a call the instance makes with the new token
                // once it has it is decided by the new token.
                if !broker.send_credential_update(conn, bytes) {
                    tracing::info!(target: "tilrd::bus", identity = %principal.identity, "the instance's connection ended before its new token was sent");
                }
                if let Some(old) = old {
                    self.supersede_instance_token(&old);
                }
            }
            Err(err) => {
                tracing::error!(target: "tilrd::bus", identity = %principal.identity, "cannot renew an instance's token: {err}");
            }
        }
    }

    /// What the sweeper does with a connection of `principal` at `now`: an
    /// instance's token is renewed 12 h after it was minted, or when it is of an
    /// earlier key (a connection greeted during a rotation); an outside
    /// participant's connection is closed when its token expired or is of an
    /// earlier key.
    fn sweep_action(&self, principal: &Principal, now: u64) -> Option<Sweep> {
        let token = token_of(principal)?;
        let meta = &token.parsed.meta;
        let renewal = u64::try_from(INSTANCE_RENEWAL.as_millis()).unwrap_or(u64::MAX);
        let rotated = token.parsed.epoch != self.keys().epoch;
        match &principal.identity {
            Identity::Instance(_) => {
                (rotated || now >= meta.minted_ms.saturating_add(renewal) || now > meta.expires_ms)
                    .then_some(Sweep::Renew)
            }
            _ if rotated => Some(Sweep::Close("key rotated")),
            _ if now > meta.expires_ms => Some(Sweep::Close("expired")),
            _ => None,
        }
    }

    /// What the sweeper does every [`SWEEP`].
    fn sweep(&self, broker: &BrokerHandle) {
        let now = self.now_ms();
        let mut open = Vec::new();
        for conn in broker.connections() {
            open.push(conn.connection_id);
            let Some(principal) = broker.principal_of(conn.connection_id) else {
                continue;
            };
            match self.sweep_action(&principal, now) {
                Some(Sweep::Renew) => self.renew_instance(broker, conn.connection_id, &principal),
                Some(Sweep::Close(reason)) => {
                    let rid = token_of(&principal).map_or("-", |token| token.parsed.rid16());
                    tracing::info!(target: AUDIT, "bus audit: close from={} rid={rid} reason={reason}", principal.identity);
                    broker.close_connection(
                        conn.connection_id,
                        tilr_bus::close::CREDENTIAL,
                        tilr_bus::close::CREDENTIAL_REASON,
                    );
                }
                None => {}
            }
        }
        let ended: Vec<String> = {
            let mut instances = lock(&self.instances);
            for seat in instances.values_mut() {
                if open.contains(&seat.conn) {
                    seat.seen = true;
                }
            }
            instances
                .iter()
                .filter(|(_, seat)| {
                    !open.contains(&seat.conn)
                        && (seat.seen || seat.made.elapsed() > UNSEEN_INSTANCE)
                })
                .map(|(rid, _)| rid.clone())
                .collect()
        };
        for rid in ended {
            self.end_instance_token(&rid);
        }
    }

    // --- methods ------------------------------------------------------------

    /// The token a call names in its payload, or the caller's own.
    fn presented(
        &self,
        ctx: &CallCtx,
        credential: &[u8],
    ) -> Result<(Bytes, Arc<Parsed>), BusError> {
        let bytes = if credential.is_empty() {
            match token_of(&ctx.principal) {
                Some(token) => return Ok((token.bytes.clone(), Arc::clone(&token.parsed))),
                None => {
                    return Err(BusError::bad_request(format!(
                        "{} holds no token; name one",
                        ctx.from
                    )));
                }
            }
        } else {
            Bytes::copy_from_slice(credential)
        };
        let parsed = self
            .verify(&bytes)
            .map_err(|denied| BusError::bad_request(denied.reason))?;
        Ok((bytes, parsed))
    }

    fn method_token(&self, ctx: &CallCtx, payload: &Bytes) -> Result<Reply, BusError> {
        let request = wire::TokenRequest::decode(payload.clone())
            .map_err(|err| BusError::bad_request(format!("TokenRequest: {err}")))?;
        let name = request.name.as_str();
        let name_ok = !name.is_empty()
            && name.len() <= 128
            && name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_-.:".contains(&b));
        if !name_ok {
            return Err(BusError::bad_request(format!(
                "{name:?} is not a participant name ([A-Za-z0-9_.:-], up to 128)"
            )));
        }
        let max_ms = u64::try_from(MAX_TTL.as_millis()).unwrap_or(u64::MAX);
        if request.ttl_ms > max_ms {
            return Err(BusError::bad_request("ttl_ms is at most 24 h"));
        }
        let ttl = match request.ttl_ms {
            0 => MAX_TTL,
            ms => Duration::from_millis(ms),
        };
        let rights = RightsText {
            call: request.call,
            provide: request.provide,
            emit: request.emit,
            subscribe: request.subscribe,
            control: request.control,
            pane: request.pane,
        };
        check_rights(&rights).map_err(BusError::bad_request)?;
        let mut origin = request.origin;
        let mut lineage = Lineage::default();
        if !ctx.from.is_privileged() || ctx.call_credential.is_some() {
            // Not the owner: only rights it holds, for no longer than it holds them,
            // under its own name and origin, and only with a token it was given.
            let minter = match &ctx.call_credential {
                Some(credential) => self
                    .verify(credential)
                    .map_err(|denied| BusError::new(codes::DENIED, denied.reason))?,
                None => token_of(&ctx.principal)
                    .map(|token| Arc::clone(&token.parsed))
                    .ok_or_else(|| BusError::new(codes::DENIED, "minting needs a token"))?,
            };
            lineage = child_lineage(&minter, name, &mut origin)?;
            if !rights_within(&rights, &minter.meta.rights) {
                return Err(BusError::new(
                    codes::DENIED,
                    "a token may be minted only with rights its minter holds",
                ));
            }
        }
        let identity = Identity::Token(name.to_owned());
        let token = self
            .mint(&identity, &rights, &origin, ttl, &lineage, "")
            .map_err(BusError::unavailable)?;
        tracing::info!(target: AUDIT, "bus audit: mint from={} participant={identity} rid={} expires_ms={}", ctx.from, token.parsed.rid16(), token.expires_ms());
        Ok(Reply::encode(&wire::TokenReply {
            credential: token.bytes.clone(),
            expires_at_ms: token.expires_ms(),
            revocation_id: token.revocation_id().to_owned(),
        }))
    }

    fn method_renew(&self, ctx: &CallCtx, payload: &Bytes) -> Result<Reply, BusError> {
        let request = wire::RenewRequest::decode(payload.clone())
            .map_err(|err| BusError::bad_request(format!("RenewRequest: {err}")))?;
        let (_, parsed) = self.presented(ctx, &request.credential)?;
        if parsed.attenuated() {
            return Err(BusError::bad_request(
                "an attenuated token cannot be renewed: renew the token it was attenuated from and attenuate the new one",
            ));
        }
        if let Identity::Instance(_) = parsed.identity {
            return Err(BusError::bad_request(
                "an instance's token is renewed by the server",
            ));
        }
        // The token's own origin, not the caller's: whoever holds it may renew it.
        self.usable(&parsed, &parsed.meta.origin, ctx.principal.connection_id)
            .map_err(|denied| BusError::new(codes::DENIED, denied.reason))?;
        let meta = &parsed.meta;
        let life = meta.expires_ms.saturating_sub(meta.minted_ms).max(1);
        let from = meta.minted_ms.saturating_add(life / 2);
        if self.now_ms() < from {
            return Err(BusError::bad_request(format!(
                "too early: a token is renewed in the second half of its life, from {from} (Unix ms)"
            )));
        }
        if meta.lineage.ceiling_ms != 0 && meta.lineage.ceiling_ms <= meta.expires_ms {
            return Err(BusError::bad_request(
                "the token already lives as long as the token of its minter: ask the minter for a new one",
            ));
        }
        let token = self
            .mint(
                &parsed.identity,
                &meta.rights,
                &meta.origin,
                Duration::from_millis(life),
                &meta.lineage,
                parsed.family(),
            )
            .map_err(BusError::unavailable)?;
        // The old token alone: its family (the new token) and what it minted live on.
        self.supersede_in_memory(&parsed.rids[0], meta.expires_ms.saturating_add(1000));
        let kept = self.store.persist();
        tracing::info!(target: AUDIT, "bus audit: renew from={} old={} rid={} expires_ms={}", ctx.from, parsed.rid16(), token.parsed.rid16(), token.expires_ms());
        if let Err(err) = kept {
            tracing::error!(target: "tilrd::bus", "the renewed token's revocation is not kept on disk: {err}");
        }
        // The family's id (the one `bus.token` answered): revoking it ends the new
        // token, every token of its chain and what they minted.
        Ok(Reply::encode(&wire::RenewReply {
            credential: token.bytes.clone(),
            expires_at_ms: token.expires_ms(),
            revocation_id: token.parsed.family().to_owned(),
        }))
    }

    fn method_revoke(&self, ctx: &CallCtx, payload: &Bytes) -> Result<Reply, BusError> {
        let request = wire::RevokeRequest::decode(payload.clone())
            .map_err(|err| BusError::bad_request(format!("RevokeRequest: {err}")))?;
        let max_ms = u64::try_from(MAX_TTL.as_millis()).unwrap_or(u64::MAX);
        let (id, until) = match request.what {
            Some(wire::revoke_request::What::Credential(credential)) => {
                let parsed = self
                    .verify(&credential)
                    .map_err(|denied| BusError::bad_request(denied.reason))?;
                if parsed.attenuated() {
                    // Its last block: the token and every one attenuated from it.
                    let last = parsed.rids.last().cloned().unwrap_or_default();
                    (last, parsed.meta.expires_ms.saturating_add(1000))
                } else {
                    // Its family: the token, its renewals (also one made from it
                    // before), what is attenuated from them and what they minted.
                    (parsed.family().to_owned(), self.family_until())
                }
            }
            Some(wire::revoke_request::What::RevocationId(id)) => {
                if !is_hex(&id) {
                    return Err(BusError::bad_request(
                        "revocation_id is lowercase hex, as TokenReply gives it",
                    ));
                }
                (
                    id,
                    self.now_ms().saturating_add(max_ms).saturating_add(1000),
                )
            }
            None => {
                return Err(BusError::bad_request(
                    "name a credential or a revocation_id",
                ));
            }
        };
        let kept = self.revoke_ids(&[(id.clone(), until)]);
        tracing::info!(target: AUDIT, "bus audit: revoke from={} rid={}", ctx.from, &id[..id.len().min(16)]);
        lock(&self.instances).retain(|rid, seat| *rid != id && seat.family != id);
        self.close_holders(&[id]);
        kept.map_err(|err| {
            BusError::unavailable(format!(
                "revoked until this server stops, but not kept on disk: {err}"
            ))
        })?;
        Ok(Reply::encode(&wire::RevokeReply {}))
    }

    fn method_rotate_key(&self, ctx: &CallCtx) -> Result<Reply, BusError> {
        self.rotate().map_err(|err| {
            tracing::info!(target: AUDIT, "bus audit: rotate_key from={} failed: {err}", ctx.from);
            BusError::unavailable(err)
        })?;
        tracing::info!(target: AUDIT, "bus audit: rotate_key from={}", ctx.from);
        let Some(broker) = self.broker.get() else {
            return Ok(Reply::encode(&wire::RotateKeyReply {}));
        };
        for conn in broker.connections() {
            let Some(principal) = broker.principal_of(conn.connection_id) else {
                continue;
            };
            match (&principal.identity, token_of(&principal)) {
                (Identity::Instance(_), _) => {
                    self.renew_instance(broker, conn.connection_id, &principal);
                }
                (_, Some(token)) => {
                    tracing::info!(target: AUDIT, "bus audit: close from={} rid={} reason=key rotated", principal.identity, token.parsed.rid16());
                    broker.close_connection(
                        conn.connection_id,
                        tilr_bus::close::CREDENTIAL,
                        tilr_bus::close::CREDENTIAL_REASON,
                    );
                }
                _ => {}
            }
        }
        Ok(Reply::encode(&wire::RotateKeyReply {}))
    }
}

impl Authorizer for Acl {
    fn hello(&self, hello: &Hello, conn: &HelloContext<'_>) -> Result<Principal, ConnectionError> {
        let principal = |identity: Identity, rights, credential| Principal {
            identity,
            rights,
            credential,
            origin: conn.origin.to_owned(),
            transport: conn.transport,
            client_id: hello.client_id.clone(),
            client_name: hello.client_name.clone(),
            connection_id: conn.connection_id,
        };
        if let Some(seat) = conn.seat {
            if !hello.credential.is_empty() {
                return Err(denied(
                    "an instance gets its token from the server, not in Hello.credential",
                ));
            }
            let rights = seat
                .rights
                .as_deref()
                .map(RightsText::from)
                .unwrap_or_default();
            let token = match self.mint_instance(
                &seat.identity,
                &rights,
                conn.connection_id,
                false,
                "",
            ) {
                Ok(token) => {
                    tracing::info!(target: AUDIT, "bus audit: mint from=tilrd participant={} rid={} expires_ms={}", seat.identity, token.parsed.rid16(), token.expires_ms());
                    Some(Arc::new(token) as Arc<dyn Any + Send + Sync>)
                }
                Err(err) => {
                    // Fail closed: the instance runs and may do nothing.
                    tracing::error!(target: "tilrd::bus", identity = %seat.identity, "no token for the instance, it is allowed nothing: {err}");
                    None
                }
            };
            return Ok(principal(seat.identity.clone(), seat.rights.clone(), token));
        }
        if !hello.instance_token.is_empty() {
            return Err(denied(
                "an instance token is taken on an instance socket only",
            ));
        }
        if matches!(
            conn.transport,
            Transport::InstanceFd | Transport::InstanceSocket
        ) {
            return Err(denied("an instance stream needs its seat"));
        }
        if !hello.credential.is_empty() {
            let parsed = self
                .verify(&hello.credential)
                .map_err(|refused| denied(refused.reason))?;
            if !matches!(parsed.identity, Identity::Token(_)) {
                return Err(denied(
                    "an instance's token is presented per call, not in Hello",
                ));
            }
            self.usable(&parsed, conn.origin, conn.connection_id)
                .map_err(|refused| denied(refused.reason))?;
            tracing::info!(target: AUDIT, "bus audit: hello from={} rid={} connection={}", parsed.identity, parsed.rid16(), conn.connection_id);
            let token = Token {
                bytes: hello.credential.clone(),
                parsed: Arc::clone(&parsed),
            };
            return Ok(principal(
                parsed.identity.clone(),
                None,
                Some(Arc::new(token) as Arc<dyn Any + Send + Sync>),
            ));
        }
        match (conn.transport, &self.access_token) {
            // The Unix socket is this user's alone: no token there.
            (Transport::UnixSocket, _) | (Transport::Tcp, None) => {
                Ok(principal(Identity::Owner, None, None))
            }
            (Transport::Tcp, Some(_)) if hello.access_token.is_empty() => Err(denied(
                "this server asks for its access token or a credential",
            )),
            (Transport::Tcp, Some(token)) => {
                if crate::auth::matches(token, &hello.access_token) {
                    Ok(principal(Identity::Owner, None, None))
                } else {
                    Err(denied("wrong access token"))
                }
            }
            (Transport::InstanceFd | Transport::InstanceSocket, _) => {
                Err(denied("an instance stream needs its seat"))
            }
        }
    }

    fn authorize(
        &self,
        principal: &Principal,
        call_credential: Option<&[u8]>,
        request: &AuthzRequest<'_>,
    ) -> Result<(), Denied> {
        let bearer = call_credential.filter(|credential| !credential.is_empty());
        let facts = request_facts(request);
        let mut token: Option<Arc<Parsed>> = None;
        let result = (|| {
            let facts = facts.as_ref().map_err(Clone::clone)?;
            if let AuthzRequest::Call {
                method: "server.command",
                ..
            } = request
            {
                // No token grants it: the owner's (or tilrd's) on the Unix socket only.
                return if bearer.is_none()
                    && principal.identity.is_privileged()
                    && principal.transport == Transport::UnixSocket
                {
                    Ok(())
                } else {
                    Err(Denied::new(
                        "server.command is the owner's on the Unix socket only",
                    ))
                };
            }
            if let Some(bytes) = bearer {
                // A call, or a control or pane check a core method makes for that
                // call (`CallCtx::call_credential`): the bearer decides alone (R4).
                if !matches!(
                    request,
                    AuthzRequest::Call { .. }
                        | AuthzRequest::Control { .. }
                        | AuthzRequest::PaneCreate { .. }
                        | AuthzRequest::PaneAccess { .. }
                ) {
                    return Err(Denied::new(
                        "a per-call credential authorizes a call and its control and pane checks only",
                    ));
                }
                let parsed = self.verify(bytes)?;
                token = Some(Arc::clone(&parsed));
                return self.run(&parsed, principal, facts);
            }
            if principal.identity.is_privileged() {
                return Ok(());
            }
            let Some(held) = token_of(principal) else {
                return Err(Denied::new(format!(
                    "{} holds no token of this server",
                    principal.identity
                )));
            };
            token = Some(Arc::clone(&held.parsed));
            self.run(&held.parsed, principal, facts)
        })();
        self.audit(
            principal,
            token.as_deref(),
            bearer.is_some(),
            facts.as_ref(),
            &result,
        );
        result
    }

    fn hello_ack_credential(&self, principal: &Principal) -> Option<Bytes> {
        match principal.identity {
            Identity::Instance(_) => token_of(principal).map(|token| token.bytes.clone()),
            _ => None,
        }
    }

    fn credential_update(
        &self,
        principal: &Principal,
        credential: &[u8],
    ) -> Result<Principal, ConnectionError> {
        let refuse = |reason: String| {
            tracing::info!(target: AUDIT, "bus audit: deny from={} kind=credential_update reason={reason}", principal.identity);
            denied(reason)
        };
        match principal.identity {
            Identity::Token(_) => {}
            Identity::Instance(_) => {
                return Err(refuse(
                    "an instance's token is renewed by the server".into(),
                ));
            }
            _ => return Err(refuse(format!("{} holds no token", principal.identity))),
        }
        let parsed = self
            .verify(credential)
            .map_err(|refused| refuse(refused.reason))?;
        if parsed.identity != principal.identity {
            return Err(refuse(format!(
                "the new token names {}, not {}",
                parsed.identity, principal.identity
            )));
        }
        self.usable(&parsed, &principal.origin, principal.connection_id)
            .map_err(|refused| refuse(refused.reason))?;
        tracing::info!(target: AUDIT, "bus audit: credential_update from={} rid={}", principal.identity, parsed.rid16());
        Ok(Principal {
            credential: Some(Arc::new(Token {
                bytes: Bytes::copy_from_slice(credential),
                parsed,
            }) as Arc<dyn Any + Send + Sync>),
            ..principal.clone()
        })
    }
}

// ---------------------------------------------------------------------------
// Files
// ---------------------------------------------------------------------------

/// The root key in `dir`, made (0600) if there is none. An existing file is never
/// replaced here: one that does not read, is not the user's or is open to others is
/// an error (every token is refused then).
fn load_or_make_key(dir: &Path) -> anyhow::Result<KeyPair> {
    use anyhow::Context;
    crate::identity::create_private_dir(dir)?;
    let path = dir.join(ROOT_KEY_FILE);
    if !path.exists() {
        let pair = KeyPair::new();
        // Written aside and linked into place: the file appears whole, and a
        // server starting at the same moment reads the key of the other.
        let staged = staged_path(&path);
        write_private(&staged, &key_text(&pair))?;
        let linked = std::fs::hard_link(&staged, &path);
        let _ = std::fs::remove_file(&staged);
        match linked {
            Err(err) if err.kind() != std::io::ErrorKind::AlreadyExists => {
                return Err(err).with_context(|| format!("create {}", path.display()));
            }
            _ => sync_dir(dir)?,
        }
    }
    read_key(&path)
}

/// The key file's text; wiped from memory when dropped.
fn key_text(pair: &KeyPair) -> Zeroizing<String> {
    let hex = Zeroizing::new(pair.private().to_bytes_hex());
    let mut text = Zeroizing::new(String::with_capacity(hex.len() + 9));
    text.push_str("ed25519/");
    text.push_str(&hex);
    text.push('\n');
    text
}

fn read_key(path: &Path) -> anyhow::Result<KeyPair> {
    use anyhow::Context;
    use std::os::unix::fs::MetadataExt;
    let meta =
        std::fs::symlink_metadata(path).with_context(|| format!("stat {}", path.display()))?;
    // SAFETY: geteuid has no preconditions.
    if !meta.is_file() || meta.uid() != unsafe { libc::geteuid() } {
        anyhow::bail!("{} is not a file of this user", path.display());
    }
    if meta.mode() & 0o077 != 0 {
        anyhow::bail!(
            "{} is open to others (mode {:o}); it must be 0600",
            path.display(),
            meta.mode() & 0o777
        );
    }
    let text = Zeroizing::new(
        std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?,
    );
    let key = text
        .trim()
        .strip_prefix("ed25519/")
        .and_then(unhex)
        .and_then(|bytes| PrivateKey::from_bytes(&bytes, Algorithm::Ed25519).ok())
        .with_context(|| format!("{} does not hold an Ed25519 key", path.display()))?;
    Ok(KeyPair::from(&key))
}

/// The bytes of lowercase or uppercase hex, wiped from memory when dropped (the
/// root key passes through it).
fn unhex(text: &str) -> Option<Zeroizing<Vec<u8>>> {
    let digit = |b: u8| (b as char).to_digit(16);
    if !text.len().is_multiple_of(2) {
        return None;
    }
    let mut bytes = Zeroizing::new(Vec::with_capacity(text.len() / 2));
    for [high, low] in text.as_bytes().as_chunks::<2>().0 {
        let byte = digit(*high)? * 16 + digit(*low)?;
        bytes.push(u8::try_from(byte).ok()?);
    }
    Some(bytes)
}

/// Replace the key file whole (rotation).
fn write_key(path: &Path, pair: &KeyPair) -> anyhow::Result<()> {
    replace_private(path, &key_text(pair))
}

/// A name beside `path` no other writer uses: the process, a counter and the
/// time, so neither a thread nor another server removes or reuses it.
fn staged_path(path: &Path) -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.subsec_nanos());
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(format!(
        ".{}.{}.{nanos}.tmp",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    path.with_file_name(name)
}

/// Write `text` to a new file of mode 0600 and sync it; a file of that name is an
/// error, never removed.
fn write_private(path: &Path, text: &str) -> anyhow::Result<()> {
    use anyhow::Context;
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .with_context(|| format!("create {}", path.display()))?;
    file.write_all(text.as_bytes())
        .and_then(|()| file.sync_all())
        .with_context(|| format!("write {}", path.display()))
}

/// Replace `path` whole with `text` (0600): written and synced aside, renamed into
/// place, the directory synced.
fn replace_private(path: &Path, text: &str) -> anyhow::Result<()> {
    use anyhow::Context;
    let staged = staged_path(path);
    if let Err(err) = write_private(&staged, text) {
        let _ = std::fs::remove_file(&staged);
        return Err(err);
    }
    if let Err(err) = std::fs::rename(&staged, path) {
        let _ = std::fs::remove_file(&staged);
        return Err(err).with_context(|| format!("replace {}", path.display()));
    }
    sync_dir(path.parent().unwrap_or(Path::new(".")))
}

/// Make a rename or link in `dir` durable.
fn sync_dir(dir: &Path) -> anyhow::Result<()> {
    use anyhow::Context;
    std::fs::File::open(dir)
        .and_then(|dir| dir.sync_all())
        .with_context(|| format!("sync {}", dir.display()))
}

/// The revocation ids still kept at `now`: one `<hex id> <until ms>` per line, with
/// ` renewed` after a renewed token's ([`Revoked::renewed`]). A missing file is
/// none; one that does not read is an error.
fn load_revoked(path: &Path, now: u64) -> anyhow::Result<Revoked> {
    use anyhow::Context;
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Revoked::default()),
        Err(err) => return Err(err).with_context(|| format!("read {}", path.display())),
    };
    let mut revoked = Revoked::default();
    for (n, line) in text.lines().enumerate() {
        let mut fields = line.split_whitespace();
        let (Some(id), until, kind, None) =
            (fields.next(), fields.next(), fields.next(), fields.next())
        else {
            if line.trim().is_empty() {
                continue;
            }
            anyhow::bail!("{} line {}: not `<id> <until ms>`", path.display(), n + 1);
        };
        let until = until.and_then(|until| until.parse::<u64>().ok());
        let set = match kind {
            None => &mut revoked.ids,
            Some("renewed") => &mut revoked.renewed,
            Some(_) => anyhow::bail!("{} line {}: not `<id> <until ms>`", path.display(), n + 1),
        };
        let Some(until) = until.filter(|_| is_hex(id)) else {
            anyhow::bail!("{} line {}: not `<id> <until ms>`", path.display(), n + 1);
        };
        if until >= now {
            set.insert(id.to_owned(), until);
        }
    }
    Ok(revoked)
}

/// `(id, until, renewed)` lines, as [`load_revoked`] reads them.
fn write_revoked(path: &Path, ids: &[(String, u64, bool)]) -> anyhow::Result<()> {
    let mut text = String::new();
    for (id, until, renewed) in ids {
        let kind = if *renewed { " renewed" } else { "" };
        text.push_str(&format!("{id} {until}{kind}\n"));
    }
    replace_private(path, &text)
}

// ---------------------------------------------------------------------------
// The methods and the sweeper
// ---------------------------------------------------------------------------

/// The [`Acl`] the broker was made with, if it is one.
fn acl_of(broker: &BrokerHandle) -> Option<Arc<Acl>> {
    let wanted = Arc::as_ptr(broker.authorizer()).cast::<()>();
    lock(&MADE)
        .iter()
        .filter_map(Weak::upgrade)
        .find(|acl| Arc::as_ptr(acl).cast::<()>() == wanted)
}

/// `bus.reload_config` applied `bus.toml`: every cached decision is dropped (R7).
/// Nothing with another authorizer.
pub fn config_reloaded(broker: &BrokerHandle) {
    if let Some(acl) = acl_of(broker) {
        lock(&acl.decisions).clear();
    }
}

/// Add `bus.token`, `bus.renew`, `bus.revoke`, `bus.rotate_key` and start the
/// sweeper (expiry, instance renewal, ended instances). Nothing with another
/// authorizer.
pub fn register(registry: &mut Registry, core: &Core) {
    let Some(acl) = acl_of(&core.broker) else {
        return;
    };
    if acl.broker.set(core.broker.clone()).is_err() {
        tracing::error!(target: "tilrd::bus", "the bus authorizer was registered twice");
        return;
    }
    if let Ok(runtime) = tokio::runtime::Handle::try_current() {
        let weak = Arc::downgrade(&acl);
        let broker = core.broker.clone();
        runtime.spawn(async move {
            let mut tick = tokio::time::interval(SWEEP);
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tick.tick().await;
                let Some(acl) = weak.upgrade() else {
                    return;
                };
                acl.sweep(&broker);
            }
        });
    }
    for (name, f) in [
        (
            tilr_bus::methods::BUS_TOKEN.name,
            Acl::method_token as MethodFn,
        ),
        (tilr_bus::methods::BUS_RENEW.name, Acl::method_renew),
        (tilr_bus::methods::BUS_REVOKE.name, Acl::method_revoke),
        (tilr_bus::methods::BUS_ROTATE_KEY.name, |acl, ctx, _| {
            acl.method_rotate_key(ctx)
        }),
    ] {
        registry.add(blocking_method(&acl, name, f));
    }
}

/// The body of a method of this module.
type MethodFn = fn(&Acl, &CallCtx, &Bytes) -> Result<Reply, BusError>;

/// A native method whose body runs on a blocking thread: nothing of it on the
/// broker's envelope path (it writes and fsyncs files). The reply follows the
/// body, so what it revoked is revoked before the caller hears back.
fn blocking_method(acl: &Arc<Acl>, name: &str, f: MethodFn) -> NativeMethod {
    let acl = Arc::clone(acl);
    NativeMethod::unary(name, move |ctx, payload| {
        let acl = Arc::clone(&acl);
        async move {
            tokio::task::spawn_blocking(move || f(&acl, &ctx, &payload))
                .await
                .unwrap_or_else(|_| Err(BusError::handler_error("the method failed")))
        }
    })
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;
    use crate::bus::identity::{CancelToken, Peer, Seat};
    use crate::bus::streams::NativeHandler;

    /// The unit tests' clock (Unix ms), one per process: tests that move it run
    /// under [`CLOCK_LOCK`].
    static NOW: AtomicU64 = AtomicU64::new(1_800_000_000_000);
    static CLOCK_LOCK: Mutex<()> = Mutex::new(());

    fn now() -> u64 {
        NOW.load(Ordering::SeqCst)
    }

    struct Fixture {
        dir: PathBuf,
        acl: Arc<Acl>,
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    fn fixture(server_id: &str) -> Fixture {
        static N: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "tilrd-acl-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let acl = Acl::new(
            &AuthzConfig {
                access_token: Some("secret".into()),
                server_id: server_id.into(),
                state_dir: Some(dir.clone()),
            },
            now,
        );
        Fixture {
            dir,
            acl: Arc::new(acl),
        }
    }

    fn rights(f: impl FnOnce(&mut RightsText)) -> RightsText {
        let mut rights = RightsText::default();
        f(&mut rights);
        rights
    }

    fn strings(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    fn holder(token: Token) -> Principal {
        Principal {
            identity: token.identity().clone(),
            credential: Some(Arc::new(token)),
            connection_id: 7,
            ..Principal::tilrd()
        }
    }

    fn call<'a>(method: &'a str, target: &'a str) -> AuthzRequest<'a> {
        AuthzRequest::Call { method, target }
    }

    fn minted(f: &Fixture, rights: &RightsText) -> Token {
        f.acl
            .mint(
                &Identity::Token("t-1".into()),
                rights,
                "",
                MAX_TTL,
                &Lineage::default(),
                "",
            )
            .unwrap()
    }

    #[test]
    fn the_root_key_file_is_private_and_kept() {
        use std::os::unix::fs::PermissionsExt;
        let f = fixture("s");
        let path = f.dir.join(ROOT_KEY_FILE);
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        let first = std::fs::read(&path).unwrap();
        let again = Acl::new(
            &AuthzConfig {
                state_dir: Some(f.dir.clone()),
                ..AuthzConfig::default()
            },
            now,
        );
        assert_eq!(std::fs::read(&path).unwrap(), first);
        assert!(again.keys().pair.is_some());
        // Open to others: refused, not replaced.
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        let refused = Acl::new(
            &AuthzConfig {
                state_dir: Some(f.dir.clone()),
                ..AuthzConfig::default()
            },
            now,
        );
        assert!(refused.keys().pair.is_none());
        assert_eq!(std::fs::read(&path).unwrap(), first);
        let error = refused
            .verify(&minted(&f, &RightsText::default()).bytes)
            .unwrap_err();
        assert!(error.reason.contains("cannot verify tokens"), "{error}");
    }

    #[test]
    fn the_root_key_reads_from_its_hex() {
        assert_eq!(
            unhex("00ff7A").as_deref().map(Vec::as_slice),
            Some(&[0, 255, 122][..])
        );
        assert!(unhex("abc").is_none());
        assert!(unhex("zz").is_none());
        let f = fixture("s");
        let read = read_key(&f.dir.join(ROOT_KEY_FILE)).unwrap();
        let pair = f.acl.keys().pair.as_ref().unwrap().public();
        assert_eq!(read.public(), pair);
    }

    #[test]
    fn each_right_allows_its_requests_only() {
        let f = fixture("s");
        let token = minted(
            &f,
            &rights(|r| {
                r.call = strings(&["test.*", "math.add"]);
                r.provide = strings(&["ui.*"]);
                r.emit = strings(&["deno.ready"]);
                r.subscribe = strings(&["pane.*"]);
                r.control = strings(&["list", "reload:t-*", "stop:t-9"]);
            }),
        );
        let p = holder(token);
        let ok = |request: AuthzRequest<'_>| f.acl.authorize(&p, None, &request).is_ok();
        assert!(ok(call("test.echo", "")));
        assert!(ok(call("math.add", "")));
        assert!(!ok(call("math.sub", "")));
        assert!(!ok(call("other.echo", "")));
        assert!(ok(AuthzRequest::Provide {
            method: "ui.notify"
        }));
        assert!(!ok(AuthzRequest::Provide {
            method: "test.echo"
        }));
        assert!(ok(AuthzRequest::Emit {
            subject: "deno.ready"
        }));
        assert!(!ok(AuthzRequest::Emit {
            subject: "deno.other"
        }));
        assert!(ok(AuthzRequest::Subscribe { pattern: "pane.*" }));
        assert!(ok(AuthzRequest::Subscribe {
            pattern: "pane.bell"
        }));
        assert!(!ok(AuthzRequest::Subscribe { pattern: "*.>" }));
        assert!(!ok(AuthzRequest::Subscribe { pattern: "test.*" }));
        assert!(ok(call("bus.instances", "")));
        assert!(ok(call("bus.methods", "")));
        assert!(ok(call("bus.reload", "")));
        assert!(ok(call("bus.stop", "")));
        assert!(!ok(call("bus.start", "")));
        assert!(!ok(call("bus.token", "")));
        assert!(!ok(call("bus.rotate_key", "")));
        assert!(!ok(call("server.reload", "")));
        assert!(!ok(call("bus.nothing", "")));
        assert!(ok(call("bus.ping", "")));
        let control = |action, instance| AuthzRequest::Control {
            action,
            instance,
            call: ForCall::default(),
        };
        assert!(ok(control("reload", Some("t-1"))));
        assert!(!ok(control("reload", Some("u-1"))));
        assert!(!ok(control("reload", None)));
        assert!(ok(control("stop", Some("t-9"))));
        assert!(!ok(control("stop", Some("t-8"))));
        assert!(ok(control("list", None)));
        let denied = f
            .acl
            .authorize(&p, None, &call("other.echo", ""))
            .unwrap_err();
        assert_eq!(denied.reason, "token:t-1 has no right to call other.echo");
    }

    #[test]
    fn an_action_over_every_instance_is_the_unscoped_right() {
        let f = fixture("s");
        let ok = |control: &[&str], request: AuthzRequest<'_>| {
            let p = holder(minted(&f, &rights(|r| r.control = strings(control))));
            f.acl.authorize(&p, None, &request).is_ok()
        };
        // As `list` and `reload_config`: over every instance, also with none.
        assert!(ok(&["list:*"], call("bus.instances", "")));
        assert!(ok(&["list:*"], call("bus.methods", "")));
        assert!(ok(&["reload_config:*"], call("bus.reload_config", "")));
        assert!(!ok(&["list:*"], call("bus.reload_config", "")));
        assert!(ok(
            &["reload:*"],
            AuthzRequest::Control {
                action: "reload",
                instance: Some("any-1"),
                call: ForCall::default(),
            }
        ));
        // `*:*` is `*`; a prefix is still some instances only.
        assert!(ok(&["*:*"], call("bus.rotate_key", "")));
        assert!(!ok(&["list:t-*"], call("bus.instances", "")));
        assert!(check_rights(&rights(|r| r.control = strings(&["list:*", "*:*"]))).is_ok());
    }

    #[test]
    fn stars_and_the_owner() {
        let f = fixture("s");
        let token = minted(
            &f,
            &rights(|r| {
                r.call = strings(&["*"]);
                r.emit = strings(&["*"]);
                r.subscribe = strings(&["*"]);
                r.control = strings(&["*"]);
            }),
        );
        let p = holder(token);
        let ok = |request: AuthzRequest<'_>| f.acl.authorize(&p, None, &request).is_ok();
        assert!(ok(call("anything.at_all", "")));
        // `*` is not bus.*; control is.
        assert!(ok(call("bus.rotate_key", "")));
        assert!(ok(AuthzRequest::Emit {
            subject: "my.event"
        }));
        // Reserved namespaces are tilrd's alone.
        assert!(!ok(AuthzRequest::Emit {
            subject: "pane.exited"
        }));
        assert!(ok(AuthzRequest::Subscribe { pattern: ">" }));
        let call_only = holder(minted(&f, &rights(|r| r.call = strings(&["*"]))));
        assert!(
            f.acl
                .authorize(&call_only, None, &call("bus.instances", ""))
                .is_err()
        );
        // Nothing without a token; the owner everything.
        let bare = Principal {
            identity: Identity::Instance("t-1".into()),
            ..Principal::tilrd()
        };
        assert!(
            f.acl
                .authorize(&bare, None, &call("test.echo", ""))
                .is_err()
        );
        let owner = Principal {
            identity: Identity::Owner,
            ..Principal::tilrd()
        };
        assert!(
            f.acl
                .authorize(&owner, None, &call("bus.rotate_key", ""))
                .is_ok()
        );
    }

    #[test]
    fn a_per_call_credential_decides_alone() {
        let f = fixture("s");
        let narrow = minted(&f, &rights(|r| r.call = strings(&["test.echo"])));
        let wide = holder(minted(&f, &rights(|r| r.call = strings(&["*"]))));
        let owner = Principal {
            identity: Identity::Owner,
            ..Principal::tilrd()
        };
        let bare = Principal {
            identity: Identity::Instance("t-1".into()),
            ..Principal::tilrd()
        };
        for p in [&wide, &owner] {
            assert!(
                f.acl
                    .authorize(p, Some(&narrow.bytes), &call("test.whoami", ""))
                    .is_err()
            );
        }
        assert!(
            f.acl
                .authorize(&bare, Some(&narrow.bytes), &call("test.echo", ""))
                .is_ok()
        );
        // Not for anything but a call.
        assert!(
            f.acl
                .authorize(
                    &owner,
                    Some(&narrow.bytes),
                    &AuthzRequest::Subscribe { pattern: "x.y" }
                )
                .is_err()
        );
    }

    #[test]
    fn appended_blocks_narrow_and_never_widen() {
        let f = fixture("s");
        let token = minted(&f, &rights(|r| r.call = strings(&["test.*"])));
        let credential = tilr_bus::Credential::from_bytes(token.bytes.clone());
        let narrowed = credential
            .attenuate(r#"check if target("instance:t-2");"#)
            .unwrap();
        let owner = Principal {
            identity: Identity::Owner,
            ..Principal::tilrd()
        };
        let bearer = narrowed.as_bytes().as_ref();
        assert!(
            f.acl
                .authorize(&owner, Some(bearer), &call("test.echo", "instance:t-2"))
                .is_ok()
        );
        let refused = f
            .acl
            .authorize(&owner, Some(bearer), &call("test.echo", "instance:t-3"))
            .unwrap_err();
        assert!(refused.reason.contains("a check of the token"), "{refused}");
        let widened = credential
            .attenuate(r#"call_any(true); call("other.echo"); control_all(true);"#)
            .unwrap();
        for method in ["other.echo", "bus.rotate_key"] {
            assert!(
                f.acl
                    .authorize(&owner, Some(widened.as_bytes()), &call(method, ""))
                    .is_err(),
                "{method}"
            );
        }
        // Nine blocks: refused.
        let mut long = credential.clone();
        for _ in 0..MAX_BLOCKS {
            long = long.attenuate("check if true;").unwrap();
        }
        let refused = f
            .acl
            .authorize(&owner, Some(long.as_bytes()), &call("test.echo", ""))
            .unwrap_err();
        assert!(refused.reason.contains("blocks"), "{refused}");
        // A rule (that would make too many facts): refused before Datalog runs.
        let heavy = credential
            .attenuate(
                "n(0); n(1); n(2); n(3); n(4); n(5); n(6); n(7); n(8); n(9);
                 p($a, $b, $c) <- n($a), n($b), n($c);
                 check if p(0, 0, 0);",
            )
            .unwrap();
        let refused = f
            .acl
            .authorize(&owner, Some(heavy.as_bytes()), &call("test.echo", ""))
            .unwrap_err();
        assert!(refused.reason.contains("checks only"), "{refused}");
        // Garbage and another server's token.
        assert!(
            f.acl
                .authorize(&owner, Some(b"garbage"), &call("test.echo", ""))
                .is_err()
        );
        let other = fixture("s");
        let foreign = minted(&other, &rights(|r| r.call = strings(&["*"])));
        let refused = f
            .acl
            .authorize(&owner, Some(&foreign.bytes), &call("test.echo", ""))
            .unwrap_err();
        assert_eq!(refused.reason, "not a token of this server");
        // Blocks are counted before a signature is checked: nine blocks of another
        // key are refused for their count.
        let mut long = tilr_bus::Credential::from_bytes(foreign.bytes.clone());
        for _ in 0..MAX_BLOCKS {
            long = long.attenuate("check if true;").unwrap();
        }
        let refused = f
            .acl
            .authorize(&owner, Some(long.as_bytes()), &call("test.echo", ""))
            .unwrap_err();
        assert!(refused.reason.contains("blocks"), "{refused}");
    }

    /// `f` on another thread, waited for `WAIT` at most: what it answers, or `None`
    /// when it is still running (it is left to run out).
    fn within<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Option<T> {
        const WAIT: Duration = Duration::from_secs(5);
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(f());
        });
        rx.recv_timeout(WAIT).ok()
    }

    #[test]
    fn an_appended_block_holds_bounded_checks_only() {
        let f = fixture("s");
        let owner = Principal {
            identity: Identity::Owner,
            ..Principal::tilrd()
        };
        // 40 rights: a join over them in one check is 40^5 bindings.
        let many: Vec<String> = (0..40).map(|i| format!("m.x{i}")).collect();
        let token = minted(&f, &rights(|r| r.call = many));
        let credential = tilr_bus::Credential::from_bytes(token.bytes.clone());
        let attenuate = |block: &str| credential.attenuate(block).unwrap().as_bytes().clone();
        let mut join = String::new();
        for i in 0..40 {
            join.push_str(&format!("f({i});\n"));
        }
        join.push_str(
            "check if f($a), f($b), f($c), f($d), f($e), $a + $b + $c + $d + $e == -1;\n",
        );
        let refused = [
            // Facts of its own and a 5-way join over them (the 86 s token).
            attenuate(&join),
            // A join over the authority block's facts.
            attenuate(
                "check if call($a), call($b), call($c), call($d), call($e),
                 $a.length() + $b.length() + $c.length() + $d.length() + $e.length() == -1;",
            ),
            // Nested closures over arrays it writes itself.
            attenuate(&format!(
                "check if [{list}].all($x -> [{list}].all($y -> [{list}].all($z -> $x + $y + $z != -1)));",
                list = (0..200)
                    .map(|i| i.to_string())
                    .collect::<Vec<_>>()
                    .join(",")
            )),
            // A fact, a rule.
            attenuate("call_any(true);"),
            attenuate("p($m) <- method($m); check if p(\"m.x1\");"),
            // A regex, compiled on every decision.
            attenuate(r#"check if method($m), $m.matches("^m\\.x[0-9]+$");"#),
            // Too many checks in one block.
            attenuate(&"check if true;\n".repeat(MAX_BLOCK_CHECKS + 1)),
        ];
        for (n, bytes) in refused.into_iter().enumerate() {
            let acl = Arc::clone(&f.acl);
            let owner = owner.clone();
            let refused = within(move || {
                acl.authorize(&owner, Some(&bytes), &call("m.x1", ""))
                    .map_err(|denied| denied.reason)
            })
            .unwrap_or_else(|| panic!("block {n}: still deciding after 5 s"));
            let reason = refused.expect_err("refused");
            assert!(
                reason.starts_with("an appended block"),
                "block {n}: {reason}"
            );
        }
        // Checks on the request's facts are what a holder narrows with.
        for block in [
            r#"check if target("instance:t-2") or kind("subscribe");"#,
            r#"check if kind($k), method($m), target($t), caller($c), origin($o), server($s), $k == "call";"#,
            r#"check if namespace("bus") or instance($i), $i.starts_with("s-");"#,
            r#"check if method("m.x1") or kind("control");"#,
            r#"check if time($t), $t <= 2100-01-01T00:00:00Z;"#,
            r#"reject if method($m), [ "m.x2", "m.x3" ].contains($m);"#,
            "check if true || false;",
        ] {
            let bytes = attenuate(block);
            assert!(f.acl.verify(&bytes).is_ok(), "{block}");
        }
        let narrowed = attenuate(r#"check if target("instance:t-2");"#);
        let narrowed_call = |target| {
            f.acl
                .authorize(&owner, Some(&narrowed), &call("m.x1", target))
        };
        assert!(narrowed_call("instance:t-2").is_ok());
        assert!(narrowed_call("instance:t-3").is_err());
    }

    /// `bytes` with `block` appended through Biscuit's builder API (no source text).
    fn appended(bytes: &[u8], block: biscuit_auth::BlockBuilder) -> Vec<u8> {
        UnverifiedBiscuit::from(bytes)
            .unwrap()
            .append(block)
            .unwrap()
            .to_vec()
            .unwrap()
    }

    /// The block whose symbol names carry source text: printed, it reads as two
    /// checks on `kind`; what it holds is `n` facts of its own, a rule with a 5-way
    /// join over them, and a check reading a predicate no request has.
    fn hidden_join_block(n: usize) -> biscuit_auth::BlockBuilder {
        use biscuit_auth::builder::{self as d, Binary, Expression, Op};
        let v = d::variable;
        let mut block = biscuit_auth::BlockBuilder::new()
            .fact(d::fact(r#"check if kind("bus");/*f"#, &[d::string("x")]))
            .unwrap();
        for i in 0..n {
            block = block
                .fact(d::fact("f", &[d::string(&i.to_string())]))
                .unwrap();
        }
        let body = ["a", "b", "c", "d", "e"].map(|x| d::pred("f", &[v(x)]));
        let never = Expression {
            ops: vec![
                Op::Value(v("a")),
                Op::Value(d::string("never")),
                Op::Binary(Binary::Equal),
            ],
        };
        block
            .rule(d::Rule::new(
                d::pred("g", &[v("a")]),
                body.to_vec(),
                vec![never],
                vec![],
            ))
            .unwrap()
            .check(check_of(
                d::pred("X*/check if kind", &[d::string("bus")]),
                vec![],
            ))
            .unwrap()
    }

    /// `check if <predicate>` with `scopes`.
    fn check_of(
        predicate: biscuit_auth::builder::Predicate,
        scopes: Vec<biscuit_auth::builder::Scope>,
    ) -> biscuit_auth::builder::Check {
        use biscuit_auth::builder as d;
        d::Check {
            queries: vec![d::Rule::new(
                d::pred("query", &[] as &[d::Term]),
                vec![predicate],
                vec![],
                scopes,
            )],
            kind: d::CheckKind::One,
        }
    }

    #[test]
    fn only_exact_names_are_control_methods() {
        let action = |request: &Request<'_>| {
            request
                .facts
                .iter()
                .any(|fact| fact.predicate.name == "action")
        };
        // Look-alikes of control methods, and unknown `bus.*` methods: plain calls,
        // without an action, which no participant provides.
        for method in [
            "bus.absent",
            "bus.Token",
            "BUS.token",
            "bus.token.",
            "bus.token ",
            " bus.token",
            "bus.token\0",
            "bus..token",
            "bus.t\u{043e}ken",
            "bus.rotate-key",
            "bus.reload_config.x",
            "server.Reload",
        ] {
            let request = request_facts(&call(method, "")).unwrap();
            assert_eq!(request.kind, "call", "{method:?}");
            assert!(!action(&request), "{method:?}");
            assert_eq!(request.what, format!("call {method}"));
        }
        // Every core method of `bus` and `server` but those any participant may call
        // or no right reaches is a control method: one added to the core without
        // its action would be a plain call that `call = ["bus.*"]` reaches.
        for method in tilr_bus::methods::CORE.iter().map(|m| m.name) {
            let open = ["bus.ping", "bus.renew", "server.info", "server.command"];
            if !(method.starts_with("bus.") || method.starts_with("server."))
                || open.contains(&method)
            {
                continue;
            }
            let request = request_facts(&call(method, "")).unwrap();
            assert!(request.kind.starts_with("control"), "{method}");
            assert!(action(&request), "{method}");
        }
        // Authorized as a call: by the owner and by `call` rights over it; a control
        // method stays its action's.
        let f = fixture("s");
        assert!(
            f.acl
                .authorize(
                    &owner_on(Transport::UnixSocket),
                    None,
                    &call("bus.absent", "")
                )
                .is_ok()
        );
        let ok = |call_rights: &[&str], method: &str| {
            let p = holder(minted(&f, &rights(|r| r.call = strings(call_rights))));
            f.acl.authorize(&p, None, &call(method, "")).is_ok()
        };
        assert!(ok(&["bus.*"], "bus.absent"));
        assert!(ok(&["bus.*"], "bus.Token"));
        assert!(ok(&["*"], "bus.absent"));
        assert!(ok(&["bus.absent"], "bus.absent"));
        assert!(!ok(&["test.*"], "bus.absent"));
        assert!(!ok(&[], "bus.absent"));
        assert!(!ok(&["bus.*"], "bus.token"));
        assert!(!ok(&["*"], "bus.rotate_key"));
    }

    #[test]
    fn follow_up_checks_carry_the_facts_of_their_call() {
        use biscuit_auth::builder as d;
        let f = fixture("s");
        let owner = owner_on(Transport::UnixSocket);
        let token = minted(
            &f,
            &rights(|r| {
                r.call = strings(&["pane.*", "bus.*"]);
                r.control = strings(&["*"]);
            }),
        );
        let only = |method: &str| {
            appended(
                &token.bytes,
                biscuit_auth::BlockBuilder::new()
                    .check(check_of(d::pred("method", &[d::string(method)]), vec![]))
                    .unwrap(),
            )
        };
        let for_call = |method| ForCall { method, target: "" };
        let access = |method| AuthzRequest::PaneAccess {
            pane_id: "1",
            creator: "owner",
            call: for_call(method),
        };
        let create = |method| AuthzRequest::PaneCreate {
            profile: "",
            overrides: false,
            call: for_call(method),
        };
        let list = |method| AuthzRequest::Control {
            action: "list",
            instance: Some("t-1"),
            call: for_call(method),
        };
        let ok = |bearer: &[u8], request: AuthzRequest<'_>| {
            f.acl.authorize(&owner, Some(bearer), &request).is_ok()
        };
        let pane_list = only("pane.list");
        // Each twice: a cached decision for one call is never another's.
        for _ in 0..2 {
            assert!(ok(&pane_list, call("pane.list", "")));
            assert!(ok(&pane_list, access("pane.list")));
            assert!(!ok(&pane_list, access("pane.read")));
            assert!(!ok(&pane_list, create("pane.create")));
            assert!(!ok(&pane_list, call("pane.create", "")));
        }
        let pane_create = only("pane.create");
        for _ in 0..2 {
            assert!(ok(&pane_create, call("pane.create", "")));
            assert!(ok(&pane_create, create("pane.create")));
            assert!(!ok(&pane_create, access("pane.close")));
            assert!(!ok(&pane_create, call("pane.close", "")));
        }
        let instances = only("bus.instances");
        for _ in 0..2 {
            assert!(ok(&instances, call("bus.instances", "")));
            assert!(ok(&instances, list("bus.instances")));
            assert!(!ok(&instances, list("bus.methods")));
        }
        // The call's target too.
        let target = appended(
            &token.bytes,
            biscuit_auth::BlockBuilder::new()
                .check(check_of(d::pred("target", &[d::string("x")]), vec![]))
                .unwrap(),
        );
        let at = |target| AuthzRequest::PaneAccess {
            pane_id: "1",
            creator: "owner",
            call: ForCall {
                method: "pane.read",
                target,
            },
        };
        assert!(ok(&target, at("x")));
        assert!(!ok(&target, at("y")));
        assert!(!ok(&target, at("")));
        // One fact of each per request: a follow-up check adds no second `method`.
        for request in [
            access("pane.list"),
            create("pane.create"),
            list("bus.instances"),
        ] {
            let facts = request_facts(&request).unwrap().facts;
            for name in REQUEST_FACTS {
                assert!(
                    facts.iter().filter(|f| f.predicate.name == *name).count() <= 1,
                    "{name} in {request:?}"
                );
            }
        }
    }

    #[test]
    fn an_appended_block_that_is_not_its_own_encoding_is_refused() {
        use biscuit_auth::format::schema;
        use biscuit_prost::Message as _;
        let symbols = biscuit_auth::datalog::SymbolTable::from(vec![
            "method".to_owned(),
            "pane.list".to_owned(),
        ])
        .unwrap();
        // `check if method("pane.list")`, its head the default symbol 0.
        let predicate = |name, terms| schema::Predicate { name, terms };
        let block = schema::Block {
            version: Some(3),
            checks: vec![schema::Check {
                queries: vec![schema::Rule {
                    head: predicate(0, vec![]),
                    body: vec![predicate(
                        1024,
                        vec![schema::Term {
                            content: Some(schema::term::Content::String(1025)),
                        }],
                    )],
                    expressions: vec![],
                    scope: vec![],
                }],
                kind: None,
            }],
            ..schema::Block::default()
        };
        let data = block.encode_to_vec();
        assert_eq!(check_appended_block(&data, &symbols), Ok(()));
        // The head's required `name` (field 1, `08 00`) left out, an unknown field 15
        // of the same size (`78 00`) in its place: it decodes to the same block, and
        // its re-encoding has the same length but other bytes.
        let head = [0x0a, 0x02, 0x08, 0x00];
        let at = data
            .windows(head.len())
            .position(|w| w == head)
            .expect("the head's bytes");
        assert_eq!(
            data.windows(head.len()).filter(|w| *w == head).count(),
            1,
            "one head"
        );
        let mut crafted = data.clone();
        crafted[at + 2] = 0x78;
        let decoded = schema::Block::decode(crafted.as_slice()).unwrap();
        assert_eq!(decoded, block);
        assert_eq!(decoded.encoded_len(), crafted.len());
        assert_eq!(
            check_appended_block(&crafted, &symbols),
            Err(UNREADABLE.to_owned())
        );
    }

    #[test]
    fn an_appended_block_is_read_from_its_data_not_its_printed_text() {
        use biscuit_auth::builder::{self as d, Scope};
        let f = fixture("s");
        let owner = Principal {
            identity: Identity::Owner,
            ..Principal::tilrd()
        };
        let many: Vec<String> = (0..40).map(|i| format!("m.x{i}")).collect();
        let token = minted(&f, &rights(|r| r.call = many));
        let block = |predicates: Vec<d::Predicate>| {
            biscuit_auth::BlockBuilder::new()
                .check(d::Check {
                    queries: vec![d::Rule::new(
                        d::pred("query", &[] as &[d::Term]),
                        predicates,
                        vec![],
                        vec![],
                    )],
                    kind: d::CheckKind::One,
                })
                .unwrap()
        };
        let joins = || ["a", "b", "c", "d", "e"].map(|x| d::pred("call", &[d::variable(x)]));
        let refused = [
            // 30 hidden facts and a 5-way join (15 s in a release build).
            hidden_join_block(30),
            // A `"` in a string closes it: the 5-way join over the rights sits in a
            // comment the string opens.
            block(
                [d::pred("kind", &[d::string(r#"x")/*"#)])]
                    .into_iter()
                    .chain(joins())
                    .chain([d::pred("*/,kind", &[d::string("y")])])
                    .collect(),
            ),
            // A `//` comment hides the fact on its line.
            biscuit_auth::BlockBuilder::new()
                .fact(d::fact(r#"check if kind("bus");//"#, &[d::string("x")]))
                .unwrap(),
            // A newline in a predicate name, inside a comment the name opens.
            biscuit_auth::BlockBuilder::new()
                .fact(d::fact("check if kind(\"bus\");/*\nf", &[d::string("x")]))
                .unwrap()
                .fact(d::fact("f", &[d::string("y")]))
                .unwrap()
                .check(check_of(
                    d::pred("\n*/check if kind", &[d::string("bus")]),
                    vec![],
                ))
                .unwrap(),
            // A rule alone.
            biscuit_auth::BlockBuilder::new()
                .rule(d::Rule::new(
                    d::pred("g", &[d::variable("m")]),
                    vec![d::pred("method", &[d::variable("m")])],
                    vec![],
                    vec![],
                ))
                .unwrap(),
            // Checks that trust other blocks.
            biscuit_auth::BlockBuilder::new()
                .check(check_of(
                    d::pred("kind", &[d::string("call")]),
                    vec![Scope::Previous],
                ))
                .unwrap(),
            biscuit_auth::BlockBuilder::new()
                .check(check_of(d::pred("kind", &[d::string("call")]), vec![]))
                .unwrap()
                .scope(Scope::Previous),
            // Names that print like a request's fact and are not one.
            block(vec![d::pred("kind ", &[d::string("call")])]),
            block(vec![d::pred("k\u{0456}nd", &[d::string("call")])]),
        ];
        for (n, block) in refused.into_iter().enumerate() {
            let bytes = appended(&token.bytes, block);
            let acl = Arc::clone(&f.acl);
            let owner = owner.clone();
            let refused = within(move || {
                acl.authorize(&owner, Some(&bytes), &call("m.x1", ""))
                    .map_err(|denied| denied.reason)
            })
            .unwrap_or_else(|| panic!("block {n}: still deciding after 5 s"));
            let reason = refused.expect_err("refused");
            assert!(
                reason.starts_with("an appended block"),
                "block {n}: {reason}"
            );
        }
        // The same narrowing built with the builder API verifies and narrows.
        let narrowed = appended(
            &token.bytes,
            biscuit_auth::BlockBuilder::new()
                .check(check_of(
                    d::pred("target", &[d::string("instance:t-2")]),
                    vec![],
                ))
                .unwrap(),
        );
        let decide = |target| {
            f.acl
                .authorize(&owner, Some(&narrowed), &call("m.x1", target))
        };
        assert!(decide("instance:t-2").is_ok());
        assert!(decide("instance:t-3").is_err());
        // About the largest accepted blocks: set operations over literals that fill
        // the token (7.6 ms in a debug build), and a string of 20 copies of one 13 KB
        // symbol (a symbol is stored once however often a block uses it) searched (5 ms
        // with 30 copies). Decided, within the bound.
        let set = (0..850)
            .map(|i| format!("\"{i:04}\""))
            .collect::<Vec<_>>()
            .join(",");
        let big = format!("\"{}\"", "a".repeat(13_000));
        let credential = tilr_bus::Credential::from_bytes(token.bytes.clone());
        for block in [
            format!("check if {{{set}}}.intersection({{{set}}}).length() >= 0;"),
            format!(
                "check if ({}).contains({big} + \"b\") || true;",
                vec![big.as_str(); 20].join(" + ")
            ),
        ] {
            let bytes = credential.attenuate(&block).unwrap().as_bytes().clone();
            let acl = Arc::clone(&f.acl);
            let owner = owner.clone();
            let decided = within(move || {
                acl.authorize(&owner, Some(&bytes), &call("m.x1", ""))
                    .map_err(|denied| denied.reason)
            })
            .expect("decided within 5 s");
            assert!(decided.is_ok(), "{decided:?}");
        }
    }

    #[test]
    fn a_token_is_bound_to_its_server_id() {
        let f = fixture("server-a");
        let token = minted(&f, &rights(|r| r.call = strings(&["*"])));
        // The same key file under another server id (a copied state directory).
        let copy = Acl::new(
            &AuthzConfig {
                server_id: "server-b".into(),
                state_dir: Some(f.dir.clone()),
                ..AuthzConfig::default()
            },
            now,
        );
        let owner = Principal {
            identity: Identity::Owner,
            ..Principal::tilrd()
        };
        assert!(
            copy.authorize(&owner, Some(&token.bytes), &call("a.b", ""))
                .is_err()
        );
        // Also where no Datalog runs: Hello, a credential update, `bus.renew`.
        let hello = Hello {
            protocol_version: 1,
            credential: token.bytes.clone(),
            ..Hello::default()
        };
        let conn = HelloContext {
            transport: Transport::UnixSocket,
            peer: Peer::Unix { pid: None },
            origin: "",
            connection_id: 5,
            seat: None,
        };
        let refused = copy.hello(&hello, &conn).unwrap_err();
        assert!(
            refused.message.contains("another server id"),
            "{}",
            refused.message
        );
        let on_copy = Principal {
            identity: Identity::Token("t-1".into()),
            ..Principal::tilrd()
        };
        assert!(copy.credential_update(&on_copy, &token.bytes).is_err());
        let payload = Bytes::from(
            wire::RenewRequest {
                credential: token.bytes.clone(),
            }
            .encode_to_vec(),
        );
        assert_eq!(
            copy.method_renew(&ctx(&owner, "bus.renew", None), &payload)
                .unwrap_err()
                .code,
            codes::DENIED
        );
        assert!(f.acl.hello(&hello, &conn).is_ok());
        assert!(
            f.acl
                .authorize(&owner, Some(&token.bytes), &call("a.b", ""))
                .is_ok()
        );
    }

    #[test]
    fn expiry_revocation_and_rotation() {
        let _clock = lock(&CLOCK_LOCK);
        let start = now();
        let f = fixture("s");
        let token = minted(&f, &rights(|r| r.call = strings(&["*"])));
        let bytes = token.bytes.clone();
        let p = holder(token);
        assert!(f.acl.authorize(&p, None, &call("a.b", "")).is_ok());
        NOW.store(start + MAX_TTL.as_millis() as u64 + 1, Ordering::SeqCst);
        let refused = f.acl.authorize(&p, None, &call("a.b", "")).unwrap_err();
        assert!(refused.reason.starts_with("expired"), "{refused}");
        NOW.store(start, Ordering::SeqCst);
        // Revoked, also through the decision cache and in a new authorizer.
        assert!(f.acl.authorize(&p, None, &call("a.b", "")).is_ok());
        let rid = token_of(&p).unwrap().revocation_id().to_owned();
        f.acl.revoke_ids(&[(rid, start + 10_000_000)]).unwrap();
        let refused = f.acl.authorize(&p, None, &call("a.b", "")).unwrap_err();
        assert!(refused.reason.starts_with("revoked"), "{refused}");
        let restarted = Acl::new(
            &AuthzConfig {
                state_dir: Some(f.dir.clone()),
                server_id: "s".into(),
                ..AuthzConfig::default()
            },
            now,
        );
        let owner = Principal {
            identity: Identity::Owner,
            ..Principal::tilrd()
        };
        let refused = restarted
            .authorize(&owner, Some(&bytes), &call("a.b", ""))
            .unwrap_err();
        assert!(refused.reason.starts_with("revoked"), "{refused}");
        // Rotation: a fresh token works, then not.
        let fresh = minted(&f, &rights(|r| r.call = strings(&["*"])));
        assert!(
            f.acl
                .authorize(&owner, Some(&fresh.bytes), &call("a.b", ""))
                .is_ok()
        );
        f.acl.rotate().unwrap();
        assert!(
            f.acl
                .authorize(&owner, Some(&fresh.bytes), &call("a.b", ""))
                .is_err()
        );
        let fresh = holder(fresh);
        assert!(f.acl.authorize(&fresh, None, &call("a.b", "")).is_err());
        let after = minted(&f, &rights(|r| r.call = strings(&["*"])));
        assert!(
            f.acl
                .authorize(&owner, Some(&after.bytes), &call("a.b", ""))
                .is_ok()
        );
    }

    #[test]
    fn an_instance_gets_a_token_of_its_rights_at_hello() {
        let f = fixture("s");
        let seat = Seat {
            identity: Identity::Instance("t-1".into()),
            rights: Some(Arc::new(Rights {
                call: strings(&["test.*"]),
                ..Rights::default()
            })),
            transport: Transport::InstanceFd,
            peer: Peer::Child { pid: None },
            check: None,
        };
        let hello = Hello {
            protocol_version: 1,
            ..Hello::default()
        };
        let principal = f
            .acl
            .hello(
                &hello,
                &HelloContext {
                    transport: Transport::InstanceFd,
                    peer: Peer::Child { pid: None },
                    origin: "",
                    connection_id: 11,
                    seat: Some(&seat),
                },
            )
            .unwrap();
        let credential = f.acl.hello_ack_credential(&principal).unwrap();
        assert!(!credential.is_empty());
        assert!(
            f.acl
                .authorize(&principal, None, &call("test.echo", ""))
                .is_ok()
        );
        assert!(
            f.acl
                .authorize(&principal, None, &call("other.echo", ""))
                .is_err()
        );
        // The instance's token, presented per call by someone else, works while
        // the instance's connection is open (`bus_acl` checks that); with no broker
        // to ask whether it is, it fails closed.
        let owner = Principal {
            identity: Identity::Owner,
            connection_id: 12,
            ..Principal::tilrd()
        };
        let refused = f
            .acl
            .authorize(&owner, Some(&credential), &call("test.echo", ""))
            .unwrap_err();
        assert!(refused.reason.contains("cannot be checked"), "{refused}");
        // Its end revokes it.
        let rid = token_of(&principal).unwrap().revocation_id().to_owned();
        f.acl.end_instance_token(&rid);
        let refused = f
            .acl
            .authorize(&owner, Some(&credential), &call("test.echo", ""))
            .unwrap_err();
        assert!(refused.reason.starts_with("revoked"), "{refused}");
        // Not usable in Hello.
        let refused = f
            .acl
            .hello(
                &Hello {
                    protocol_version: 1,
                    credential: credential.clone(),
                    ..Hello::default()
                },
                &HelloContext {
                    transport: Transport::UnixSocket,
                    peer: Peer::Unix { pid: None },
                    origin: "",
                    connection_id: 13,
                    seat: None,
                },
            )
            .unwrap_err();
        assert_eq!(refused.code, codes::DENIED);
    }

    #[test]
    fn hello_with_a_credential_is_its_participant() {
        let f = fixture("s");
        let conn = |transport, origin| HelloContext {
            transport,
            peer: Peer::Unix { pid: None },
            origin,
            connection_id: 21,
            seat: None,
        };
        let hello = |credential: &Bytes, access_token: &str| Hello {
            protocol_version: 1,
            credential: credential.clone(),
            access_token: access_token.into(),
            ..Hello::default()
        };
        let token = minted(&f, &rights(|r| r.call = strings(&["test.*"])));
        let principal = f
            .acl
            .hello(&hello(&token.bytes, ""), &conn(Transport::Tcp, ""))
            .unwrap();
        assert_eq!(principal.identity, Identity::Token("t-1".into()));
        // Neither the access token nor a credential on TCP; a wrong one.
        let none = Bytes::new();
        assert!(
            f.acl
                .hello(&hello(&none, ""), &conn(Transport::Tcp, ""))
                .is_err()
        );
        assert!(
            f.acl
                .hello(&hello(&none, "nope"), &conn(Transport::Tcp, ""))
                .is_err()
        );
        let owner = f
            .acl
            .hello(&hello(&none, "secret"), &conn(Transport::Tcp, ""))
            .unwrap();
        assert_eq!(owner.identity, Identity::Owner);
        let garbage = Bytes::from_static(b"garbage");
        assert!(
            f.acl
                .hello(&hello(&garbage, ""), &conn(Transport::UnixSocket, ""))
                .is_err()
        );
        // An origin-bound token only from that origin.
        let bound = f
            .acl
            .mint(
                &Identity::Token("web:p".into()),
                &rights(|r| r.call = strings(&["test.*"])),
                "tilr://app",
                MAX_TTL,
                &Lineage::default(),
                "",
            )
            .unwrap();
        assert!(
            f.acl
                .hello(
                    &hello(&bound.bytes, ""),
                    &conn(Transport::Tcp, "tilr://app")
                )
                .is_ok()
        );
        let refused = f
            .acl
            .hello(
                &hello(&bound.bytes, ""),
                &conn(Transport::Tcp, "http://localhost:7891"),
            )
            .unwrap_err();
        assert!(refused.message.starts_with("origin"), "{}", refused.message);
        // A credential update must name the same participant.
        let other = f
            .acl
            .mint(
                &Identity::Token("t-2".into()),
                &RightsText::default(),
                "",
                MAX_TTL,
                &Lineage::default(),
                "",
            )
            .unwrap();
        assert!(f.acl.credential_update(&principal, &other.bytes).is_err());
        let renewed = minted(&f, &rights(|r| r.call = strings(&["test.*"])));
        let updated = f.acl.credential_update(&principal, &renewed.bytes).unwrap();
        assert_eq!(
            token_of(&updated).unwrap().revocation_id(),
            renewed.revocation_id()
        );
    }

    #[test]
    fn minting_checks_rights() {
        let bad = [
            rights(|r| r.call = strings(&["Bad"])),
            rights(|r| r.call = strings(&["x.*.y"])),
            rights(|r| r.control = strings(&["explode"])),
            rights(|r| r.pane = strings(&["profile:"])),
            rights(|r| r.subscribe = strings(&["a..b"])),
        ];
        for rights in &bad {
            assert!(check_rights(rights).is_err(), "{rights:?}");
        }
        let good = rights(|r| {
            r.call = strings(&["*", "test.*", "math.add"]);
            r.subscribe = strings(&["pane.*", "pane.>"]);
            r.control = strings(&["list", "reload:t-*", "stop:t-1", "server.reload", "*"]);
            r.pane = strings(&["profile:agent", "profile:*", "access:own"]);
        });
        assert!(check_rights(&good).is_ok());
        let held = rights(|r| r.call = strings(&["test.*"]));
        assert!(rights_within(
            &rights(|r| r.call = strings(&["test.*"])),
            &held
        ));
        assert!(!rights_within(&rights(|r| r.call = strings(&["*"])), &held));
        assert!(!rights_within(
            &rights(|r| r.emit = strings(&["x.y"])),
            &held
        ));
    }

    #[test]
    fn the_revocation_file_reads_back_and_a_broken_one_refuses_all() {
        let f = fixture("s");
        let path = f.dir.join(REVOKED_FILE);
        write_revoked(
            &path,
            &[
                ("ab".into(), now() + 5, false),
                ("cd".into(), now() - 5, false),
                ("ef".into(), now() + 5, true),
            ],
        )
        .unwrap();
        let read = load_revoked(&path, now()).unwrap();
        assert_eq!(read.renewed.keys().collect::<Vec<_>>(), ["ef"]);
        let ids = read.ids;
        assert_eq!(ids.len(), 1);
        assert!(ids.contains_key("ab"));
        std::fs::write(&path, "not a line\n").unwrap();
        let broken = Acl::new(
            &AuthzConfig {
                state_dir: Some(f.dir.clone()),
                ..AuthzConfig::default()
            },
            now,
        );
        let token = broken
            .mint(
                &Identity::Token("x".into()),
                &RightsText::default(),
                "",
                MAX_TTL,
                &Lineage::default(),
                "",
            )
            .unwrap();
        let owner = Principal {
            identity: Identity::Owner,
            ..Principal::tilrd()
        };
        let refused = broken
            .authorize(&owner, Some(&token.bytes), &call("a.b", ""))
            .unwrap_err();
        assert!(refused.reason.contains("revocations"), "{refused}");
    }

    fn owner_on(transport: Transport) -> Principal {
        Principal {
            identity: Identity::Owner,
            transport,
            connection_id: 3,
            ..Principal::tilrd()
        }
    }

    /// A call's context as the broker makes it.
    fn ctx(principal: &Principal, method: &str, credential: Option<&Bytes>) -> CallCtx {
        CallCtx {
            id: 1,
            method: method.into(),
            target: String::new(),
            from: principal.identity.clone(),
            principal: Arc::new(principal.clone()),
            call_credential: credential.cloned(),
            deadline: None,
            cancel: CancelToken::new(),
            content_type: String::new(),
            trace_id: String::new(),
        }
    }

    fn token_request(name: &str, rights: &RightsText, origin: &str, ttl_ms: u64) -> Bytes {
        Bytes::from(
            wire::TokenRequest {
                name: name.into(),
                call: rights.call.clone(),
                provide: rights.provide.clone(),
                emit: rights.emit.clone(),
                subscribe: rights.subscribe.clone(),
                control: rights.control.clone(),
                pane: rights.pane.clone(),
                origin: origin.into(),
                ttl_ms,
            }
            .encode_to_vec(),
        )
    }

    /// `bus.token` by `ctx`: the child, verified.
    fn mint_by(f: &Fixture, ctx: &CallCtx, payload: Bytes) -> Result<Token, BusError> {
        let reply = f.acl.method_token(ctx, &payload)?;
        let reply = wire::TokenReply::decode(reply.payload).unwrap();
        let parsed = f.acl.verify(&reply.credential).unwrap();
        Ok(Token {
            bytes: reply.credential,
            parsed,
        })
    }

    fn renew(f: &Fixture, holder: &Principal) -> Result<Token, BusError> {
        let payload = Bytes::from(wire::RenewRequest::default().encode_to_vec());
        let reply = f
            .acl
            .method_renew(&ctx(holder, "bus.renew", None), &payload)?;
        let reply = wire::RenewReply::decode(reply.payload).unwrap();
        let parsed = f.acl.verify(&reply.credential).unwrap();
        Ok(Token {
            bytes: reply.credential,
            parsed,
        })
    }

    #[test]
    fn a_bearer_decides_the_control_checks_of_its_call() {
        let f = fixture("s");
        // A wide connection, a narrow per-call credential: the method's per-instance
        // checks with the call's credential follow the credential alone.
        let wide = holder(minted(&f, &rights(|r| r.control = strings(&["*"]))));
        let narrow = minted(&f, &rights(|r| r.control = strings(&["reload:t-1"])));
        let control = |instance| AuthzRequest::Control {
            action: "reload",
            instance: Some(instance),
            call: ForCall {
                method: "bus.reload",
                target: "",
            },
        };
        assert!(
            f.acl
                .authorize(&wide, Some(&narrow.bytes), &control("t-1"))
                .is_ok()
        );
        let refused = f
            .acl
            .authorize(&wide, Some(&narrow.bytes), &control("t-2"))
            .unwrap_err();
        assert_eq!(
            refused.reason,
            "token:t-1 has no right to control reload:t-2"
        );
        // The gate of the call passes for the same credential.
        assert!(
            f.acl
                .authorize(&wide, Some(&narrow.bytes), &call("bus.reload", ""))
                .is_ok()
        );
        // Without it the connection's own token decides.
        assert!(f.acl.authorize(&wide, None, &control("t-2")).is_ok());
        // Not for anything but a call and its control and pane checks.
        assert!(
            f.acl
                .authorize(
                    &wide,
                    Some(&narrow.bytes),
                    &AuthzRequest::Subscribe { pattern: "x.y" }
                )
                .is_err()
        );
        let pane = AuthzRequest::PaneAccess {
            pane_id: "pane-1",
            creator: "owner",
            call: ForCall {
                method: "pane.read",
                target: "",
            },
        };
        assert!(f.acl.authorize(&wide, Some(&narrow.bytes), &pane).is_ok());
    }

    #[test]
    fn server_command_is_the_owners_on_the_unix_socket_only() {
        let f = fixture("s");
        let command = call("server.command", "");
        assert!(
            f.acl
                .authorize(&owner_on(Transport::UnixSocket), None, &command)
                .is_ok()
        );
        assert!(f.acl.authorize(&Principal::tilrd(), None, &command).is_ok());
        // The owner on TCP (the access token, or `--no-auth`): no.
        let refused = f
            .acl
            .authorize(&owner_on(Transport::Tcp), None, &command)
            .unwrap_err();
        assert_eq!(
            refused.reason,
            "server.command is the owner's on the Unix socket only"
        );
        // No token grants it, as the connection's or per call.
        let every = minted(
            &f,
            &rights(|r| {
                r.call = strings(&["*", "server.*", "server.command"]);
                r.control = strings(&["*"]);
            }),
        );
        assert!(
            f.acl
                .authorize(
                    &owner_on(Transport::UnixSocket),
                    Some(&every.bytes),
                    &command
                )
                .is_err()
        );
        let mut on_unix = holder(every);
        on_unix.transport = Transport::UnixSocket;
        assert!(f.acl.authorize(&on_unix, None, &command).is_err());
    }

    #[test]
    fn without_an_access_token_tcp_is_the_owner() {
        // `--no-auth`: the server asks for no access token.
        let acl = Acl::new(
            &AuthzConfig {
                server_id: "s".into(),
                ..AuthzConfig::default()
            },
            now,
        );
        let conn = HelloContext {
            transport: Transport::Tcp,
            peer: Peer::Unix { pid: None },
            origin: "",
            connection_id: 31,
            seat: None,
        };
        let hello = |credential: Bytes| Hello {
            protocol_version: 1,
            credential,
            ..Hello::default()
        };
        let owner = acl.hello(&hello(Bytes::new()), &conn).unwrap();
        assert_eq!(owner.identity, Identity::Owner);
        // Still no server.command on TCP; a credential is still its participant.
        assert!(
            acl.authorize(&owner, None, &call("server.command", ""))
                .is_err()
        );
        let token = acl
            .mint(
                &Identity::Token("t-1".into()),
                &RightsText::default(),
                "",
                MAX_TTL,
                &Lineage::default(),
                "",
            )
            .unwrap();
        let participant = acl.hello(&hello(token.bytes.clone()), &conn).unwrap();
        assert_eq!(participant.identity, Identity::Token("t-1".into()));
    }

    #[test]
    fn server_update_needs_its_control_action() {
        let f = fixture("s");
        let update = call("server.update", "");
        let ok = |rights: RightsText| {
            f.acl
                .authorize(&holder(minted(&f, &rights)), None, &update)
                .is_ok()
        };
        assert!(!ok(rights(|r| r.call = strings(&["*"]))));
        assert!(!ok(rights(|r| r.call = strings(&["server.*"]))));
        assert!(!ok(rights(|r| r.control = strings(&["server.reload"]))));
        assert!(!ok(rights(|r| r.control = strings(&["reload"]))));
        assert!(ok(rights(|r| r.control = strings(&["server.update"]))));
        assert!(ok(rights(|r| r.control = strings(&["*"]))));
        assert!(check_rights(&rights(|r| r.control = strings(&["server.update"]))).is_ok());
        assert!(check_rights(&rights(|r| r.control = strings(&["server.command"]))).is_err());
    }

    #[test]
    fn a_minter_mints_under_its_name_origin_and_life() {
        let f = fixture("s");
        let minter = f
            .acl
            .mint(
                &Identity::Token("app".into()),
                &rights(|r| {
                    r.call = strings(&["test.*"]);
                    r.control = strings(&["token"]);
                }),
                "tilr://app",
                Duration::from_secs(3600),
                &Lineage::default(),
                "",
            )
            .unwrap();
        let minter_expires = minter.expires_ms();
        let minter_rid = minter.revocation_id().to_owned();
        let mut p = holder(minter);
        p.origin = "tilr://app".into();
        let wanted = rights(|r| r.call = strings(&["test.*"]));
        let by = ctx(&p, "bus.token", None);
        // Its own name only.
        let refused = mint_by(&f, &by, token_request("other", &wanted, "", 0)).unwrap_err();
        assert_eq!(refused.code, codes::DENIED);
        assert!(refused.message.contains("token.app.<name>"), "{refused}");
        // Bound to the minter's origin, and never past its token.
        let child = mint_by(&f, &by, token_request("token.app.child", &wanted, "", 0)).unwrap();
        assert_eq!(child.identity(), &Identity::Token("token.app.child".into()));
        assert_eq!(child.parsed.meta.origin, "tilr://app");
        assert_eq!(child.expires_ms(), minter_expires);
        assert_eq!(child.parsed.meta.lineage.ceiling_ms, minter_expires);
        assert_eq!(child.parsed.meta.lineage.parents, vec![minter_rid]);
        let refused = mint_by(
            &f,
            &by,
            token_request("token.app.c2", &wanted, "http://localhost:7891", 0),
        )
        .unwrap_err();
        assert_eq!(refused.code, codes::DENIED);
        // A grandchild inherits the chain.
        let child_minter = mint_by(
            &f,
            &by,
            token_request(
                "token.app.m",
                &rights(|r| {
                    r.call = strings(&["test.*"]);
                    r.control = strings(&["token"]);
                }),
                "tilr://app",
                0,
            ),
        )
        .unwrap();
        let child_rid = child_minter.revocation_id().to_owned();
        let mut cp = holder(child_minter);
        cp.origin = "tilr://app".into();
        let grandchild = mint_by(
            &f,
            &ctx(&cp, "bus.token", None),
            token_request("token.token.app.m.g", &wanted, "", 0),
        )
        .unwrap();
        assert_eq!(grandchild.parsed.meta.lineage.parents.len(), 2);
        assert_eq!(grandchild.parsed.meta.lineage.parents[1], child_rid);

        // An attenuated minter cannot mint, as the connection's token or per call.
        let narrowed = tilr_bus::Credential::from_bytes(token_of(&p).unwrap().bytes.clone())
            .attenuate(r#"check if method("test.echo") or kind("control");"#)
            .unwrap();
        let narrowed = narrowed.as_bytes().clone();
        let owner = owner_on(Transport::UnixSocket);
        for by in [
            ctx(&owner, "bus.token", Some(&narrowed)),
            ctx(
                &Principal {
                    credential: Some(Arc::new(Token {
                        bytes: narrowed.clone(),
                        parsed: f.acl.verify(&narrowed).unwrap(),
                    })),
                    ..p.clone()
                },
                "bus.token",
                None,
            ),
        ] {
            let refused =
                mint_by(&f, &by, token_request("token.app.n", &wanted, "", 0)).unwrap_err();
            assert_eq!(refused.code, codes::DENIED);
            assert!(refused.message.contains("appended blocks"), "{refused}");
        }
        // The owner names anything and binds to any origin.
        assert!(
            mint_by(
                &f,
                &ctx(&owner, "bus.token", None),
                token_request("anyone", &wanted, "http://localhost:7891", 0)
            )
            .is_ok()
        );
    }

    #[test]
    fn a_child_dies_with_its_minter_and_alone() {
        let f = fixture("s");
        let minter = minted(
            &f,
            &rights(|r| {
                r.call = strings(&["test.*"]);
                r.control = strings(&["token"]);
            }),
        );
        let minter_bytes = minter.bytes.clone();
        let minter_rid = minter.revocation_id().to_owned();
        let p = holder(minter);
        let wanted = rights(|r| r.call = strings(&["test.*"]));
        let by = ctx(&p, "bus.token", None);
        let first = mint_by(&f, &by, token_request("token.t-1.a", &wanted, "", 0)).unwrap();
        let second = mint_by(&f, &by, token_request("token.t-1.b", &wanted, "", 0)).unwrap();
        let owner = owner_on(Transport::UnixSocket);
        let echo = call("test.echo", "");
        let ok = |bytes: &Bytes| f.acl.authorize(&owner, Some(bytes), &echo);
        // The child alone: the minter and its other child keep working.
        f.acl
            .revoke_ids(&[(first.revocation_id().to_owned(), u64::MAX)])
            .unwrap();
        assert!(ok(&first.bytes).unwrap_err().reason.starts_with("revoked"));
        assert!(ok(&minter_bytes).is_ok());
        assert!(ok(&second.bytes).is_ok());
        // The minter: every child with it, also after a restart.
        f.acl.revoke_ids(&[(minter_rid, u64::MAX)]).unwrap();
        let refused = ok(&second.bytes).unwrap_err();
        assert_eq!(
            refused.reason,
            "revoked: the token of its minter was revoked"
        );
        let restarted = Acl::new(
            &AuthzConfig {
                state_dir: Some(f.dir.clone()),
                server_id: "s".into(),
                ..AuthzConfig::default()
            },
            now,
        );
        assert!(
            restarted
                .authorize(&owner, Some(&second.bytes), &echo)
                .unwrap_err()
                .reason
                .starts_with("revoked")
        );
    }

    #[test]
    fn renewal_is_for_the_second_half_and_stops_at_the_minters_expiry() {
        let _clock = lock(&CLOCK_LOCK);
        let start = now();
        let hour = 3_600_000;
        let f = fixture("s");
        let minter = minted(
            &f,
            &rights(|r| {
                r.call = strings(&["test.*"]);
                r.control = strings(&["token"]);
            }),
        );
        let ceiling = minter.expires_ms();
        let minter = holder(minter);
        // An hour into a day: too early.
        NOW.store(start + hour, Ordering::SeqCst);
        let refused = renew(&f, &minter).unwrap_err();
        assert_eq!(refused.code, codes::BAD_REQUEST);
        assert!(refused.message.starts_with("too early"), "{refused}");
        // A child of 2 h minted 20 h in, renewed 21.5 h in: up to the minter's expiry.
        NOW.store(start + 20 * hour, Ordering::SeqCst);
        let child = mint_by(
            &f,
            &ctx(&minter, "bus.token", None),
            token_request(
                "token.t-1.c",
                &rights(|r| r.call = strings(&["test.*"])),
                "",
                2 * hour,
            ),
        )
        .unwrap();
        assert_eq!(child.expires_ms(), start + 22 * hour);
        NOW.store(start + 21 * hour + hour / 2, Ordering::SeqCst);
        let renewed = renew(&f, &holder(child)).unwrap();
        assert_eq!(renewed.expires_ms(), start + 23 * hour + hour / 2);
        NOW.store(start + 23 * hour, Ordering::SeqCst);
        let renewed = renew(&f, &holder(renewed)).unwrap();
        assert_eq!(renewed.expires_ms(), ceiling, "not past the minter's token");
        NOW.store(start + 23 * hour + hour / 2, Ordering::SeqCst);
        let refused = renew(&f, &holder(renewed)).unwrap_err();
        assert!(
            refused
                .message
                .contains("as long as the token of its minter"),
            "{refused}"
        );
        // A token the owner minted: renewed in its second half, for a full life.
        let renewed = renew(&f, &minter).unwrap();
        assert_eq!(
            renewed.expires_ms(),
            start + 23 * hour + hour / 2 + 24 * hour
        );
        NOW.store(start, Ordering::SeqCst);
    }

    /// `bus.revoke` of `what` by the owner.
    fn revoke(f: &Fixture, what: wire::revoke_request::What) {
        let payload = Bytes::from(wire::RevokeRequest { what: Some(what) }.encode_to_vec());
        let owner = owner_on(Transport::UnixSocket);
        f.acl
            .method_revoke(&ctx(&owner, "bus.revoke", None), &payload)
            .unwrap();
    }

    #[test]
    fn a_renewal_dies_with_the_token_it_renewed() {
        use wire::revoke_request::What;
        let _clock = lock(&CLOCK_LOCK);
        let start = now();
        let hour = 3_600_000;
        let f = fixture("s");
        let owner = owner_on(Transport::UnixSocket);
        let echo = call("test.echo", "");
        let ok = |acl: &Acl, bytes: &Bytes| acl.authorize(&owner, Some(bytes), &echo);
        let revoked = |acl: &Acl, bytes: &Bytes| {
            let refused = ok(acl, bytes).unwrap_err();
            assert!(refused.reason.starts_with("revoked"), "{refused}");
        };
        let restarted = || {
            Acl::new(
                &AuthzConfig {
                    state_dir: Some(f.dir.clone()),
                    server_id: "s".into(),
                    ..AuthzConfig::default()
                },
                now,
            )
        };
        let wanted = rights(|r| r.call = strings(&["test.*"]));
        // Two leaked tokens; the thief renews both, the second one twice.
        let leaked = [minted(&f, &wanted), minted(&f, &wanted)];
        let ids: Vec<String> = leaked
            .iter()
            .map(|token| token.revocation_id().to_owned())
            .collect();
        let bytes: Vec<Bytes> = leaked.iter().map(|token| token.bytes.clone()).collect();
        NOW.store(start + 13 * hour, Ordering::SeqCst);
        let [first, second] = leaked;
        let renewed = renew(&f, &holder(first)).unwrap();
        let payload = Bytes::from(wire::RenewRequest::default().encode_to_vec());
        let reply = f
            .acl
            .method_renew(&ctx(&holder(second), "bus.renew", None), &payload)
            .unwrap();
        let reply = wire::RenewReply::decode(reply.payload).unwrap();
        let again = Token {
            parsed: f.acl.verify(&reply.credential).unwrap(),
            bytes: reply.credential,
        };
        // Renewed: the old ones refused, the new ones work, also after a restart.
        for acl in [&*f.acl, &restarted()] {
            revoked(acl, &bytes[0]);
            revoked(acl, &bytes[1]);
            assert!(ok(acl, &renewed.bytes).is_ok());
            assert!(ok(acl, &again.bytes).is_ok());
        }
        NOW.store(start + 26 * hour, Ordering::SeqCst);
        let twice = renew(&f, &holder(again)).unwrap();
        assert!(ok(&f.acl, &twice.bytes).is_ok());
        // The owner revokes what it gave out, the token or its id: the renewals die.
        revoke(&f, What::Credential(bytes[0].clone()));
        revoke(&f, What::RevocationId(ids[1].clone()));
        for acl in [&*f.acl, &restarted()] {
            revoked(acl, &renewed.bytes);
            revoked(acl, &twice.bytes);
        }
        // The renewal's reply named the id `bus.token` answered: the chain's.
        assert_eq!(reply.revocation_id, ids[1]);
        // And no further renewal.
        assert_eq!(renew(&f, &holder(twice)).unwrap_err().code, codes::DENIED);
        NOW.store(start, Ordering::SeqCst);
    }

    #[test]
    fn renewing_a_minter_keeps_its_children() {
        use wire::revoke_request::What;
        let _clock = lock(&CLOCK_LOCK);
        let start = now();
        let hour = 3_600_000;
        let f = fixture("s");
        let owner = owner_on(Transport::UnixSocket);
        let echo = call("test.echo", "");
        let ok = |bytes: &Bytes| f.acl.authorize(&owner, Some(bytes), &echo);
        let minter_rights = rights(|r| {
            r.call = strings(&["test.*"]);
            r.control = strings(&["token"]);
        });
        let wanted = rights(|r| r.call = strings(&["test.*"]));
        // An outside minter: renewed, its child lives; revoked, its child dies.
        let minter = holder(minted(&f, &minter_rights));
        let child = mint_by(
            &f,
            &ctx(&minter, "bus.token", None),
            token_request("token.t-1.c", &wanted, "", 0),
        )
        .unwrap();
        NOW.store(start + 13 * hour, Ordering::SeqCst);
        let renewed = renew(&f, &minter).unwrap();
        assert!(ok(&child.bytes).is_ok());
        let renewed_bytes = renewed.bytes.clone();
        // What the renewed minter mints dies with the chain as well.
        let later = mint_by(
            &f,
            &ctx(&holder(renewed), "bus.token", None),
            token_request("token.t-1.d", &wanted, "", 0),
        )
        .unwrap();
        revoke(&f, What::Credential(renewed_bytes.clone()));
        for bytes in [&child.bytes, &later.bytes] {
            assert_eq!(
                ok(bytes).unwrap_err().reason,
                "revoked: the token of its minter was revoked"
            );
        }
        NOW.store(start, Ordering::SeqCst);

        // An instance: its 12 h renewal (as `renew_instance` does it) keeps what it
        // minted; the end of its connection ends it.
        let identity = Identity::Instance("i".into());
        let first = f
            .acl
            .mint_instance(&identity, &minter_rights, 9, true, "")
            .unwrap();
        let first_rid = first.revocation_id().to_owned();
        let family = first.parsed.family().to_owned();
        let instance = |token: Token| Principal {
            identity: identity.clone(),
            credential: Some(Arc::new(token)),
            connection_id: 9,
            ..Principal::tilrd()
        };
        let first = instance(first);
        let child = mint_by(
            &f,
            &ctx(&first, "bus.token", None),
            token_request("instance.i.c", &wanted, "", 0),
        )
        .unwrap();
        let next = f
            .acl
            .mint_instance(&identity, &minter_rights, 9, true, &family)
            .unwrap();
        let next_rid = next.revocation_id().to_owned();
        f.acl.supersede_instance_token(&first_rid);
        let next = instance(next);
        assert!(f.acl.authorize(&first, None, &echo).is_err());
        assert!(f.acl.authorize(&next, None, &echo).is_ok());
        assert!(ok(&child.bytes).is_ok());
        f.acl.end_instance_token(&next_rid);
        assert!(f.acl.authorize(&next, None, &echo).is_err());
        assert_eq!(
            ok(&child.bytes).unwrap_err().reason,
            "revoked: the token of its minter was revoked"
        );
    }

    #[test]
    fn a_timeout_is_denied_and_never_cached() {
        let mut f = fixture("s");
        assert_eq!(run_limits().max_time, Duration::from_millis(50));
        assert_eq!(run_limits().max_facts, 1000);
        assert_eq!(run_limits().max_iterations, 100);
        let p = holder(minted(&f, &rights(|r| r.call = strings(&["test.*"]))));
        let echo = call("test.echo", "");
        // A machine so loaded that the backstop ends the run: denied this once.
        let acl = Arc::get_mut(&mut f.acl).unwrap();
        acl.policies =
            AuthorizerBuilder::new()
                .code(POLICIES)
                .unwrap()
                .set_limits(AuthorizerLimits {
                    max_time: Duration::ZERO,
                    ..run_limits()
                });
        assert_eq!(
            f.acl.authorize(&p, None, &echo).unwrap_err().reason,
            "authorization limit reached"
        );
        assert!(lock(&f.acl.decisions).is_empty());
        let acl = Arc::get_mut(&mut f.acl).unwrap();
        acl.policies = AuthorizerBuilder::new()
            .code(POLICIES)
            .unwrap()
            .set_limits(run_limits());
        assert!(f.acl.authorize(&p, None, &echo).is_ok());
        assert_eq!(lock(&f.acl.decisions).len(), 1);
    }

    #[test]
    fn concurrent_revocations_all_survive_a_restart() {
        let f = fixture("s");
        let threads = 8;
        let each = 12;
        std::thread::scope(|scope| {
            for t in 0..threads {
                let acl = &f.acl;
                scope.spawn(move || {
                    for i in 0..each {
                        acl.revoke_ids(&[(format!("{t:02x}{i:04x}"), u64::MAX)])
                            .unwrap();
                    }
                });
            }
        });
        let ids = load_revoked(&f.dir.join(REVOKED_FILE), now()).unwrap().ids;
        assert_eq!(ids.len(), threads * each);
        // No staged file left behind.
        let left: Vec<_> = std::fs::read_dir(&f.dir)
            .unwrap()
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.ends_with(".tmp"))
            .collect();
        assert!(left.is_empty(), "{left:?}");
    }

    #[test]
    fn a_broken_revocation_file_is_never_overwritten() {
        let f = fixture("s");
        let token = minted(&f, &rights(|r| r.call = strings(&["*"])));
        let path = f.dir.join(REVOKED_FILE);
        std::fs::write(&path, "not a line\n").unwrap();
        let config = AuthzConfig {
            state_dir: Some(f.dir.clone()),
            server_id: "s".into(),
            ..AuthzConfig::default()
        };
        let broken = Acl::new(&config, now);
        let error = broken
            .revoke_ids(&[(token.revocation_id().to_owned(), u64::MAX)])
            .unwrap_err();
        assert!(error.contains("not written"), "{error}");
        broken.rotate().unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "not a line\n");
        // After a restart: still broken, still refusing (a token of the rotated key).
        let restarted = Acl::new(&config, now);
        let token = restarted
            .mint(
                &Identity::Token("t-1".into()),
                &rights(|r| r.call = strings(&["*"])),
                "",
                MAX_TTL,
                &Lineage::default(),
                "",
            )
            .unwrap();
        let owner = owner_on(Transport::UnixSocket);
        let refused = restarted
            .authorize(&owner, Some(&token.bytes), &call("a.b", ""))
            .unwrap_err();
        assert!(refused.reason.contains("revocations"), "{refused}");
    }

    #[test]
    fn a_target_longer_than_a_name_is_denied_unread() {
        let f = fixture("s");
        let p = holder(minted(&f, &rights(|r| r.call = strings(&["*"]))));
        let owner = owner_on(Transport::UnixSocket);
        let long = "x".repeat(MAX_TARGET_BYTES + 1);
        for who in [&p, &owner] {
            let refused = f.acl.authorize(who, None, &call("a.b", &long)).unwrap_err();
            assert!(
                refused.reason.starts_with("the target is over"),
                "{refused}"
            );
        }
        // Not cached: the cache keeps no target's bytes.
        assert!(lock(&f.acl.decisions).is_empty());
        let longest = "x".repeat(MAX_TARGET_BYTES);
        assert!(f.acl.authorize(&p, None, &call("a.b", &longest)).is_ok());
        assert_eq!(lock(&f.acl.decisions).len(), 1);
        // Parts are length-prefixed: `a.b` with the target `c|d` is not `a.b|c`
        // with `d`.
        let hash = [0; 32];
        assert_ne!(
            decision_key(&hash, &["a.b", "c|d"]),
            decision_key(&hash, &["a.b|c", "d"])
        );
    }

    #[test]
    fn a_revocation_keeps_the_decisions_of_other_tokens() {
        let f = fixture("s");
        let kept = holder(minted(&f, &rights(|r| r.call = strings(&["*"]))));
        let gone = holder(minted(&f, &rights(|r| r.call = strings(&["*"]))));
        for p in [&kept, &gone] {
            f.acl.authorize(p, None, &call("a.b", "")).unwrap();
        }
        assert_eq!(lock(&f.acl.decisions).len(), 2);
        let rid = token_of(&gone).unwrap().revocation_id().to_owned();
        f.acl.revoke_ids(&[(rid.clone(), u64::MAX)]).unwrap();
        let decisions = lock(&f.acl.decisions);
        assert_eq!(decisions.len(), 1);
        assert!(decisions.values().all(|d| !d.ids.contains(&rid)));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_file_methods_run_off_the_envelope_path() {
        let f = fixture("s");
        let token = minted(&f, &rights(|r| r.call = strings(&["*"])));
        let owner = owner_on(Transport::UnixSocket);
        let payload = Bytes::from(
            wire::RevokeRequest {
                what: Some(wire::revoke_request::What::RevocationId(
                    token.revocation_id().to_owned(),
                )),
            }
            .encode_to_vec(),
        );
        let method = blocking_method(&f.acl, "bus.revoke", Acl::method_revoke);
        let NativeHandler::Unary(handler) = method.handler else {
            panic!("a unary method");
        };
        // What the broker calls on the envelope path does nothing yet.
        let reply = handler(ctx(&owner, "bus.revoke", None), payload);
        assert!(
            f.acl
                .authorize(&owner, Some(&token.bytes), &call("a.b", ""))
                .is_ok()
        );
        // The reply comes once it is revoked, in memory and on disk.
        reply.await.unwrap();
        assert!(
            f.acl
                .authorize(&owner, Some(&token.bytes), &call("a.b", ""))
                .unwrap_err()
                .reason
                .starts_with("revoked")
        );
        let ids = load_revoked(&f.dir.join(REVOKED_FILE), now()).unwrap().ids;
        assert!(ids.contains_key(token.revocation_id()));
    }

    #[test]
    fn the_sweeper_ends_tokens_of_an_earlier_key() {
        let f = fixture("s");
        let outside = holder(minted(&f, &rights(|r| r.call = strings(&["*"]))));
        let instance = Principal {
            identity: Identity::Instance("t-1".into()),
            credential: Some(Arc::new(
                f.acl
                    .mint_instance(
                        &Identity::Instance("t-1".into()),
                        &RightsText::default(),
                        9,
                        true,
                        "",
                    )
                    .unwrap(),
            )),
            connection_id: 9,
            ..Principal::tilrd()
        };
        let now = now();
        assert_eq!(f.acl.sweep_action(&outside, now), None);
        assert_eq!(f.acl.sweep_action(&instance, now), None);
        // Said Hello with a token of the old key while the rotation ran.
        f.acl.rotate().unwrap();
        assert_eq!(
            f.acl.sweep_action(&outside, now),
            Some(Sweep::Close("key rotated"))
        );
        assert_eq!(f.acl.sweep_action(&instance, now), Some(Sweep::Renew));
        let fresh = holder(minted(&f, &rights(|r| r.call = strings(&["*"]))));
        assert_eq!(f.acl.sweep_action(&fresh, now), None);
    }

    #[test]
    fn an_instance_acting_on_its_connection_is_seen() {
        let f = fixture("s");
        let token = f
            .acl
            .mint_instance(
                &Identity::Instance("t-1".into()),
                &rights(|r| r.call = strings(&["*"])),
                11,
                false,
                "",
            )
            .unwrap();
        let rid = token.revocation_id().to_owned();
        let p = Principal {
            identity: Identity::Instance("t-1".into()),
            credential: Some(Arc::new(token)),
            connection_id: 11,
            ..Principal::tilrd()
        };
        assert!(!lock(&f.acl.instances)[&rid].seen);
        f.acl.authorize(&p, None, &call("a.b", "")).unwrap();
        assert!(lock(&f.acl.instances)[&rid].seen);
    }

    #[test]
    fn mint_never_makes_a_token_verify_refuses() {
        let f = fixture("s");
        let many: Vec<String> = (0..MAX_RIGHTS_FACTS + 1)
            .map(|i| format!("m.x{i}"))
            .collect();
        let error = f
            .acl
            .mint(
                &Identity::Token("x".into()),
                &rights(|r| r.call = many),
                "",
                MAX_TTL,
                &Lineage::default(),
                "",
            )
            .unwrap_err();
        assert!(error.contains("at most"), "{error}");
        let long: Vec<String> = (0..200)
            .map(|i| format!("m.{}{i}", "x".repeat(100)))
            .collect();
        let error = f
            .acl
            .mint(
                &Identity::Token("x".into()),
                &rights(|r| r.call = long),
                "",
                MAX_TTL,
                &Lineage::default(),
                "",
            )
            .unwrap_err();
        assert!(error.contains("16 KiB"), "{error}");
    }

    /// Decision cost; `cargo test --release -p tilrd acl::tests::authorize_cost
    /// -- --ignored --nocapture` prints it.
    #[test]
    #[ignore = "a measurement, not a check"]
    fn authorize_cost() {
        let f = fixture("s");
        let token = minted(&f, &rights(|r| r.call = strings(&["test.*", "math.add"])));
        let p = holder(token);
        let n = 10_000;
        let start = Instant::now();
        for _ in 0..n {
            f.acl.authorize(&p, None, &call("test.echo", "")).unwrap();
        }
        let cached = start.elapsed() / n;
        let start = Instant::now();
        for i in 0..n {
            lock(&f.acl.decisions).clear();
            let _ = i;
            f.acl.authorize(&p, None, &call("test.echo", "")).unwrap();
        }
        let uncached = start.elapsed() / n;
        let bytes = token_of(&p).unwrap().bytes.clone();
        let start = Instant::now();
        for _ in 0..1000 {
            lock(&f.acl.parsed).clear();
            f.acl.verify(&bytes).unwrap();
        }
        let parse = start.elapsed() / 1000;
        println!("cached decision {cached:?}, uncached {uncached:?}, parse+verify {parse:?}");
    }
}
