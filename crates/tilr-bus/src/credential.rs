//! Narrowing a token without the server (design R5, R6; change `bus-rights`).
//!
//! A token of the bus is a Biscuit: its authority block, signed with the server's
//! root key, holds the participant, its rights and the checks that bind it to the
//! server and its expiry. A holder may append a block of checks with no key of the
//! server; the server then enforces every block on every request. An appended block
//! cannot add a right: the server refuses a token whose appended block holds facts
//! or rules (such as `call_any()`), so an appended block holds checks only, and
//! those checks test the facts of the request.
//!
//! What a check of an appended block can test, per request (the server supplies
//! these facts): `kind(k)` (`call`, `control`, `control_gate`, `provide`, `emit`,
//! `subscribe`, `event`, `pane`, `bus`; `control_gate` is a call of `bus.start`,
//! `bus.stop` or `bus.reload`, whose method then checks each instance with
//! `control`), `method(m)`, `namespace(n)`, `target(t)`, `subject(s)`,
//! `pattern(p)`, `action(a)`, `instance(i)`, `scope(s)`, `caller(c)`, `origin(o)`,
//! `server(id)` and `time(t)`. A check that names a fact the request does not carry
//! fails, so `check if method("test.echo")` also refuses subscriptions; write
//! `check if method("test.echo") or kind("subscribe")` to keep them.

use crate::{BusError, Credential};

impl Credential {
    /// This token with one more block appended: `block` is Biscuit Datalog,
    /// `check if ...;` lines (`check if target("instance:t-2");`); the server refuses
    /// a token whose appended block holds facts or rules. Needs no key of the server;
    /// the server enforces the new block with the earlier ones.
    /// At most 8 blocks and 16 KiB are accepted by the server.
    pub fn attenuate(&self, block: &str) -> Result<Credential, BusError> {
        let token = biscuit_auth::UnverifiedBiscuit::from(self.as_bytes().as_ref())
            .map_err(|err| BusError::bad_request(format!("not a token: {err}")))?;
        let block = biscuit_auth::BlockBuilder::new()
            .code(block)
            .map_err(|err| BusError::bad_request(format!("not a block of checks: {err}")))?;
        let narrowed = token
            .append(block)
            .map_err(|err| BusError::bad_request(format!("cannot append: {err}")))?;
        let bytes = narrowed
            .to_vec()
            .map_err(|err| BusError::bad_request(format!("cannot encode: {err}")))?;
        Ok(Credential::from_bytes(bytes))
    }
}

#[cfg(test)]
mod tests {
    use biscuit_auth::{AuthorizerBuilder, Biscuit, KeyPair};

    use super::*;

    /// A token minted here, with a key made for the test.
    fn minted(root: &KeyPair) -> Credential {
        let token = Biscuit::builder()
            .code(r#"participant("token:t"); call("test.echo"); call("test.whoami");"#)
            .unwrap()
            .build(root)
            .unwrap();
        Credential::from_bytes(token.to_vec().unwrap())
    }

    fn allowed(root: &KeyPair, credential: &Credential, method: &str, target: &str) -> bool {
        let token = Biscuit::from(credential.as_bytes().as_ref(), root.public()).unwrap();
        let mut authorizer = AuthorizerBuilder::new()
            .fact(biscuit_auth::builder::fact(
                "method",
                &[biscuit_auth::builder::string(method)],
            ))
            .unwrap()
            .fact(biscuit_auth::builder::fact(
                "target",
                &[biscuit_auth::builder::string(target)],
            ))
            .unwrap()
            .code("allow if method($m), call($m); deny if true;")
            .unwrap()
            .build(&token)
            .unwrap();
        authorizer.authorize().is_ok()
    }

    #[test]
    fn an_attenuated_credential_keeps_the_signature_and_only_narrows() {
        let root = KeyPair::new();
        let token = minted(&root);
        let narrowed = token
            .attenuate(r#"check if target("instance:t-2");"#)
            .unwrap();
        assert_ne!(narrowed, token);
        // Still a token of the same root key, one block longer.
        let parsed = Biscuit::from(narrowed.as_bytes().as_ref(), root.public()).unwrap();
        assert_eq!(parsed.block_count(), 2);
        assert!(allowed(&root, &narrowed, "test.echo", "instance:t-2"));
        assert!(!allowed(&root, &narrowed, "test.echo", "instance:t-3"));
        // A fact in an appended block adds no right.
        let widened = token.attenuate(r#"call("other.echo");"#).unwrap();
        assert!(!allowed(&root, &widened, "other.echo", ""));
        assert!(allowed(&root, &widened, "test.echo", ""));
        // Another root key does not verify it.
        assert!(Biscuit::from(narrowed.as_bytes().as_ref(), KeyPair::new().public()).is_err());
    }

    #[test]
    fn attenuating_refuses_what_is_not_a_token_or_a_block() {
        let root = KeyPair::new();
        let not_a_token = Credential::from_bytes(&b"not a token"[..]);
        assert_eq!(
            not_a_token.attenuate("check if true;").unwrap_err().code(),
            crate::codes::BAD_REQUEST
        );
        let token = minted(&root);
        assert_eq!(
            token.attenuate("check if (").unwrap_err().code(),
            crate::codes::BAD_REQUEST
        );
    }
}
