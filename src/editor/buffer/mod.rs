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

    /// Mark every line after `line` as needing its syntax state recomputed.
    pub fn invalidate_syntax_from(&mut self, line: usize) {
        self.syntax.invalidate_from(line);
    }
}
