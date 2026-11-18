//! `pane.profiles` and the resolution of `pane.create {profile}` (P1-P8).
//!
//! Owner: `pane-profiles`. The files, their checks and the parsing are
//! `crate::profile`'s; this file puts them on the bus:
//!
//! - [`register`] adds `pane.profiles` (P6);
//! - [`prepare_create`] is the seam for `pane.create` (`bus-core-panes`): everything
//!   the request and the profile decide before a pane starts, with no supervisor in
//!   it. `panes.rs` calls it first thing in its `pane.create` handler and starts the
//!   pane from what it returns;
//! - [`remember`], [`of`] and [`forget`] keep the snapshot of each pane created with
//!   a profile on the supervisor (a slot, [`ProfilesState`]), for `PaneInfo.profile`
//!   and `PaneInfo.sandboxed`, until the pane is gone. A reload does not carry it
//!   yet (`bus-reload`).

use std::collections::HashMap;
use std::path::PathBuf;

use prost::Message;
use tilr_bus::pane::{CreateRequest, ProfileInfo, ProfilesReply, ProfilesRequest};
use tilr_bus::{BusError, CONTENT_TYPE_JSON, Reply};

use crate::bus::authz::{Authorizer, AuthzRequest, ForCall};
use crate::bus::core::{Core, Registry};
use crate::bus::identity::CallCtx;
use crate::bus::streams::NativeMethod;
use crate::profile::{self, Error, Snapshot};
use crate::supervisor::{PaneId, Supervisor};

/// The snapshots of the panes created with a profile (`CoreState::slot`).
#[derive(Default)]
pub struct ProfilesState {
    panes: HashMap<PaneId, Snapshot>,
}

/// The pane `pane_id` was created with `snapshot`.
pub(crate) fn remember(s: &mut Supervisor, pane_id: PaneId, snapshot: Snapshot) {
    s.bus
        .slot::<ProfilesState>()
        .panes
        .insert(pane_id, snapshot);
}

/// The snapshot of the profile the pane was created with, if any.
pub fn of(s: &Supervisor, pane_id: PaneId) -> Option<&Snapshot> {
    s.bus.slot_ref::<ProfilesState>()?.panes.get(&pane_id)
}

/// The pane is gone, or never started.
pub(crate) fn forget(s: &mut Supervisor, pane_id: PaneId) {
    s.bus.slot::<ProfilesState>().panes.remove(&pane_id);
}

/// Whether this server can enforce a profile's `[sandbox]` (S7, B14: the `sandbox`
/// feature). `sandbox-linux` and `sandbox-macos` replace this with their platform
/// check; until then no profile with a `[sandbox]` table starts.
fn sandbox_enforceable() -> bool {
    false
}

/// Add `pane.profiles`.
pub fn register(registry: &mut Registry, _core: &Core) {
    registry.add(NativeMethod::unary(
        tilr_bus::methods::PANE_PROFILES.name,
        |ctx, payload| async move {
            check_request(&ctx, &payload)?;
            let listed = tokio::task::spawn_blocking(|| {
                let Some(dir) = profile::dir() else {
                    return Ok(Vec::new());
                };
                profile::list(dir, profile::home().as_deref())
            })
            .await
            .map_err(|err| BusError::handler_error(format!("pane.profiles: {err}")))?
            .map_err(|err| BusError::handler_error(format!("pane.profiles: {err}")))?;
            let infos = listed.into_iter().map(|row| ProfileInfo {
                name: row.name,
                description: row.description,
                sandboxed: row.sandboxed,
                error: row.error,
            });
            if ctx.wants_json() {
                let rows: Vec<serde_json::Value> = infos
                    .map(|info| {
                        serde_json::json!({
                            "name": info.name,
                            "description": info.description,
                            "sandboxed": info.sandboxed,
                            "error": info.error,
                        })
                    })
                    .collect();
                let body = serde_json::json!({ "profiles": rows }).to_string();
                return Ok(Reply::new(body).with_content_type(CONTENT_TYPE_JSON));
            }
            Ok(Reply::encode(&ProfilesReply {
                profiles: infos.collect(),
            }))
        },
    ));
}

/// `ProfilesRequest` has no fields: accept an empty or well-formed body, in either
/// encoding, and refuse anything else.
fn check_request(ctx: &CallCtx, payload: &[u8]) -> Result<(), BusError> {
    if ctx.wants_json() {
        if payload.is_empty() {
            return Ok(());
        }
        return match serde_json::from_slice::<serde_json::Value>(payload) {
            Ok(serde_json::Value::Object(_) | serde_json::Value::Null) => Ok(()),
            Ok(_) => Err(BusError::bad_request(
                "pane.profiles: the request is not an object",
            )),
            Err(err) => Err(BusError::bad_request(format!("pane.profiles: {err}"))),
        };
    }
    ProfilesRequest::decode(payload)
        .map(|_| ())
        .map_err(|err| BusError::bad_request(format!("pane.profiles: {err}")))
}

