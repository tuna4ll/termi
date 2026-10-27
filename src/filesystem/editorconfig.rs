//! # EditorConfig
//!
//! **Purpose:** honour the `.editorconfig` files a project ships, so its
//! indentation and line endings are right without anyone configuring termi.
//!
//! **Responsibility:** find the `.editorconfig` files that apply to a path —
//! from the file's directory upwards until one says `root = true` — and fold
//! their matching sections into one set of [`Properties`]. Nearer files win
//! over farther ones and later sections over earlier ones, as the
//! specification asks.
//!
//! **Public API:** [`Properties`], [`properties_for`].

use std::path::Path;

use regex::Regex;

use super::Encoding;
use crate::editor::document::LineEnding;

/// The settings `.editorconfig` can make for one file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Properties {
    /// `indent_style = space` (`true`) or `tab` (`false`).
    pub expand_tabs: Option<bool>,
    /// `indent_size`, or `tab_width` when the size is `tab`.
    pub indent_size: Option<usize>,
    /// `tab_width`.
    pub tab_width: Option<usize>,
    /// `end_of_line`.
    pub end_of_line: Option<LineEnding>,
    /// `charset`.
    pub charset: Option<Encoding>,
    /// `trim_trailing_whitespace`.
    pub trim_trailing_whitespace: Option<bool>,
    /// `insert_final_newline`.
    pub insert_final_newline: Option<bool>,
}

impl Properties {
    /// The width one level of indentation takes.
    #[must_use]
    pub fn indent_width(&self) -> Option<usize> {
        self.indent_size.or(self.tab_width)
    }

    fn set(&mut self, key: &str, value: &str) {
        let flag = || match value {
            "true" => Some(true),
            "false" => Some(false),
            _ => None,
        };
        let number = || {
            value
                .parse::<usize>()
                .ok()
                .filter(|width| (1..=16).contains(width))
        };
        match key {
            "indent_style" => {
                self.expand_tabs = match value {
                    "space" => Some(true),
                    "tab" => Some(false),
                    _ => None,
                };
            }
            "indent_size" => {
                // `indent_size = tab` defers to `tab_width`.
                self.indent_size = if value == "tab" { None } else { number() };
            }
            "tab_width" => self.tab_width = number(),
            "end_of_line" => {
                self.end_of_line = match value {
                    "lf" => Some(LineEnding::Lf),
                    "crlf" => Some(LineEnding::Crlf),
                    _ => None,
                };
            }
            "charset" => self.charset = Encoding::from_name(value),
            "trim_trailing_whitespace" => self.trim_trailing_whitespace = flag(),
            "insert_final_newline" => self.insert_final_newline = flag(),
            _ => {}
        }
    }
}

/// The properties every applicable `.editorconfig` gives `path`.
#[must_use]
pub fn properties_for(path: &Path) -> Properties {
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().unwrap_or_default().join(path)
    };
    let mut files = Vec::new();
    let mut directory = path.parent();
    while let Some(current) = directory {
        let candidate = current.join(".editorconfig");
        if let Ok(text) = std::fs::read_to_string(&candidate) {
            let root = is_root(&text);
            files.push((current.to_path_buf(), text));
            if root {
                break;
            }
        }
        directory = current.parent();
    }

    let mut properties = Properties::default();
    for (directory, text) in files.iter().rev() {
        apply(&mut properties, directory, text, &path);
    }
    properties
}

fn is_root(text: &str) -> bool {
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            return false;
        }
        if let Some((key, value)) = line.split_once('=')
            && key.trim().eq_ignore_ascii_case("root")
        {
            return value.trim().eq_ignore_ascii_case("true");
        }
    }
    false
}

fn apply(properties: &mut Properties, directory: &Path, text: &str, file: &Path) {
    let Ok(relative) = file.strip_prefix(directory) else {
        return;
    };
    let relative = relative.to_string_lossy().replace('\\', "/");
    let mut matching = false;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if let Some(section) = line
            .strip_prefix('[')
            .and_then(|rest| rest.strip_suffix(']'))
        {
            matching = matches(section, &relative);
            continue;
        }
        if matching && let Some((key, value)) = line.split_once('=') {
            properties.set(
                &key.trim().to_ascii_lowercase(),
                &value.trim().to_ascii_lowercase(),
            );
        }
    }
}

/// Whether a section glob covers a path relative to its file.
///
/// A glob without a `/` matches a file name at any depth; one with a `/` is
/// anchored to the directory holding the `.editorconfig`.
fn matches(section: &str, relative: &str) -> bool {
    let anchored = section.contains('/');
    let body = translate(section.trim_start_matches('/'));
    let source = if anchored {
        format!("^{body}$")
    } else {
        format!("^(?:.*/)?{body}$")
    };
    Regex::new(&source).is_ok_and(|regex| regex.is_match(relative))
}

