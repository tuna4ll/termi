//! # Document
//!
//! **Purpose:** hold one file's text and the metadata that belongs to the text
//! rather than to the view of it.
//!
//! **Responsibility:** storage (a [`Rope`]), the originating path, the line
//! ending style, and the dirty flag. A `Document` never knows where the cursor
//! is or how it is displayed — that is [`crate::editor::buffer`]'s job.
//!
//! Line endings are normalised to `\n` on load and restored on save. Doing it
//! at the edges means every offset in the editor is a plain character index
//! with no invisible `\r` to account for.
//!
//! **Public API:** [`Document`], [`LineEnding`], and the [`indent`] and
//! [`pairs`] helpers.

pub mod indent;
pub mod pairs;

use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
use ropey::{Rope, RopeSlice};

use crate::editor::cursor::Position;
use crate::filesystem;

/// The line terminator a file uses on disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LineEnding {
    /// Unix `\n`.
    #[default]
    Lf,
    /// Windows `\r\n`.
    Crlf,
}

impl LineEnding {
    /// Guess the line ending from the first terminator in `text`.
    ///
    /// Mixed files are common; the first one wins and the file is normalised to
    /// it on save.
    #[must_use]
    pub fn detect(text: &str) -> Self {
        match text.find('\n') {
            Some(idx) if idx > 0 && text.as_bytes()[idx - 1] == b'\r' => Self::Crlf,
            _ => Self::Lf,
        }
    }

    /// Short label for the status bar.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Lf => "LF",
            Self::Crlf => "CRLF",
        }
    }
}

/// A single open file's text.
#[derive(Debug)]
pub struct Document {
    text: Rope,
    path: Option<PathBuf>,
    line_ending: LineEnding,
    dirty: bool,
}

impl Default for Document {
    fn default() -> Self {
        Self::new()
    }
}

impl Document {
    /// An empty, unnamed document.
    #[must_use]
    pub fn new() -> Self {
        Self {
            text: Rope::new(),
            path: None,
            line_ending: LineEnding::default(),
            dirty: false,
        }
    }

    /// Build a document from in-memory text, normalising line endings.
    #[must_use]
    pub fn from_text(text: &str, path: Option<PathBuf>) -> Self {
        let line_ending = LineEnding::detect(text);
        let normalised = if line_ending == LineEnding::Crlf {
            Rope::from_str(&text.replace("\r\n", "\n"))
        } else {
            Rope::from_str(text)
        };
        Self {
            text: normalised,
            path,
            line_ending,
            dirty: false,
        }
    }

    /// The underlying rope, for read-only traversal.
    #[must_use]
    pub const fn text(&self) -> &Rope {
        &self.text
    }

    /// The file this document came from, if any.
    #[must_use]
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// The line ending this document will be written with.
    #[must_use]
    pub const fn line_ending(&self) -> LineEnding {
        self.line_ending
    }

    /// File name for the tab strip and status bar.
    #[must_use]
    pub fn display_name(&self) -> &str {
        self.path
            .as_deref()
            .and_then(Path::file_name)
            .and_then(std::ffi::OsStr::to_str)
            .unwrap_or("[No Name]")
    }
}

/// Disk I/O.
impl Document {
    /// Open `path`, or start an empty document bound to it if it does not exist
    /// yet — `termi new_file.rs` should not be an error.
    ///
    /// # Errors
    /// Returns an error if the file exists but cannot be read or is not UTF-8.
    pub fn open(path: PathBuf) -> Result<Self> {
        if !path.exists() {
            return Ok(Self {
                path: Some(path),
                ..Self::new()
            });
        }
        if path.is_dir() {
            bail!("{} is a directory", path.display());
        }
        let text = filesystem::read_file(&path)?;
        Ok(Self::from_text(&text, Some(path)))
    }

    /// Write the document back to its own path.
    ///
    /// # Errors
    /// Returns an error if the document has no path or the write fails.
    pub fn save(&mut self) -> Result<()> {
        let Some(path) = self.path.clone() else {
            bail!("no file name — use :w <path>");
        };
        filesystem::write_file(&path, &self.to_disk_text())?;
        self.dirty = false;
        Ok(())
    }

