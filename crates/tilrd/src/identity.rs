//! The id of this server: a UUID made the first time the server runs on a
//! machine and kept on disk, so it is the same after a restart. Clients get
//! it in `HelloAck` and can tell one server from another by it.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use uuid::Uuid;

/// Overrides where the server keeps what must outlive a restart.
const STATE_DIR_ENV: &str = "TILR_STATE_DIR";
const ID_FILE: &str = "server-id";

/// Directory for what the server keeps between runs.
pub fn state_dir() -> Option<PathBuf> {
    let set = |name: &str| std::env::var_os(name).filter(|dir| !dir.is_empty());
    if let Some(dir) = set(STATE_DIR_ENV) {
        return Some(PathBuf::from(dir));
    }
    if let Some(dir) = set("XDG_STATE_HOME") {
        return Some(Path::new(&dir).join("tilr"));
    }
    Some(std::env::home_dir()?.join(".local/state/tilr"))
}

/// Make the state directory (or another of the user's own) if it is not
/// there: private to the user (0700), its parents as usual.
pub fn create_private_dir(dir: &Path) -> Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    if let Some(parent) = dir.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    match std::fs::DirBuilder::new().mode(0o700).create(dir) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists && dir.is_dir() => Ok(()),
        Err(err) => Err(err).with_context(|| format!("create {}", dir.display())),
    }
}

/// The id of this server. A server that cannot keep one on disk gets one
/// that lasts until it stops.
pub fn server_id() -> Uuid {
    let kept = state_dir()
        .context("no home directory")
        .and_then(|dir| read_or_create(&dir));
    kept.unwrap_or_else(|err| {
        tracing::warn!("this server's id will change when it restarts: {err:#}");
        Uuid::new_v4()
    })
}

fn read_or_create(dir: &Path) -> Result<Uuid> {
    let path = dir.join(ID_FILE);
    if !path.exists() {
        create_private_dir(dir)?;
        // Written aside and linked into place: the file appears whole, and a
        // server starting at the same moment does not replace the id of this
        // one, it reads it.
        let staged = dir.join(format!("{ID_FILE}.{}", std::process::id()));
        {
            use std::io::Write;
            use std::os::unix::fs::OpenOptionsExt;
            std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o600)
                .open(&staged)
                .and_then(|mut file| writeln!(file, "{}", Uuid::new_v4()))
                .with_context(|| format!("write {}", staged.display()))?;
        }
        let linked = std::fs::hard_link(&staged, &path);
        let _ = std::fs::remove_file(&staged);
        match linked {
            Err(err) if err.kind() != std::io::ErrorKind::AlreadyExists => {
                return Err(err).with_context(|| format!("create {}", path.display()));
            }
            _ => {}
        }
    }
    std::fs::read_to_string(&path)
        .with_context(|| format!("read {}", path.display()))?
        .trim()
        .parse()
        .with_context(|| format!("{} does not hold an id", path.display()))
}
