//! # Commands
//!
//! **Purpose:** the vocabulary of the `:` command line.
//!
//! **Responsibility:** define what a command *is*. Turning text into a
//! [`Command`] happens in [`parser`]; carrying one out happens in the
//! application layer. Keeping the enum here means the parser and the executor
//! cannot drift apart — adding a variant breaks both until both are updated.
//!
//! **Public API:** [`Command`].

pub mod parser;

use std::path::PathBuf;

use crate::editor::window::Axis;

/// A parsed command line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// `:w [path]` — write the buffer.
    Write(Option<PathBuf>),
    /// `:q[!]` — close the window, then the buffer, then the editor.
    Quit { force: bool },
    /// `:sp[lit]` / `:vs[plit]` — divide the focused window in two.
    Split { axis: Axis },
    /// `:clo[se]` — close the focused window, keeping the buffer open.
    CloseWindow,
    /// `:on[ly]` — close every window except the focused one.
    OnlyWindow,
    /// `:wq` / `:x` — write and quit.
    WriteQuit { force: bool },
    /// `:e[!] path` — open a file; `force` discards unsaved changes.
    Edit { path: PathBuf, force: bool },
    /// `:e!` with no path — reload the current file from disk.
    Reload,
    /// `:42` — jump to a line.
    GotoLine(usize),
    /// `:set key value` — change a setting for this session.
    Set { key: String, value: String },
    /// `:theme name` — switch colour scheme.
    Theme(String),
    /// `:bn` / `:bp` — switch buffers.
    CycleBuffer { forward: bool },
    /// `:%s/pattern/replacement/[g]` — substitute.
    Substitute {
        /// Search pattern, interpreted according to the search settings.
        pattern: String,
        /// Replacement text.
        replacement: String,
        /// Replace every match on a line rather than the first.
        all: bool,
        /// Apply to the whole document rather than the current line.
        whole_file: bool,
    },
    /// `:help` — list the commands.
    Help,
}
