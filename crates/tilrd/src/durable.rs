//! Putting a written file in place so that it survives a power loss: its
//! data synced before the rename, the directory (which holds the new name)
//! synced after it. Used by a finished `FsWrite` and by `tilrd update`.
//!
//! Plain `fsync` everywhere, also on macOS (no `F_FULLFSYNC`), as the reload
//! state files do.

use std::fs::{self, File};
use std::io;
use std::path::Path;

/// Sync a directory: the names created, renamed or removed in it reach the
/// disk.
pub fn sync_dir(dir: &Path) -> io::Result<()> {
    File::open(dir)?.sync_all()
}

/// Rename `from` (a file whose data is already synced) over `to` and sync
/// the directory of `to`. An error after the rename leaves the file renamed.
pub fn rename_synced(from: &Path, to: &Path) -> io::Result<()> {
    fs::rename(from, to)?;
    sync_dir(parent(to))
}

/// The directory a path is in; `.` for a bare name.
pub fn parent(path: &Path) -> &Path {
    match path.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => dir,
        _ => Path::new("."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_synced_rename_puts_the_file_in_place() {
        let dir = std::env::temp_dir().join(format!("tilrd-durable-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let (from, to) = (dir.join("new"), dir.join("file"));
        fs::write(&to, b"old").unwrap();
        fs::write(&from, b"new").unwrap();
        rename_synced(&from, &to).unwrap();
        assert_eq!(fs::read(&to).unwrap(), b"new");
        assert!(!from.exists());
        assert!(rename_synced(&from, &to).is_err(), "a missing source fails");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_parent_of_a_bare_name_is_the_current_directory() {
        assert_eq!(parent(Path::new("file")), Path::new("."));
        assert_eq!(parent(Path::new("/a/file")), Path::new("/a"));
    }
}
