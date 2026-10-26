//! # Syntax
//!
//! **Purpose:** decide which parts of a line are keywords, strings, comments and
//! so on.
//!
//! **Responsibility:** own the token vocabulary ([`HighlightKind`]), the data
//! that describes a language ([`Language`]), and the registry that maps a file
//! to one. The actual scanning lives in [`engine`]; the languages themselves are
//! Lua scripts, read by [`script`] — the built-in ones from [`languages`], and
//! the user's own from `<config-dir>/termi/syntax/`.
//!
//! Nothing about a language is compiled into the engine. A script can describe
//! one as data — word lists, comment markers, extra regex rules — or take over
//! completely with a `highlight` function, and a user script with the same
//! `name` as a built-in one replaces it.
//!
//! **Public API:** [`HighlightKind`], [`Highlight`], [`Language`], [`detect`],
//! [`detect_document`], [`by_name`], [`names`], [`load_errors`].

pub mod cache;
pub mod engine;
pub mod languages;
pub mod script;

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

pub use cache::HighlightCache;
pub use engine::{BlockState, Highlighter};

/// A class of token, matching one slot in [`crate::theme::SyntaxStyles`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HighlightKind {
    /// Language keyword.
    Keyword,
    /// Type name or primitive.
    Type,
    /// Function or method name.
    Function,
    /// String or character literal.
    String,
    /// Numeric literal.
    Number,
    /// Comment of any kind.
    Comment,
    /// Named constant, `true`/`null`, or a `SCREAMING_CASE` identifier.
    Constant,
    /// Operator.
    Operator,
    /// Bracket, comma or semicolon.
    Punctuation,
    /// Attribute, decorator or preprocessor directive.
    Attribute,
    /// Macro invocation.
    Macro,
    /// Markdown heading.
    Heading,
    /// Markdown bold or italic.
    Emphasis,
    /// Markdown link or bare URL.
    Link,
    /// A line a diff adds.
    DiffAdded,
    /// A line a diff removes.
    DiffRemoved,
    /// A diff hunk header.
    DiffHunk,
    /// A diff file header or other metadata line.
    DiffMeta,
}

impl HighlightKind {
    /// Every kind, with the name scripts and themes use for it.
    pub const NAMES: [(&'static str, Self); 18] = [
        ("keyword", Self::Keyword),
        ("type", Self::Type),
        ("function", Self::Function),
        ("string", Self::String),
        ("number", Self::Number),
        ("comment", Self::Comment),
        ("constant", Self::Constant),
        ("operator", Self::Operator),
        ("punctuation", Self::Punctuation),
        ("attribute", Self::Attribute),
        ("macro", Self::Macro),
        ("heading", Self::Heading),
        ("emphasis", Self::Emphasis),
        ("link", Self::Link),
        ("diff_added", Self::DiffAdded),
        ("diff_removed", Self::DiffRemoved),
        ("diff_hunk", Self::DiffHunk),
        ("diff_meta", Self::DiffMeta),
    ];

    /// The kind a script names.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Self::NAMES
            .iter()
            .find(|(known, _)| known.eq_ignore_ascii_case(name))
            .map(|(_, kind)| *kind)
    }
}

/// A styled span within one line, in character offsets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Highlight {
    /// First character of the span.
    pub start: usize,
    /// One past the last character.
    pub end: usize,
    /// How to style it.
    pub kind: HighlightKind,
}

