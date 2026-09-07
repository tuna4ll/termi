//! # Syntax
//!
//! **Purpose:** decide which parts of a line are keywords, strings, comments and
//! so on.
//!
//! **Responsibility:** own the token vocabulary ([`HighlightKind`]), the data
//! that describes a language ([`Language`]), and the registry that maps a file
//! to one. The actual scanning lives in [`engine`]; the languages themselves are
//! data in [`languages`].
//!
//! The engine is regex based rather than a real parser. That is a deliberate
//! first step: it is exact enough for keywords, literals and comments, costs
//! nothing to add a language to, and — because everything above it only ever
//! sees [`Highlight`] spans — can be replaced by tree-sitter later without any
//! change to the renderer or the theme.
//!
//! **Public API:** [`HighlightKind`], [`Highlight`], [`Language`], [`detect`],
//! [`by_name`].

pub mod cache;
pub mod engine;
pub mod languages;

use std::path::Path;
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
///
/// All of it is `&'static` data, so a language definition is a constant and the
/// registry costs one regex compile per language, once.
pub struct Language {
    /// Name shown in the status bar.
    pub name: &'static str,
    /// File extensions, lower case and without the dot.
    pub extensions: &'static [&'static str],
    /// Bare file names that identify the language regardless of extension.
    pub filenames: &'static [&'static str],
    /// Reserved words.
    pub keywords: &'static [&'static str],
    /// Built-in and standard type names.
    pub types: &'static [&'static str],
    /// Literal constants such as `true` or `None`.
    pub constants: &'static [&'static str],
    /// Token that starts a comment running to end of line.
    pub line_comment: Option<&'static str>,
    /// Opening and closing tokens of a block comment.
    pub block_comment: Option<(&'static str, &'static str)>,
    /// Whether block comments may nest, as they do in Rust and Zig.
    pub nested_block_comments: bool,
    /// Whether `name!` is a macro invocation.
    pub macro_suffix: bool,
    /// Whether an initial capital letter suggests a type name.
    ///
    /// True for languages that conventionally use `PascalCase` types, false for
    /// C where a capitalised identifier is usually a macro or a constant.
    pub capitalised_types: bool,
    /// Extra patterns tried *before* the generic ones, in order.
    ///
    /// Used for anything the generic scanner would get wrong: Rust lifetimes,
    /// Python decorators, C preprocessor lines, Markdown headings. Sub-groups
    /// must be non-capturing.
    pub extra_rules: &'static [(&'static str, HighlightKind)],
    /// Suppress the generic identifier, operator and punctuation rules.
    ///
    /// Prose languages such as Markdown want only their own rules; running the
    /// code-oriented ones over prose produces noise.
    pub prose: bool,
}

/// Every language the editor knows, compiled on first use.
fn registry() -> &'static [Highlighter] {
    static REGISTRY: OnceLock<Vec<Highlighter>> = OnceLock::new();
    REGISTRY.get_or_init(|| languages::all().iter().map(Highlighter::new).collect())
}

/// Pick a highlighter for a path, by file name first and extension second.
#[must_use]
pub fn detect(path: &Path) -> Option<&'static Highlighter> {
    let name = path.file_name()?.to_str()?.to_ascii_lowercase();
    if let Some(found) = registry()
        .iter()
        .find(|h| h.language.filenames.contains(&name.as_str()))
    {
        return Some(found);
    }
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    registry()
        .iter()
        .find(|h| h.language.extensions.contains(&extension.as_str()))
}

/// Look a highlighter up by language name, for `:set syntax`.
#[must_use]
pub fn by_name(name: &str) -> Option<&'static Highlighter> {
    registry()
        .iter()
        .find(|h| h.language.name.eq_ignore_ascii_case(name))
}
