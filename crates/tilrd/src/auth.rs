//! The access token: a secret the server makes the first time it runs and
//! keeps on disk, readable by the user only. A client proves it may use the
//! server by sending the token in `Hello`. `tilrd token` prints it.

use std::fs;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;

use anyhow::{Context, Result, ensure};

use crate::identity;

const TOKEN_FILE: &str = "token";
const TOKEN_BYTES: usize = 32;

fn token_path() -> Result<PathBuf> {
    Ok(identity::state_dir()
        .context("no home directory")?
        .join(TOKEN_FILE))
}

/// The token of the server on this machine, if it has made one.
pub fn stored_token() -> Option<String> {
    let token = fs::read_to_string(token_path().ok()?).ok()?;
    let token = token.trim();
    (!token.is_empty()).then(|| token.to_owned())
}

/// The token of this server; made now if there is none yet.
pub fn token() -> Result<String> {
    let path = token_path()?;
    if let Some(dir) = path.parent() {
        identity::create_private_dir(dir)?;
    }
    let created = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path);
    match created {
        Ok(mut file) => {
            let mut secret = [0u8; TOKEN_BYTES];
            getrandom::fill(&mut secret).context("no random numbers from the system")?;
            let token: String = secret.iter().map(|byte| format!("{byte:02x}")).collect();
            writeln!(file, "{token}").with_context(|| format!("write {}", path.display()))?;
            Ok(token)
        }
        Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
            let token = stored_token();
            ensure!(token.is_some(), "{} holds no token", path.display());
            Ok(token.unwrap_or_default())
        }
        Err(err) => Err(err).with_context(|| format!("create {}", path.display())),
    }
}

/// Whether `given` is the token. Takes the same time wherever they differ.
pub fn matches(token: &str, given: &str) -> bool {
    token.len() == given.len()
        && token
            .bytes()
            .zip(given.bytes())
            .fold(0, |differ, (a, b)| differ | (a ^ b))
            == 0
}

/// `tilrd token`.
pub fn print() -> Result<()> {
    println!("{}", token()?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::matches;

    #[test]
    fn only_the_token_matches() {
        assert!(matches("abc123", "abc123"));
        assert!(!matches("abc123", "abc124"));
        assert!(!matches("abc123", "abc12"));
        assert!(!matches("abc123", ""));
    }
}