/// Everything the engine needs to know about a language.
#[derive(Debug, Clone, Default)]
pub struct Language {
    /// Name shown in the status bar and used by `:set syntax`.
    pub name: String,
    /// File extensions, lower case and without the dot.
    pub extensions: Vec<String>,
    /// Bare file names, lower case, that identify the language regardless of
    /// extension.
    pub filenames: Vec<String>,
    /// Interpreters that, named on a `#!` line, identify the language.
    pub shebangs: Vec<String>,
    /// Reserved words.
    pub keywords: HashSet<String>,
    /// Built-in and standard type names.
    pub types: HashSet<String>,
    /// Literal constants such as `true` or `None`.
    pub constants: HashSet<String>,
    /// Token that starts a comment running to end of line.
    pub line_comment: Option<String>,
    /// Opening and closing tokens of a block comment.
    pub block_comment: Option<(String, String)>,
    /// Whether block comments may nest, as they do in Rust.
    pub nested_block_comments: bool,
    /// Whether `name!` is a macro invocation.
    pub macro_suffix: bool,
    /// Whether an initial capital letter suggests a type name.
    pub capitalised_types: bool,
    /// Extra patterns tried *before* the generic ones, in order. Sub-groups
    /// must be non-capturing.
    pub extra_rules: Vec<(String, HighlightKind)>,
    /// Suppress the generic identifier, operator and punctuation rules, for
    /// prose and for formats that only want their own rules.
    pub prose: bool,
    /// Where the definition came from: a built-in script or a user file.
    pub origin: String,
}

/// Every language the editor knows, and what went wrong loading the rest.
struct Registry {
    highlighters: Vec<Highlighter>,
    errors: Vec<String>,
}

/// The user's own language scripts.
///
/// Tests never look there: a definition on the developer's machine must not
/// change what the tests see.
fn user_dir() -> Option<PathBuf> {
    (!cfg!(test)).then(|| crate::config::config_dir().join("syntax"))
}

impl Registry {
    fn load() -> Self {
        Self::load_from(user_dir().as_deref())
    }

    fn load_from(directory: Option<&Path>) -> Self {
        let mut errors = Vec::new();
        let mut user = Vec::new();
        if let Some(directory) = directory
            && let Ok(entries) = std::fs::read_dir(directory)
        {
            let mut paths: Vec<PathBuf> = entries
                .flatten()
                .map(|entry| entry.path())
                .filter(|path| path.extension().is_some_and(|ext| ext == "lua"))
                .collect();
            paths.sort();
            for path in paths {
                let origin = path.display().to_string();
                let compiled = std::fs::read_to_string(&path)
                    .map_err(|error| format!("{origin}: {error}"))
                    .and_then(|source| script::load(&source, &origin))
                    .and_then(Highlighter::new);
                match compiled {
                    Ok(highlighter) => user.push(highlighter),
                    Err(error) => errors.push(error),
                }
            }
        }

        // The user's definitions are searched first, so their extensions win,
        // and one named like a built-in replaces it outright.
        let mut highlighters = user;
        for (file, source) in languages::BUILTIN {
            let origin = format!("built-in {file}");
            match script::load(source, &origin).and_then(Highlighter::new) {
                Ok(highlighter)
                    if !highlighters
                        .iter()
                        .any(|h| h.language.name == highlighter.language.name) =>
                {
                    highlighters.push(highlighter);
                }
                Ok(_) => {}
                Err(error) => errors.push(error),
            }
        }
        Self {
            highlighters,
            errors,
        }
    }
}

fn registry() -> &'static Registry {
    static REGISTRY: OnceLock<Registry> = OnceLock::new();
    REGISTRY.get_or_init(Registry::load)
}

/// Problems found while loading language scripts, for the status bar.
#[must_use]
pub fn load_errors() -> &'static [String] {
    &registry().errors
}

/// The name of every language, for completion and `:set syntax`.
#[must_use]
pub fn names() -> Vec<&'static str> {
    registry()
        .highlighters
        .iter()
        .map(|h| h.language.name.as_str())
        .collect()
}

/// Pick a highlighter for a path, by file name first and extension second.
#[must_use]
pub fn detect(path: &Path) -> Option<&'static Highlighter> {
    let highlighters = &registry().highlighters;
    let name = path.file_name()?.to_str()?.to_ascii_lowercase();
    if let Some(found) = highlighters
        .iter()
        .find(|h| h.language.filenames.contains(&name))
    {
        return Some(found);
    }
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    highlighters
        .iter()
        .find(|h| h.language.extensions.contains(&extension))
}