/// What a `pane.create` that names a profile starts (P4, P5).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resolved {
    /// What the pane keeps for life (`PaneInfo.profile` is `snapshot.name`;
    /// `PaneInfo.sandboxed` is `snapshot.sandbox.is_some()`).
    pub snapshot: Snapshot,
    /// The program to run: the request's `command` if not empty, else the profile's;
    /// empty: the user's login shell.
    pub command: Vec<String>,
    /// The directory to start in: the request's `cwd` if it names an existing
    /// directory, else the profile's if it does; `None`: the home directory.
    pub cwd: Option<PathBuf>,
    /// The profile's `[env]`, to be set over the server's environment (before the
    /// terminal variables `TERM`, `COLORTERM`, `SHELL` and `TILR_PANE`) in an
    /// unsandboxed pane; a sandboxed pane's environment is `sandbox-*`'s (S6).
    pub env: Vec<(String, String)>,
}

/// The seam for `pane.create` (`bus-core-panes` calls it): asks the authorizer for
/// `PaneCreate`, then, for a request that names a profile, checks the name, reads the
/// file now, refuses a sandbox this server cannot enforce and lets the request's
/// command and directory win (P4, P5, P7).
///
/// `Ok(None)`: the request names no profile; `pane.create` goes on as without one.
/// `Err`: `denied` from the authorizer, or `bad_request` with the text of P7
/// (`create pane: bad profile name: …`, `create pane: no such profile: …`,
/// `create pane: profile <n>: <file>: <reason>`, `create pane: profile <n> asks for a
/// sandbox, which this server cannot enforce`); no pane starts.
pub fn prepare_create(
    authz: &dyn Authorizer,
    ctx: &CallCtx,
    request: &CreateRequest,
) -> Result<Option<Resolved>, BusError> {
    let overrides = !request.command.is_empty() || !request.cwd.is_empty();
    authz.authorize(
        &ctx.principal,
        ctx.call_credential.as_deref(),
        &AuthzRequest::PaneCreate {
            profile: &request.profile,
            overrides,
            call: ForCall {
                method: &ctx.method,
                target: &ctx.target,
            },
        },
    )?;
    resolve_in(profile::dir(), profile::home().as_deref(), request)
}

/// [`prepare_create`] without the authorizer, over the directory `dir`, for the user
/// whose home directory is `home`.
fn resolve_in(
    dir: Option<&std::path::Path>,
    home: Option<&std::path::Path>,
    request: &CreateRequest,
) -> Result<Option<Resolved>, BusError> {
    if request.profile.is_empty() {
        return Ok(None);
    }
    let name = &request.profile;
    if !profile::valid_name(name) {
        return Err(refused(name, &Error::BadName(name.clone())));
    }
    let snapshot = match dir {
        Some(dir) => profile::load(dir, name, home),
        None => Err(Error::NoSuch(name.clone())),
    }
    .map_err(|err| refused(name, &err))?;
    if snapshot.sandbox.is_some() && !sandbox_enforceable() {
        return Err(BusError::bad_request(format!(
            "create pane: profile {name} asks for a sandbox, which this server cannot enforce"
        )));
    }
    let command = if request.command.is_empty() {
        snapshot.argv.clone()
    } else {
        request.command.clone()
    };
    let existing = |path: &std::path::Path| path.is_dir();
    let cwd = Some(PathBuf::from(&request.cwd))
        .filter(|cwd| !request.cwd.is_empty() && existing(cwd))
        .or_else(|| snapshot.cwd.clone().filter(|cwd| existing(cwd)));
    let env = snapshot
        .env
        .iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    Ok(Some(Resolved {
        snapshot,
        command,
        cwd,
        env,
    }))
}

