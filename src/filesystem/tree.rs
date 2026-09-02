//! # Directory tree
//!
//! **Purpose:** a browsable view of a directory.
//!
//! **Responsibility:** keep a *flattened* list of visible entries — the shape a
//! list widget can render and index directly — and expand or collapse
//! directories on demand. Only expanded directories are read, so opening the
//! tree on a large project costs one `read_dir`, not a full walk.
//!
//! **Public API:** [`Tree`], [`Entry`].

use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// One visible row of the tree.
#[derive(Debug, Clone)]
pub struct Entry {
    /// Full path on disk.
    pub path: PathBuf,
    /// File name, for display.
    pub name: String,
    /// Whether this is a directory.
    pub is_dir: bool,
    /// Whether an expanded directory.
    pub is_open: bool,
    /// Nesting level, used for indentation.
    pub depth: usize,
}

/// A directory rendered as a flat, indented list.
#[derive(Debug)]
pub struct Tree {
    root: PathBuf,
    entries: Vec<Entry>,
    expanded: HashSet<PathBuf>,
}

impl Tree {
    /// Build a tree rooted at `root`, with the root itself already expanded.
    #[must_use]
    pub fn new(root: PathBuf) -> Self {
        let mut tree = Self {
            root,
            entries: Vec::new(),
            expanded: HashSet::new(),
        };
        tree.expanded.insert(tree.root.clone());
        tree.refresh();
        tree
    }

    /// The directory being browsed.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Visible rows, in display order.
    #[must_use]
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// Directory in which a new entry should be created for this selection.
    ///
    /// A highlighted directory owns the new entry; a highlighted file lends
    /// its parent. An empty tree falls back to the root itself.
    #[must_use]
    pub fn creation_directory(&self, index: usize) -> PathBuf {
        let Some(entry) = self.entries.get(index) else {
            return self.root.clone();
        };
        if entry.is_dir {
            entry.path.clone()
        } else {
            entry
                .path
                .parent()
                .map_or_else(|| self.root.clone(), Path::to_path_buf)
        }
    }

    /// Expand the parents of `path`, refresh, and return its visible row.
    pub fn reveal(&mut self, path: &Path) -> Option<usize> {
        if !path.starts_with(&self.root) {
            self.refresh();
            return None;
        }

        let mut parent = path.parent();
        while let Some(directory) = parent {
            if !directory.starts_with(&self.root) {
                break;
            }
            self.expanded.insert(directory.to_path_buf());
            if directory == self.root {
                break;
            }
            parent = directory.parent();
        }

        self.refresh();
        self.entries.iter().position(|entry| entry.path == path)
    }

    /// Re-read every expanded directory.
    ///
    /// Called on open and whenever the tree may be stale; unexpanded
    /// directories are still not touched.
    pub fn refresh(&mut self) {
        self.entries.clear();
        let root = self.root.clone();
        self.collect(&root, 0);
    }

    /// Expand or collapse the directory at `index`, returning the file to open
    /// when the row is a file instead.
    pub fn activate(&mut self, index: usize) -> Option<PathBuf> {
        let entry = self.entries.get(index)?;
        if !entry.is_dir {
            return Some(entry.path.clone());
        }
        let path = entry.path.clone();
        if !self.expanded.remove(&path) {
            self.expanded.insert(path);
        }
        self.refresh();
        None
    }

    /// Read one directory and append its children, recursing into the ones that
    /// are expanded.
    ///
    /// An unreadable directory is skipped rather than reported: a permission
    /// error on one folder should not empty the whole panel.
    fn collect(&mut self, directory: &Path, depth: usize) {
        let Ok(read) = std::fs::read_dir(directory) else {
            return;
        };
        let mut children: Vec<Entry> = read
            .flatten()
            .filter_map(|item| {
                let path = item.path();
                let name = path.file_name()?.to_str()?.to_string();
                // Dotfiles are hidden; they are rarely what someone is browsing
                // for and they dominate a project root.
                if name.starts_with('.') {
                    return None;
                }
                let is_dir = item.file_type().is_ok_and(|kind| kind.is_dir());
                Some(Entry {
                    is_open: is_dir && self.expanded.contains(&path),
                    path,
                    name,
                    is_dir,
                    depth,
                })
            })
            .collect();

        // Directories first, then files, each alphabetically — the ordering
        // every file browser uses, and stable across platforms unlike `read_dir`.
        children.sort_by(|a, b| {
            b.is_dir
                .cmp(&a.is_dir)
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });

        for child in children {
            let recurse = child.is_open.then(|| child.path.clone());
            self.entries.push(child);
            if let Some(path) = recurse {
                self.collect(&path, depth + 1);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a small directory layout under the system temp directory.
    fn fixture(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("termi-tree-{name}"));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("src")).expect("create fixture");
        std::fs::create_dir_all(root.join(".hidden")).expect("create fixture");
        std::fs::write(root.join("Cargo.toml"), "").expect("create fixture");
        std::fs::write(root.join("src/main.rs"), "").expect("create fixture");
        root
    }

    #[test]
    fn directories_sort_before_files() {
        let tree = Tree::new(fixture("sorting"));
        let names: Vec<&str> = tree.entries().iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["src", "Cargo.toml"]);
    }

    #[test]
    fn hidden_entries_are_skipped() {
        let tree = Tree::new(fixture("hidden"));
        assert!(tree.entries().iter().all(|e| e.name != ".hidden"));
    }

    #[test]
    fn expanding_a_directory_reveals_its_children() {
        let mut tree = Tree::new(fixture("expand"));
        assert_eq!(tree.entries().len(), 2);

        assert_eq!(tree.activate(0), None);
        let names: Vec<&str> = tree.entries().iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["src", "main.rs", "Cargo.toml"]);
        assert_eq!(tree.entries()[1].depth, 1);

        // Collapsing puts it back.
        assert_eq!(tree.activate(0), None);
        assert_eq!(tree.entries().len(), 2);
    }

    #[test]
    fn activating_a_file_returns_its_path() {
        let root = fixture("activate");
        let mut tree = Tree::new(root.clone());
        assert_eq!(tree.activate(1), Some(root.join("Cargo.toml")));
    }

    #[test]
    fn an_out_of_range_index_is_ignored() {
        let mut tree = Tree::new(fixture("range"));
        assert_eq!(tree.activate(99), None);
    }

    #[test]
    fn creation_uses_a_directory_or_a_files_parent() {
        let root = fixture("creation-directory");
        let tree = Tree::new(root.clone());

        assert_eq!(tree.creation_directory(0), root.join("src"));
        assert_eq!(tree.creation_directory(1), root);
    }

    #[test]
    fn creation_in_an_empty_tree_uses_the_root() {
        let root = fixture("empty-creation");
        std::fs::remove_dir_all(root.join("src")).expect("remove fixture directory");
        std::fs::remove_file(root.join("Cargo.toml")).expect("remove fixture file");
        let tree = Tree::new(root.clone());

        assert_eq!(tree.creation_directory(0), root);
    }

    #[test]
    fn revealing_a_file_opens_its_parent_and_finds_its_row() {
        let root = fixture("reveal");
        let mut tree = Tree::new(root.clone());
        let path = root.join("src/main.rs");

        let index = tree.reveal(&path).expect("file should be visible");

        assert_eq!(tree.entries()[index].path, path);
        assert!(tree.entries()[0].is_open);
    }
}