/// Pick a highlighter for a document: by its path, and failing that by the
/// interpreter its first line names — a script called `deploy` that starts
/// with `#!/usr/bin/env bash` is shell.
#[must_use]
pub fn detect_document(path: Option<&Path>, first_line: &str) -> Option<&'static Highlighter> {
    path.and_then(detect).or_else(|| by_shebang(first_line))
}

fn by_shebang(first_line: &str) -> Option<&'static Highlighter> {
    let command = first_line.strip_prefix("#!")?.trim();
    let mut words = command.split_whitespace();
    let mut program = words.next()?.rsplit('/').next()?;
    // `#!/usr/bin/env -S python3 -u` names the interpreter after env's flags.
    if program == "env" {
        program = words.find(|word| !word.starts_with('-'))?;
    }
    let bare = program.trim_end_matches(|ch: char| ch.is_ascii_digit() || ch == '.');
    registry().highlighters.iter().find(|h| {
        h.language
            .shebangs
            .iter()
            .any(|known| known == program || known == bare)
    })
}

/// Look a highlighter up by language name, for `:set syntax`.
#[must_use]
pub fn by_name(name: &str) -> Option<&'static Highlighter> {
    registry()
        .highlighters
        .iter()
        .find(|h| h.language.name.eq_ignore_ascii_case(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_built_in_script_loads() {
        assert!(load_errors().is_empty(), "{:?}", load_errors());
        assert_eq!(names().len(), languages::BUILTIN.len());
    }

    #[test]
    fn a_user_script_replaces_a_built_in_and_adds_its_own() {
        let directory = std::env::temp_dir().join("termi-user-syntax");
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).expect("create directory");
        std::fs::write(
            directory.join("rust.lua"),
            r#"return { name = "rust", extensions = { "rs" }, keywords = words "fn" }"#,
        )
        .expect("write override");
        std::fs::write(
            directory.join("ini.lua"),
            r##"return { name = "ini", extensions = { "ini" }, line_comment = ";",
                rules = { { [=[^\s*\[[^\]]*\]]=], "heading" } } }"##,
        )
        .expect("write new language");
        std::fs::write(directory.join("broken.lua"), "return {").expect("write broken");

        let registry = Registry::load_from(Some(&directory));
        let rust: Vec<_> = registry
            .highlighters
            .iter()
            .filter(|h| h.language.name == "rust")
            .collect();
        assert_eq!(rust.len(), 1);
        assert!(
            !rust[0].language.keywords.contains("let"),
            "the user's rust wins"
        );
        assert!(
            registry
                .highlighters
                .iter()
                .any(|h| h.language.name == "ini")
        );
        assert_eq!(registry.errors.len(), 1);
        assert!(registry.errors[0].contains("broken.lua"));
    }

    #[test]
    fn files_are_matched_by_name_then_extension() {
        assert_eq!(
            detect(Path::new("src/main.rs")).map(|h| h.language.name.as_str()),
            Some("rust")
        );
        assert_eq!(
            detect(Path::new("Makefile")).map(|h| h.language.name.as_str()),
            Some("makefile")
        );
        assert_eq!(
            detect(Path::new("Cargo.lock")).map(|h| h.language.name.as_str()),
            Some("toml")
        );
        assert!(detect(Path::new("notes.unknown")).is_none());
    }

    #[test]
    fn a_shebang_names_the_language_of_an_extensionless_script() {
        let found = |line: &str| {
            detect_document(Some(Path::new("deploy")), line).map(|h| h.language.name.as_str())
        };
        assert_eq!(found("#!/bin/bash"), Some("shell"));
        assert_eq!(found("#!/usr/bin/env python3.12"), Some("python"));
        assert_eq!(
            found("#!/usr/bin/env -S node --experimental"),
            Some("javascript")
        );
        assert_eq!(found("plain text"), None);
    }
}
