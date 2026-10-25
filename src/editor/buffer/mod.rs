//! # Buffer
//!
//! **Purpose:** a file being edited — its text and everything derived from it.
//!
//! **Responsibility:** owns one [`Document`], the [`History`] that rewinds it
//! and the syntax state computed from it. Everything here is a property of the
//! *file* rather than of anyone looking at it, which is the line that makes
//! split windows possible: two windows showing the same buffer share one text,
//! one undo history and one set of highlights, while each keeps its own cursors
//! and scroll position in a [`Window`](crate::editor::window::Window).
//!
//! **Public API:** [`Buffer`], [`BufferId`].

use std::collections::HashMap;

use anyhow::Result;

use crate::editor::cursor::Position;
use crate::editor::document::Document;
use crate::syntax::{self, HighlightCache};
use crate::undo::History;

/// Position of a buffer in the editor's buffer list.
///
/// Buffers are only ever appended to and removed from that list through
/// [`App`](crate::app::App), which repairs the window references as it goes, so
/// a plain index is enough of an identifier.
pub type BufferId = usize;

/// A document, the history that rewinds it and the highlighting derived from it.
#[derive(Debug)]
pub struct Buffer {
    /// The text.
    pub document: Document,
    /// Undo and redo stacks for this file only — history is per file, so
    /// switching buffers never mixes two files' edits into one undo step.
    pub history: History,
    /// Detected language and the per-line syntax state derived from it.
    pub syntax: HighlightCache,
    /// Marks set with `m` and the ones the editor keeps itself: `<` and `>`
    /// for the last visual selection, `.` for the last change.
    pub marks: HashMap<char, Position>,
}

impl Buffer {
    /// Wrap a document in a fresh buffer.
    #[must_use]
    pub fn new(document: Document) -> Self {
        let syntax = HighlightCache::new(document.path().and_then(syntax::detect));
        Self {
            document,
            history: History::default(),
            syntax,
            marks: HashMap::new(),
        }
    }

    /// An empty scratch buffer.
    #[must_use]
    pub fn empty() -> Self {
        Self::new(Document::new())
    }

    /// Re-run language detection, after the file has been renamed or reloaded.
    pub fn detect_language(&mut self) {
        self.syntax
            .set_language(self.document.path().and_then(syntax::detect));
    }

    /// Re-read the file from disk, discarding unsaved changes.
    ///
    /// The history goes with them: every recorded change describes offsets in
    /// the text that was just thrown away, and replaying one against the new
    /// text would quietly corrupt it.
    ///
    /// # Errors
    /// Returns an error, leaving the buffer untouched, if the file cannot be read.
    pub fn reload(&mut self) -> Result<()> {
        self.document.reload()?;
        self.history = History::default();
        self.detect_language();
        Ok(())
    }

    /// Mark every line after `line` as needing its syntax state recomputed.
    pub fn invalidate_syntax_from(&mut self, line: usize) {
        self.syntax.invalidate_from(line);
    }
}
