//! # Persistent undo
//!
//! **Purpose:** keep a file's undo history across restarts.
//!
//! **Responsibility:** write a buffer's [`History`] next to nothing — into the
//! editor's data directory, keyed by the file's path — whenever the file is
//! saved, and read it back when the file is opened again. A history is only
//! valid for the exact text it was recorded against, so each one carries a
//! hash of that text; if the file changed in between, the history is ignored
//! rather than replayed into the wrong text.
//!
//! **Public API:** [`save`], [`load`], [`fingerprint`].

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::{History, Snapshot};

/// What is written to disk for one file.
#[derive(Serialize, Deserialize)]
struct Stored {
    /// The file the history belongs to, for anyone reading the directory.
    path: PathBuf,
    /// [`fingerprint`] of the text at the history's current state.
    text: u64,
    history: Snapshot,
}

/// A stable 64-bit FNV-1a hash.
///
/// The standard library's hasher may change between Rust releases, and these
/// hashes live on disk across upgrades, so the algorithm is spelled out here.
#[must_use]
pub fn fingerprint(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    hash
}

/// Where the history for `file` lives inside `directory`.
fn location(directory: &Path, file: &Path) -> PathBuf {
    let canonical = file.canonicalize().unwrap_or_else(|_| file.to_path_buf());
    let key = fingerprint(canonical.to_string_lossy().as_bytes());
    directory.join(format!("{key:016x}.json"))
}

/// Write `history` for `file`, whose text is now `text`.
///
/// # Errors
/// Returns an error if the directory cannot be created or the file written.
pub fn save(directory: &Path, file: &Path, text: &str, history: &History) -> anyhow::Result<()> {
    std::fs::create_dir_all(directory)?;
    let stored = Stored {
        path: file.to_path_buf(),
        text: fingerprint(text.as_bytes()),
        history: history.snapshot(),
    };
    let json = serde_json::to_string(&stored)?;
    crate::filesystem::write_file(&location(directory, file), &json)
}

/// Read the history for `file`, if one was saved against exactly `text`.
#[must_use]
pub fn load(directory: &Path, file: &Path, text: &str, limit: usize) -> Option<History> {
    let json = std::fs::read_to_string(location(directory, file)).ok()?;
    let stored: Stored = serde_json::from_str(&json).ok()?;
    if stored.text != fingerprint(text.as_bytes()) {
        return None;
    }
    History::restore(stored.history, limit)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::cursor::Position;
    use crate::editor::document::Document;
    use crate::undo::Change;

    #[test]
    fn a_history_comes_back_only_for_the_same_text() {
        let directory = std::env::temp_dir().join("termi-persist-undo");
        let _ = std::fs::remove_dir_all(&directory);
        let file = directory.join("notes.txt");

        let mut document = Document::from_text("", None);
        let mut history = History::default();
        document.insert(0, "hi");
        history.record(
            Change::insertion(0, "hi"),
            Position::ZERO,
            Position::new(0, 2),
        );
        save(&directory, &file, "hi", &history).expect("save history");

        assert!(load(&directory, &file, "changed", 100).is_none());
        let mut loaded = load(&directory, &file, "hi", 100).expect("history for the same text");
        loaded.undo(&mut document);
        assert_eq!(document.text().to_string(), "");
    }
}
