//! # Saving settings
//!
//! **Purpose:** make `:set!` stick.
//!
//! **Responsibility:** change one key in the user's config file without
//! disturbing anything else in it — the comments, the order, the blank lines
//! the user wrote stay where they were.
//!
//! **Public API:** [`Value`], [`store`].

use std::path::Path;

use anyhow::{Context, Result};
use toml_edit::DocumentMut;

/// A setting's new value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    /// `true` or `false`.
    Bool(bool),
    /// A whole number.
    Int(i64),
    /// A string.
    Text(String),
}

/// Set `key` to `value` in the config file at `path`, creating it if needed.
///
/// # Errors
/// Returns an error if the file is not valid TOML or cannot be written.
pub fn store(path: &Path, key: &str, value: Value) -> Result<()> {
    let text = if path.is_file() {
        std::fs::read_to_string(path).with_context(|| format!("cannot read {}", path.display()))?
    } else {
        String::new()
    };
    let mut document: DocumentMut = text
        .parse()
        .with_context(|| format!("{} is not valid TOML", path.display()))?;
    document[key] = match value {
        Value::Bool(flag) => toml_edit::value(flag),
        Value::Int(number) => toml_edit::value(number),
        Value::Text(text) => toml_edit::value(text),
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("cannot create {}", parent.display()))?;
    }
    crate::filesystem::write_file(path, &document.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stored_value_leaves_the_rest_of_the_file_alone() {
        let directory = std::env::temp_dir().join("termi-persist-config");
        let _ = std::fs::remove_dir_all(&directory);
        let path = directory.join("config.toml");
        std::fs::create_dir_all(&directory).expect("create directory");
        std::fs::write(
            &path,
            "# my settings\ntheme = \"light\"  # bright\n\n[keys.normal]\nQ = \":q\"\n",
        )
        .expect("write config");

        store(&path, "tab_width", Value::Int(2)).expect("store number");
        store(&path, "theme", Value::Text("dark".into())).expect("store text");

        let text = std::fs::read_to_string(&path).expect("read back");
        assert!(text.starts_with("# my settings\n"), "{text}");
        assert!(text.contains("theme = \"dark\""), "{text}");
        assert!(text.contains("tab_width = 2"), "{text}");
        assert!(text.contains("Q = \":q\""), "{text}");
        let config = crate::config::Config::parse(&text).expect("still a valid config");
        assert_eq!(config.tab_width, 2);
    }
}