    /// Write the document to `path` and adopt it as the document's own.
    ///
    /// # Errors
    /// Returns an error if the write fails; the path is only adopted on success.
    pub fn save_as(&mut self, path: PathBuf) -> Result<()> {
        filesystem::write_file(&path, &self.to_disk_text())?;
        self.path = Some(path);
        self.dirty = false;
        Ok(())
    }

    /// Reload from disk, discarding unsaved changes.
    ///
    /// # Errors
    /// Returns an error if the document has no path or cannot be read.
    pub fn reload(&mut self) -> Result<()> {
        let Some(path) = self.path.clone() else {
            bail!("nothing to reload");
        };
        let text = filesystem::read_file(&path)?;
        *self = Self::from_text(&text, Some(path));
        Ok(())
    }

    /// Whether `text` is exactly what saving this document would produce.
    ///
    /// Used to tell a real external edit from the editor seeing its own write:
    /// comparing content is reliable where comparing timestamps is not, and it
    /// accounts for the line endings the file will be written with.
    #[must_use]
    pub fn matches_disk_text(&self, text: &str) -> bool {
        self.to_disk_text() == text
    }

    /// Serialise the rope with this document's line ending.
    ///
    /// This materialises the whole document; saving is not a hot path, and doing
    /// it in one allocation keeps [`filesystem::write_file`]'s atomicity simple.
    fn to_disk_text(&self) -> String {
        let text = self.text.to_string();
        match self.line_ending {
            LineEnding::Lf => text,
            LineEnding::Crlf => text.replace('\n', "\r\n"),
        }
    }
}

/// Read-only text access.
///
/// Every method here clamps its arguments instead of panicking. Cursors,
/// viewports and search results all index the rope, and a document can shrink
/// underneath any of them (undo, external reload); clamping turns a whole class
/// of races into a harmless off-by-a-few instead of a crash.
impl Document {
    /// Total number of characters, excluding nothing.
    #[must_use]
    pub fn len_chars(&self) -> usize {
        self.text.len_chars()
    }

    /// Number of lines. A trailing newline yields a final empty line, matching
    /// what the user sees.
    #[must_use]
    pub fn len_lines(&self) -> usize {
        self.text.len_lines()
    }

    /// Index of the last line.
    #[must_use]
    pub fn last_line(&self) -> usize {
        self.text.len_lines() - 1
    }

    /// A line including its terminator, clamped to the document.
    #[must_use]
    pub fn line(&self, line: usize) -> RopeSlice<'_> {
        self.text.line(line.min(self.last_line()))
    }

    /// Length of a line in characters, *excluding* the line terminator.
    #[must_use]
    pub fn line_len(&self, line: usize) -> usize {
        line_content_len(self.line(line))
    }

    /// Character index of the first character on `line`.
    #[must_use]
    pub fn line_start(&self, line: usize) -> usize {
        self.text.line_to_char(line.min(self.last_line()))
    }

    /// Convert a position into a rope character index.
    #[must_use]
    pub fn pos_to_char(&self, pos: Position) -> usize {
        let line = pos.line.min(self.last_line());
        self.text.line_to_char(line) + pos.col.min(self.line_len(line))
    }

    /// Convert a rope character index back into a position.
    #[must_use]
    pub fn char_to_pos(&self, index: usize) -> Position {
        let index = index.min(self.text.len_chars());
        let line = self.text.char_to_line(index);
        Position::new(line, index - self.text.line_to_char(line))
    }

    /// Pull a position back inside the document.
    ///
    /// `allow_eol` is `true` in insert mode, where the cursor legitimately sits
    /// one past the last character, and `false` in normal mode, where the cursor
    /// always covers a real character.
    #[must_use]
    pub fn clamp(&self, pos: Position, allow_eol: bool) -> Position {
        let line = pos.line.min(self.last_line());
        let len = self.line_len(line);
        let max_col = if allow_eol {
            len
        } else {
            len.saturating_sub(1)
        };
        Position::new(line, pos.col.min(max_col))
    }

    /// Copy a line into a `String`, without its terminator.
    #[must_use]
    pub fn line_string(&self, line: usize) -> String {
        let slice = self.line(line);
        slice.slice(..line_content_len(slice)).to_string()
    }
}