/// An EditorConfig glob as a regular expression.
///
/// The syntax is close to a shell's but not the same as any library's: `**`
/// crosses directories even in the middle of a name (`src/**.rs`), and
/// `{1..10}` matches a range of numbers.
fn translate(glob: &str) -> String {
    let chars: Vec<char> = glob.chars().collect();
    let mut out = String::new();
    let mut braces = 0usize;
    let mut index = 0;
    while index < chars.len() {
        let ch = chars[index];
        match ch {
            '*' if chars.get(index + 1) == Some(&'*') => {
                out.push_str(".*");
                index += 1;
            }
            '*' => out.push_str("[^/]*"),
            '?' => out.push_str("[^/]"),
            '[' => {
                let close = chars[index + 1..].iter().position(|c| *c == ']');
                if let Some(close) = close {
                    let inner: String = chars[index + 1..index + 1 + close].iter().collect();
                    let inner = inner
                        .strip_prefix('!')
                        .map_or_else(|| inner.clone(), |rest| format!("^{rest}"));
                    out.push('[');
                    out.push_str(&inner.replace('\\', "\\\\"));
                    out.push(']');
                    index += close + 1;
                } else {
                    out.push_str("\\[");
                }
            }
            '{' => {
                let close = chars[index + 1..].iter().position(|c| *c == '}');
                let range = close.and_then(|close| {
                    let inner: String = chars[index + 1..index + 1 + close].iter().collect();
                    let (low, high) = inner.split_once("..")?;
                    Some((low.parse::<i64>().ok()?, high.parse::<i64>().ok()?, close))
                });
                if let Some((low, high, close)) = range {
                    let (low, high) = (low.min(high), low.max(high));
                    let numbers: Vec<String> = if high - low <= 1000 {
                        (low..=high).map(|n| n.to_string()).collect()
                    } else {
                        vec!["-?\\d+".to_string()]
                    };
                    out.push_str(&format!("(?:{})", numbers.join("|")));
                    index += close + 1;
                } else {
                    braces += 1;
                    out.push_str("(?:");
                }
            }
            ',' if braces > 0 => out.push('|'),
            '}' if braces > 0 => {
                braces -= 1;
                out.push(')');
            }
            '\\' if index + 1 < chars.len() => {
                index += 1;
                out.push_str(&regex::escape(&chars[index].to_string()));
            }
            other => out.push_str(&regex::escape(&other.to_string())),
        }
        index += 1;
    }
    // An unclosed brace is a literal one; close the group so the regex parses.
    for _ in 0..braces {
        out.push(')');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nearer_files_and_later_sections_win() {
        let root = std::env::temp_dir().join("termi-editorconfig");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("web")).expect("create fixture");
        std::fs::write(
            root.join(".editorconfig"),
            "root = true\n\n[*]\nindent_style = space\nindent_size = 4\nend_of_line = lf\n\n[Makefile]\nindent_style = tab\n",
        )
        .expect("write root config");
        std::fs::write(
            root.join("web/.editorconfig"),
            "[*.{js,css}]\nindent_size = 2\ninsert_final_newline = true\n",
        )
        .expect("write nested config");

        let js = properties_for(&root.join("web/app.js"));
        assert_eq!(js.expand_tabs, Some(true));
        assert_eq!(js.indent_width(), Some(2));
        assert_eq!(js.insert_final_newline, Some(true));
        assert_eq!(js.end_of_line, Some(LineEnding::Lf));

        let rust = properties_for(&root.join("src/main.rs"));
        assert_eq!(rust.indent_width(), Some(4));
        assert_eq!(rust.insert_final_newline, None);

        let make = properties_for(&root.join("Makefile"));
        assert_eq!(make.expand_tabs, Some(false));
    }

    #[test]
    fn section_globs() {
        assert!(matches("*.rs", "src/main.rs"));
        assert!(matches("src/*.rs", "src/main.rs"));
        assert!(!matches("src/*.rs", "src/nested/main.rs"));
        assert!(matches("src/**.rs", "src/nested/main.rs"));
        assert!(matches("{Makefile,*.mk}", "build/rules.mk"));
        assert!(matches("file{1..3}.txt", "file2.txt"));
        assert!(!matches("file{1..3}.txt", "file4.txt"));
        assert!(matches("[!a]*.c", "b.c"));
        assert!(!matches("[!a]*.c", "a.c"));
    }
}
