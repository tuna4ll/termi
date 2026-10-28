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

/// One end of a line range: a base line and an offset from it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Address {
    /// Where the address starts from.
    pub base: Base,
    /// Lines to add (or, negative, subtract) — `.+3`, `$-1`.
    pub offset: isize,
}

/// What an address is counted from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Base {
    /// A line number, one-based as typed.
    Line(usize),
    /// `.` — the caret's line.
    Current,
    /// `$` — the last line.
    Last,
    /// `'a`, `'<`, `'>` — a mark's line.
    Mark(char),
}

/// `:12,20`, `:%`, `:'<,'>` — the lines a command works on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineRange {
    /// The first line.
    pub start: Address,
    /// The last line; the same as `start` for a single address.
    pub end: Address,
}

use std::path::PathBuf;

use crate::editor::window::Axis;

/// A parsed command line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// `:w[!] [path]` — write the buffer; `force` writes a read-only one.
    Write {
        /// Where to write, when not to the buffer's own file.
        path: Option<PathBuf>,
        /// Write even though the buffer is read-only.
        force: bool,
    },
    /// `:sudow` — write the buffer through `sudo`.
    SudoWrite,
    /// `:recover` — bring back the unsaved text a swap file kept.
    Recover,
    /// `:dropswap` — throw away a swap file's unsaved text.
    DropSwap,
    /// `:q[!]` — close the window, then the buffer, then the editor.
    Quit { force: bool },
    /// `:wa` — write every modified buffer that has a name.
    WriteAll,
    /// `:qa[!]` — leave the editor, refusing while anything is unsaved unless
    /// forced.
    QuitAll { force: bool },
    /// `:wqa` / `:xa` — write every modified buffer, then leave.
    WriteQuitAll,
    /// `:sp[lit]` / `:vs[plit]` — divide the focused window in two.
    Split { axis: Axis },
    /// `:clo[se]` — close the focused window, keeping the buffer open.
    CloseWindow,
    /// `:on[ly]` — close every window except the focused one.
    OnlyWindow,
    /// `:wq [path]` / `:x [path]` — write and quit.
    WriteQuit {
        /// Where to write, when not to the buffer's own file.
        path: Option<PathBuf>,
        /// Write a read-only buffer, and quit even with others unsaved.
        force: bool,
    },
    /// `:e[!] path` — open a file; `force` discards unsaved changes.
    Edit { path: PathBuf, force: bool },
    /// `:touch path` — create an empty file without overwriting anything.
    CreateFile(PathBuf),
    /// `:mkdir path` — create one directory.
    CreateDirectory(PathBuf),
    /// Rename the selected tree entry to this path.
    Rename(PathBuf),
    /// Copy the selected tree entry to this path.
    Copy(PathBuf),
    /// Move the selected tree entry to this path.
    Move(PathBuf),
    /// `:e!` with no path — reload the current file from disk.
    Reload,
    /// `:42` — jump to a line.
    GotoLine(usize),
    /// `:$`, `:'a`, `:.+5` — jump to a line given any other way.
    GotoAddress(Address),
    /// `:set key value` — change a setting; `:set!` also saves it to the
    /// config file.
    Set {
        /// The option, as typed.
        key: String,
        /// Its new value.
        value: String,
        /// Write it to the config file too.
        persist: bool,
    },
    /// `:config` — open the config file; `:config reload` re-reads it.
    Config { reload: bool },
    /// `:theme name` — switch colour scheme.
    Theme(String),
    /// `:bn` / `:bp` — switch buffers.
    CycleBuffer { forward: bool },
    /// `:[range]s/pattern/replacement/[flags]` — substitute.
    Substitute {
        /// Search pattern, interpreted according to the search settings.
        pattern: String,
        /// Replacement text; `$1` and `${name}` name capture groups.
        replacement: String,
        /// Replace every match on a line rather than the first — `g`.
        all: bool,
        /// Ask before each replacement — `c`.
        confirm: bool,
        /// `i` or `I`: case-insensitive or -sensitive whatever the setting.
        case_sensitive: Option<bool>,
        /// The lines to work on; the caret's line when none was given.
        range: Option<LineRange>,
    },
    /// `:grep [pattern]` — search the project, live when no pattern is given.
    Grep(Option<String>),
    /// `:greplace/pattern/replacement/[flags]` — substitute in every project
    /// file, leaving the changed files open and unsaved.
    ProjectReplace {
        /// Search pattern.
        pattern: String,
        /// Replacement text.
        replacement: String,
        /// `i` or `I`.
        case_sensitive: Option<bool>,
    },
    /// `:help [topic]` — open the reference, at the topic if one was given.
    HelpTopic(String),
    /// `:earlier [n]` — step back through every state the buffer has had.
    Earlier(usize),
    /// `:later [n]` — step forward again.
    Later(usize),
    /// `:blame` — who last changed the caret's line.
    Blame,
    /// `:help` — open the key and command reference.
    Help,
    /// `:find` — search project files.
    Find,
    /// `:buffers` — search open buffers.
    Buffers,
    /// `:commands` — search commands.
    Commands,
    /// `:themes` — search themes.
    Themes,
}
