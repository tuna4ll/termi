//! # Picker
//!
//! **Purpose:** rank selectable application items against a short query.
//!
//! **Responsibility:** hold picker state, fuzzy-match labels and discover
//! project files while respecting ignore files.
//!
//! **Public API:** [`Picker`], [`PickerItem`], [`PickerTarget`], [`project_files`].

use std::path::{Path, PathBuf};

use ignore::WalkBuilder;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PickerTarget {
    File(PathBuf),
    Buffer(usize),
    Command(String),
    Theme(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PickerItem {
    pub label: String,
    pub target: PickerTarget,
    pub priority: usize,
}

impl PickerItem {
    #[must_use]
    pub fn new(label: impl Into<String>, target: PickerTarget) -> Self {
        Self {
            label: label.into(),
            target,
            priority: usize::MAX,
        }
    }

    #[must_use]
    pub const fn prioritised(mut self, priority: usize) -> Self {
        self.priority = priority;
        self
    }
}

#[derive(Debug, Clone)]
pub struct Picker {
    pub title: String,
    pub query: String,
    items: Vec<PickerItem>,
    matches: Vec<usize>,
    pub selected: usize,
}

impl Picker {
    #[must_use]
    pub fn new(title: impl Into<String>, items: Vec<PickerItem>) -> Self {
        let mut picker = Self {
            title: title.into(),
            query: String::new(),
            items,
            matches: Vec::new(),
            selected: 0,
        };
        picker.update_matches();
        picker
    }

    #[must_use]
    pub fn matches(&self) -> impl Iterator<Item = &PickerItem> {
        self.matches.iter().map(|index| &self.items[*index])
    }

    #[must_use]
    pub fn selected(&self) -> Option<&PickerItem> {
        self.matches
            .get(self.selected)
            .map(|index| &self.items[*index])
    }

    pub fn push(&mut self, ch: char) {
        self.query.push(ch);
        self.update_matches();
    }

    pub fn pop(&mut self) {
        self.query.pop();
        self.update_matches();
    }

    pub fn move_selection(&mut self, delta: isize) {
        let last = self.matches.len().saturating_sub(1);
        if delta < 0 {
            self.selected = self.selected.saturating_sub(delta.unsigned_abs());
        } else {
            self.selected = self.selected.saturating_add(delta as usize).min(last);
        }
    }

    fn update_matches(&mut self) {
        let query = self.query.to_lowercase();
        let mut ranked: Vec<(usize, usize, usize)> = self
            .items
            .iter()
            .enumerate()
            .filter_map(|(index, item)| {
                fuzzy_score(&item.label.to_lowercase(), &query)
                    .map(|score| (index, score, item.priority))
            })
            .collect();
        ranked.sort_by_key(|(index, score, priority)| (*score, *priority, *index));
        self.matches = ranked.into_iter().map(|(index, _, _)| index).collect();
        self.selected = self.selected.min(self.matches.len().saturating_sub(1));
    }
}

#[must_use]
pub fn project_files(root: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = WalkBuilder::new(root)
        .hidden(true)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .require_git(false)
        .build()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_some_and(|kind| kind.is_file()))
        .map(|entry| entry.into_path())
        .collect();
    files.sort();
    files
}

fn fuzzy_score(candidate: &str, query: &str) -> Option<usize> {
    if query.is_empty() {
        return Some(0);
    }
    let mut from = 0;
    let mut first = None;
    let mut previous = None;
    let mut gaps = 0;
    for needle in query.chars() {
        let found = candidate[from..].find(needle)? + from;
        if let Some(last) = previous {
            gaps += found.saturating_sub(last + needle.len_utf8());
        } else {
            first = Some(found);
        }
        previous = Some(found);
        from = found + needle.len_utf8();
    }
    Some(gaps * 4 + first.unwrap_or(0) + candidate.len().saturating_sub(query.len()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(label: &str) -> PickerItem {
        PickerItem::new(label, PickerTarget::Command(label.to_string()))
    }

    #[test]
    fn contiguous_matches_rank_first() {
        let mut picker = Picker::new("files", vec![item("src/picker.rs"), item("src/parser.rs")]);
        for ch in "pick".chars() {
            picker.push(ch);
        }
        assert_eq!(
            picker.selected().map(|item| item.label.as_str()),
            Some("src/picker.rs")
        );
    }

    #[test]
    fn recent_items_win_an_empty_query() {
        let picker = Picker::new(
            "files",
            vec![item("old").prioritised(4), item("recent").prioritised(0)],
        );
        assert_eq!(
            picker.selected().map(|item| item.label.as_str()),
            Some("recent")
        );
    }

    #[test]
    fn project_walk_obeys_gitignore() {
        let root = std::env::temp_dir().join("termi-picker-ignore");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("target")).expect("create fixture");
        std::fs::write(root.join(".gitignore"), "target/\n").expect("write ignore file");
        std::fs::write(root.join("main.rs"), "").expect("write fixture");
        std::fs::write(root.join("target/generated.rs"), "").expect("write fixture");

        let files = project_files(&root);
        assert!(files.contains(&root.join("main.rs")));
        assert!(!files.contains(&root.join("target/generated.rs")));
    }
}