/// Mutation.
///
/// These are the *only* two ways text changes. Every higher-level edit (typing,
/// deleting a selection, replacing a match, undoing) is expressed as a sequence
/// of `insert` and `remove`, which is what lets [`crate::undo`] record a single
/// kind of event and replay it backwards.
impl Document {
    /// Insert `text` at a character index.
    ///
    /// The index is clamped, and `\r\n` in the incoming text is normalised —
    /// pasted content routinely carries foreign line endings.
    pub fn insert(&mut self, char_index: usize, text: &str) {
        if text.is_empty() {
            return;
        }
        let index = char_index.min(self.text.len_chars());
        if text.contains('\r') {
            self.text
                .insert(index, &text.replace("\r\n", "\n").replace('\r', "\n"));
        } else {
            self.text.insert(index, text);
        }
        self.dirty = true;
    }

    /// Remove `[start, end)` and return what was removed, for the undo stack.
    pub fn remove(&mut self, start: usize, end: usize) -> String {
        let len = self.text.len_chars();
        let (start, end) = (start.min(len), end.min(len));
        if start >= end {
            return String::new();
        }
        let removed = self.text.slice(start..end).to_string();
        self.text.remove(start..end);
        self.dirty = true;
        removed
    }

    /// Copy an arbitrary character range into a `String`.
    #[must_use]
    pub fn slice_string(&self, start: usize, end: usize) -> String {
        let len = self.text.len_chars();
        let (start, end) = (start.min(len), end.min(len));
        if start >= end {
            return String::new();
        }
        self.text.slice(start..end).to_string()
    }

    /// Whether there are unsaved changes.
    #[must_use]
    pub const fn is_dirty(&self) -> bool {
        self.dirty
    }
}

/// Length of a line slice with any `\r\n` / `\n` terminator removed.
fn line_content_len(slice: RopeSlice<'_>) -> usize {
    let mut len = slice.len_chars();
    if len > 0 && slice.char(len - 1) == '\n' {
        len -= 1;
    }
    if len > 0 && slice.char(len - 1) == '\r' {
        len -= 1;
    }
    len
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(text: &str) -> Document {
        Document::from_text(text, None)
    }

    #[test]
    fn detects_and_strips_crlf() {
        let document = doc("a\r\nb\r\n");
        assert_eq!(document.line_ending(), LineEnding::Crlf);
        assert_eq!(document.line_len(0), 1);
        assert_eq!(document.to_disk_text(), "a\r\nb\r\n");
    }

    #[test]
    fn line_length_excludes_terminator() {
        let document = doc("hello\nworld");
        assert_eq!(document.line_len(0), 5);
        assert_eq!(document.line_len(1), 5);
        assert_eq!(document.len_lines(), 2);
    }

    #[test]
    fn positions_round_trip_through_char_indices() {
        let document = doc("ağaç\nşeker");
        let pos = Position::new(1, 3);
        assert_eq!(document.char_to_pos(document.pos_to_char(pos)), pos);
    }

    #[test]
    fn clamping_respects_end_of_line_rules() {
        let document = doc("abc\n");
        assert_eq!(document.clamp(Position::new(0, 99), true).col, 3);
        assert_eq!(document.clamp(Position::new(0, 99), false).col, 2);
        assert_eq!(document.clamp(Position::new(99, 0), true).line, 1);
    }

    #[test]
    fn clamping_an_empty_line_stays_at_zero() {
        let document = doc("\n");
        assert_eq!(document.clamp(Position::new(0, 5), false), Position::ZERO);
    }

    #[test]
    fn edits_flip_the_dirty_flag() {
        let mut document = doc("abc");
        assert!(!document.is_dirty());
        document.insert(1, "X");
        assert!(document.is_dirty());
        assert_eq!(document.text().to_string(), "aXbc");
        assert_eq!(document.remove(1, 2), "X");
        assert_eq!(document.text().to_string(), "abc");
    }

    #[test]
    fn insert_normalises_pasted_line_endings() {
        let mut document = doc("");
        document.insert(0, "a\r\nb\rc");
        assert_eq!(document.text().to_string(), "a\nb\nc");
    }

    #[test]
    fn out_of_range_removal_is_a_no_op() {
        let mut document = doc("abc");
        assert_eq!(document.remove(10, 20), "");
        assert_eq!(document.text().to_string(), "abc");
    }
}
