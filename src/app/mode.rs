//! # Editor modes
//!
//! **Purpose:** describe which modal state the editor is in.
//!
//! **Responsibility:** owns the [`Mode`] enum and the small amount of behaviour
//! that is derived purely from the mode (display name, cursor shape, whether a
//! selection is being extended). It deliberately knows nothing about keys,
//! buffers or rendering — those modules ask the mode, the mode never asks them.
//!
//! **Public API:** [`Mode`].

/// The modal state of the editor.
///
/// Modes are a plain `Copy` enum rather than a state machine object: transitions
/// are always driven by input handling, so there is nothing to encapsulate here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    /// Keys are commands; text is navigated and manipulated.
    #[default]
    Normal,
    /// Keys insert text at every cursor.
    Insert,
    /// Character-wise selection is extended by movement.
    Visual,
    /// Line-wise selection is extended by movement.
    VisualLine,
    /// The command bar is focused and accumulating an ex-style command line.
    Command,
    /// The search prompt is focused; the document scrolls as the query is typed.
    Search,
    /// The file tree panel has the keyboard.
    Tree,
    /// A child process running in an embedded terminal has the keyboard.
    Terminal,
}

impl Mode {
    /// Short uppercase label shown in the status bar.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Normal => "NORMAL",
            Self::Insert => "INSERT",
            Self::Visual => "VISUAL",
            Self::VisualLine => "V-LINE",
            Self::Command => "COMMAND",
            Self::Search => "SEARCH",
            Self::Tree => "TREE",
            Self::Terminal => "TERMINAL",
        }
    }

    /// Whether movement in this mode should extend the selection instead of
    /// collapsing it.
    #[must_use]
    pub const fn is_visual(self) -> bool {
        matches!(self, Self::Visual | Self::VisualLine)
    }

    /// Whether printable keys should be inserted as text.
    #[must_use]
    pub const fn is_insert(self) -> bool {
        matches!(self, Self::Insert)
    }

    /// Whether the command bar owns the keyboard.
    #[must_use]
    pub const fn is_command(self) -> bool {
        matches!(self, Self::Command)
    }

    /// Whether the search prompt owns the keyboard.
    #[must_use]
    pub const fn is_search(self) -> bool {
        matches!(self, Self::Search)
    }

    /// In insert mode the cursor sits *between* characters, so it is drawn as a
    /// bar; every other mode selects a character and uses a block.
    #[must_use]
    pub const fn uses_bar_cursor(self) -> bool {
        matches!(self, Self::Insert | Self::Command | Self::Search)
    }
}