/// The refusal of P7 for a profile `name` that does not load.
fn refused(name: &str, err: &Error) -> BusError {
    match err {
        Error::BadName(_) | Error::NoSuch(_) => {
            BusError::bad_request(format!("create pane: {err}"))
        }
        Error::Invalid(reason) => {
            BusError::bad_request(format!("create pane: profile {name}: {reason}"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::tests::{Scratch, write};
    use std::path::Path;

    /// A profiles directory in `tree` holding `files` (mode 0600).
    fn dir_with(tree: &Scratch, files: &[(&str, &str)]) -> PathBuf {
        let dir = tree.dir("profiles");
        for (name, text) in files {
            write(&dir, name, text, 0o600);
        }
        dir
    }

    fn request(profile: &str, command: &[&str], cwd: &str) -> CreateRequest {
        CreateRequest {
            profile: profile.into(),
            command: command.iter().map(|s| (*s).to_owned()).collect(),
            cwd: cwd.into(),
            ..CreateRequest::default()
        }
    }

    fn resolve(
        tree: &Scratch,
        dir: &Path,
        request: &CreateRequest,
    ) -> Result<Option<Resolved>, BusError> {
        resolve_in(Some(dir), tree.home(), request)
    }

    fn err(tree: &Scratch, dir: &Path, request: &CreateRequest) -> String {
        resolve(tree, dir, request).unwrap_err().message
    }

    #[test]
    fn no_profile_is_left_to_pane_create() {
        let tree = Scratch::new();
        let dir = dir_with(&tree, &[]);
        assert_eq!(resolve(&tree, &dir, &request("", &["x"], "")), Ok(None));
    }

    #[test]
    fn a_bad_name_is_refused_before_any_file_is_touched() {
        let tree = Scratch::new();
        let dir = dir_with(&tree, &[]);
        for name in ["../server", "/etc/passwd", ".hidden", "a/b"] {
            assert_eq!(
                err(&tree, &dir, &request(name, &[], "")),
                format!("create pane: bad profile name: {name}")
            );
        }
        // Even where there is no directory at all.
        let gone = dir.join("absent");
        assert_eq!(
            err(&tree, &gone, &request("a/b", &[], "")),
            "create pane: bad profile name: a/b"
        );
        assert_eq!(
            err(&tree, &gone, &request("nope", &[], "")),
            "create pane: no such profile: nope"
        );
    }

    #[test]
    fn an_unknown_or_invalid_profile_is_bad_request_with_its_file() {
        let tree = Scratch::new();
        let dir = dir_with(&tree, &[("bad.toml", "comand = []")]);
        assert_eq!(
            err(&tree, &dir, &request("nope", &[], "")),
            "create pane: no such profile: nope"
        );
        let text = err(&tree, &dir, &request("bad", &[], ""));
        assert!(text.starts_with("create pane: profile bad: "), "{text}");
        assert!(
            text.contains("bad.toml") && text.contains("comand"),
            "{text}"
        );
        let error = resolve(&tree, &dir, &request("bad", &[], "")).unwrap_err();
        assert_eq!(error.code, tilr_bus::codes::BAD_REQUEST);
    }

    #[test]
    fn the_request_wins_and_the_profiles_environment_stays() {
        let tree = Scratch::new();
        let other = tree.dir("other");
        let work = tree.dir("work");
        let text = format!(
            "command = [\"/bin/sh\"]\ncwd = \"{}\"\n[env]\nGREETING = \"hi\"",
            work.display()
        );
        let dir = dir_with(&tree, &[("demo.toml", &text)]);
        // Nothing in the request: the profile's.
        let plain = resolve(&tree, &dir, &request("demo", &[], ""))
            .unwrap()
            .unwrap();
        assert_eq!(plain.command, ["/bin/sh"]);
        assert_eq!(plain.cwd.as_deref(), Some(work.as_path()));
        assert_eq!(plain.env, [("GREETING".to_owned(), "hi".to_owned())]);
        assert_eq!(plain.snapshot.name, "demo");
        // The request's command and an existing directory win.
        let given = resolve(
            &tree,
            &dir,
            &request("demo", &["/bin/bash", "--norc"], other.to_str().unwrap()),
        )
        .unwrap()
        .unwrap();
        assert_eq!(given.command, ["/bin/bash", "--norc"]);
        assert_eq!(given.cwd.as_deref(), Some(other.as_path()));
        assert_eq!(given.env, plain.env);
        assert_eq!(given.snapshot, plain.snapshot);
        // A directory that does not exist falls back to the profile's.
        let missing = resolve(&tree, &dir, &request("demo", &[], "/no/such/directory"))
            .unwrap()
            .unwrap();
        assert_eq!(missing.cwd.as_deref(), Some(work.as_path()));
    }

    #[test]
    fn a_profile_without_a_directory_falls_back_to_home() {
        let tree = Scratch::new();
        let dir = dir_with(&tree, &[("demo.toml", "cwd = \"/no/such/directory\"")]);
        let resolved = resolve(&tree, &dir, &request("demo", &[], ""))
            .unwrap()
            .unwrap();
        assert!(resolved.command.is_empty());
        assert_eq!(resolved.cwd, None);
    }

    #[test]
    fn prepare_create_refuses_a_sandbox_it_cannot_enforce() {
        let tree = Scratch::new();
        let dir = dir_with(
            &tree,
            &[(
                "box.toml",
                "command = [\"/bin/sh\"]\n[sandbox]\nread_write = [\"~/w\"]",
            )],
        );
        assert_eq!(
            err(&tree, &dir, &request("box", &["/bin/sh", "-c", "true"], "")),
            "create pane: profile box asks for a sandbox, which this server cannot enforce"
        );
    }
}
