//! # Theme
//!
//! **Purpose:** decide what every part of the screen looks like.
//!
//! **Responsibility:** map named UI and syntax slots onto concrete
//! [`ratatui::style::Style`] values. Widgets ask the theme for a slot and never
//! mention a colour themselves, so a new theme is a data change rather than a
//! rendering change.
//!
//! Slots are struct fields rather than map keys: highlighting looks up a style
//! per token, and a field access keeps that on the fast path.
//!
//! **Public API:** [`Theme`], [`SyntaxStyles`].

pub mod builtin;
pub mod custom;

use std::path::Path;

use ratatui::style::Style;

use crate::syntax::HighlightKind;

/// Styles for the chrome around and behind the text.
#[derive(Debug, Clone)]
pub struct Theme {
    /// Human readable name, shown by `:theme`.
    pub name: String,
    /// Default text and the editor background.
    pub text: Style,
    /// Line numbers on inactive lines.
    pub gutter: Style,
    /// Line number on the cursor's line.
    pub gutter_active: Style,
    /// Full-width highlight behind the cursor's line.
    pub cursor_line: Style,
    /// Selected text.
    pub selection: Style,
    /// Status bar background.
    pub status: Style,
    /// Mode badge inside the status bar.
    pub status_mode: Style,
    /// Marker shown for unsaved buffers.
    pub status_dirty: Style,
    /// Command bar, including the leading `:`.
    pub command: Style,
    /// Error text in the command bar.
    pub command_error: Style,
    /// The focused tab.
    pub tab_active: Style,
    /// Unfocused tabs.
    pub tab_inactive: Style,
    /// Popup body.
    pub popup: Style,
    /// Popup border.
    pub popup_border: Style,
    /// Directories in the file tree.
    pub tree_directory: Style,
    /// Files in the file tree.
    pub tree_file: Style,
    /// Every search match.
    pub search: Style,
    /// The match the cursor is currently on.
    pub search_active: Style,
    /// Token styles.
    pub syntax: SyntaxStyles,
}

/// Styles for syntax tokens.
///
/// `Copy` because highlighting hands these around per token; cloning a `String`
/// there would be measurable on large files.
#[derive(Debug, Clone, Copy)]
pub struct SyntaxStyles {
    /// `fn`, `if`, `return`, …
    pub keyword: Style,
    /// Type names and primitives.
    pub type_name: Style,
    /// Function and method names at definition or call sites.
    pub function: Style,
    /// String and character literals.
    pub string: Style,
    /// Numeric literals.
    pub number: Style,
    /// Line and block comments.
    pub comment: Style,
    /// `true`, `null`, `SCREAMING_CASE` constants.
    pub constant: Style,
    /// Operators such as `+`, `=>`, `::`.
    pub operator: Style,
    /// Brackets, commas, semicolons.
    pub punctuation: Style,
    /// Attributes, decorators and pragmas.
    pub attribute: Style,
    /// Macro invocations.
    pub macro_call: Style,
    /// Markdown headings.
    pub heading: Style,
    /// Markdown bold and italic.
    pub emphasis: Style,
    /// Markdown links and URLs.
    pub link: Style,
    /// Lines a diff adds.
    pub diff_added: Style,
    /// Lines a diff removes.
    pub diff_removed: Style,
    /// Diff hunk headers.
    pub diff_hunk: Style,
    /// Diff file headers and other metadata lines.
    pub diff_meta: Style,
}

impl SyntaxStyles {
    /// The style for one token class.
    ///
    /// A `match` rather than a map: this runs once per styled character, and the
    /// exhaustive match also means adding a [`HighlightKind`] cannot silently
    /// fall back to the default colour.
    #[must_use]
    pub const fn style_for(&self, kind: HighlightKind) -> Style {
        match kind {
            HighlightKind::Keyword => self.keyword,
            HighlightKind::Type => self.type_name,
            HighlightKind::Function => self.function,
            HighlightKind::String => self.string,
            HighlightKind::Number => self.number,
            HighlightKind::Comment => self.comment,
            HighlightKind::Constant => self.constant,
            HighlightKind::Operator => self.operator,
            HighlightKind::Punctuation => self.punctuation,
            HighlightKind::Attribute => self.attribute,
            HighlightKind::Macro => self.macro_call,
            HighlightKind::Heading => self.heading,
            HighlightKind::Emphasis => self.emphasis,
            HighlightKind::Link => self.link,
            HighlightKind::DiffAdded => self.diff_added,
            HighlightKind::DiffRemoved => self.diff_removed,
            HighlightKind::DiffHunk => self.diff_hunk,
            HighlightKind::DiffMeta => self.diff_meta,
        }
    }
}

impl Default for Theme {
    fn default() -> Self {
        builtin::dark()
    }
}

impl Theme {
    /// Look a built-in theme up by name, falling back to dark.
    ///
    /// Unknown names deliberately do not error: a typo in a config file should
    /// still leave the user with a usable editor.
    #[must_use]
    pub fn builtin(name: &str) -> Self {
        match name.trim().to_ascii_lowercase().as_str() {
            "light" => builtin::light(),
            _ => builtin::dark(),
        }
    }

    /// Resolve a theme name, preferring `<themes_dir>/<name>.toml` over the
    /// built-ins so a user can shadow `dark` with their own version.
    ///
    /// A missing or malformed file falls back to the built-in of the same name;
    /// the caller gets the parse error back so it can be shown in the status bar
    /// without the editor refusing to start.
    #[must_use]
    pub fn load(name: &str, themes_dir: &Path) -> (Self, Option<String>) {
        let path = themes_dir.join(format!("{name}.toml"));
        if !path.is_file() {
            return (Self::builtin(name), None);
        }
        match crate::filesystem::read_file(&path)
            .and_then(|text| Ok(toml::from_str::<custom::CustomTheme>(&text)?))
        {
            Ok(spec) => (spec.build(name), None),
            Err(error) => (Self::builtin(name), Some(format!("{path:?}: {error}"))),
        }
    }
}
