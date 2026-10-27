//! # Swap files
//!
//! **Purpose:** keep unsaved work safe from a crash, a killed terminal or a
//! power cut.
//!
//! **Responsibility:** write the text of a modified buffer to the editor's
//! data directory every few seconds, remove it once the buffer is saved or
//! closed, and find it again when the same file is next opened. One swap file
//! per edited file, named by a hash of its path so two projects never collide.
//!
//! **Public API:** [`Swap`], [`path_for`], [`write`], [`read`], [`remove`].

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use crate::undo::persist::fingerprint;

/// What a swap file holds.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Swap {
    /// The file the text belongs to.
    pub path: PathBuf,
    /// The process that wrote it, to tell a crash from a second editor.
    pub pid: u32,
    /// The unsaved text.
    pub text: String,
}

impl Swap {
    /// Whether the editor that wrote this swap file is still running.
    #[must_use]
    pub fn writer_alive(&self) -> bool {
        if self.pid == std::process::id() {
            return true;
        }
        // Only Linux answers this without a system call the editor does not
        // otherwise need; elsewhere a leftover swap is assumed to be a crash.
        cfg!(target_os = "linux") && Path::new("/proc").join(self.pid.to_string()).exists()
    }
}

/// Where the swap file for `file` lives inside `directory`.
#[must_use]
pub fn path_for(directory: &Path, file: &Path) -> PathBuf {
    let canonical = file.canonicalize().unwrap_or_else(|_| {
        std::env::current_dir()
            .map(|cwd| cwd.join(file))
            .unwrap_or_else(|_| file.to_path_buf())
    });
    let key = fingerprint(canonical.to_string_lossy().as_bytes());
    let name = file.file_name().map_or_else(
        || "unnamed".to_string(),
        |name| name.to_string_lossy().into_owned(),
    );
    directory.join(format!("{name}.{key:016x}.swp"))
}

/// Save `text` as the unsaved state of `file`.
///
/// # Errors
/// Returns an error if the directory or the swap file cannot be written.
pub fn write(directory: &Path, file: &Path, text: &str) -> anyhow::Result<()> {
    std::fs::create_dir_all(directory)?;
    let swap = Swap {
        path: file.to_path_buf(),
        pid: std::process::id(),
        text: text.to_string(),
    };
    super::write_file(&path_for(directory, file), &serde_json::to_string(&swap)?)
}

/// The swap file for `file`, with the time it was last written.
#[must_use]
pub fn read(directory: &Path, file: &Path) -> Option<(Swap, SystemTime)> {
    let location = path_for(directory, file);
    let modified = std::fs::metadata(&location)
        .and_then(|m| m.modified())
        .ok()?;
    let json = std::fs::read_to_string(location).ok()?;
    let swap = serde_json::from_str(&json).ok()?;
    Some((swap, modified))
}

/// Delete the swap file for `file`, if there is one.
pub fn remove(directory: &Path, file: &Path) {
    let _ = std::fs::remove_file(path_for(directory, file));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_swap_file_round_trips_and_goes_away() {
        let directory = std::env::temp_dir().join("termi-swap");
        let _ = std::fs::remove_dir_all(&directory);
        let file = directory.join("draft.txt");

        write(&directory, &file, "unsaved").expect("write swap");
        let (swap, _) = read(&directory, &file).expect("swap exists");
        assert_eq!(swap.text, "unsaved");
        assert!(swap.writer_alive(), "this process wrote it");

        remove(&directory, &file);
        assert!(read(&directory, &file).is_none());
    }
}
