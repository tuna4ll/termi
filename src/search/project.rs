//! # Project search
//!
//! **Purpose:** find text across every file in the project, the way `grep -rn`
//! would, but respecting `.gitignore`.
//!
//! **Responsibility:** walk the project's files, skip the ones that are not
//! text or are too big to be worth reading, and report where a [`Matcher`]
//! hits. The caller decides what a hit means — a picker entry, a file to
//! rewrite.
//!
//! **Public API:** [`Hit`], [`search`], [`text_of`].

use std::path::{Path, PathBuf};

use super::Matcher;
use crate::picker::project_files;

/// Files larger than this are skipped: they are build output or data far more
/// often than source anyone is searching.
const LARGEST_FILE: u64 = 4 << 20;

/// One match: where it is and the line it is on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    /// The file.
    pub path: PathBuf,
    /// Zero-based line.
    pub line: usize,
    /// Zero-based character column where the match starts.
    pub col: usize,
    /// The line's text, trimmed for display.
    pub text: String,
}

/// The text of `path`, unless it is too big, unreadable or binary.
#[must_use]
pub fn text_of(path: &Path) -> Option<String> {
    let size = std::fs::metadata(path).ok()?.len();
    if size > LARGEST_FILE {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    crate::filesystem::decode(&bytes).ok().map(|(text, _)| text)
}

/// Every hit for `matcher` under `root`, at most `limit` of them, in file
/// order.
#[must_use]
pub fn search(root: &Path, matcher: &Matcher, limit: usize) -> Vec<Hit> {
    let mut hits = Vec::new();
    for path in project_files(root) {
        let Some(text) = text_of(&path) else {
            continue;
        };
        for (line, content) in text.lines().enumerate() {
            if let Some(found) = matcher.find_all(content).first() {
                hits.push(Hit {
                    path: path.clone(),
                    line,
                    col: found.start,
                    text: content.trim().to_string(),
                });
                if hits.len() >= limit {
                    return hits;
                }
            }
        }
    }
    hits
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hits_carry_their_place_and_skip_ignored_files() {
        let root = std::env::temp_dir().join("termi-project-search");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("build")).expect("create fixture");
        std::fs::write(root.join(".gitignore"), "build/\n").expect("write ignore");
        std::fs::write(root.join("a.rs"), "fn main() {\n    todo!()\n}\n").expect("write a");
        std::fs::write(root.join("build/out.rs"), "todo!()\n").expect("write ignored");
        std::fs::write(root.join("blob.bin"), b"todo\0\0").expect("write binary");

        let matcher = Matcher::new("todo", false, None).expect("valid pattern");
        let hits = search(&root, &matcher, 10);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].path, root.join("a.rs"));
        assert_eq!((hits[0].line, hits[0].col), (1, 4));
        assert_eq!(hits[0].text, "todo!()");
    }
}
