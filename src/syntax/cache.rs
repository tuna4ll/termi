//! # Highlight cache
//!
//! **Purpose:** make highlighting a scrolled-to line cheap.
//!
//! **Responsibility:** remember the [`BlockState`] each line *starts* in. Whether
//! line 40 000 is inside a comment depends on every line above it, so without a
//! cache, drawing one screen deep in a file would rescan the whole file — every
//! frame.
//!
//! The cache is a prefix: entries `0..len` are known good, and an edit on line
//! `n` truncates it to `n + 1`, because a change on a line cannot affect the
//! state any line above it starts in. Scrolling forward extends it; scrolling
//! back costs nothing.
//!
//! **Public API:** [`HighlightCache`].

use super::{BlockState, Highlight, Highlighter};
use crate::editor::document::Document;

/// Per-buffer syntax state.
#[derive(Default)]
pub struct HighlightCache {
    /// Language in use, or `None` for plain text.
    highlighter: Option<&'static Highlighter>,
    /// `states[i]` is the block state line `i` begins in.
    states: Vec<BlockState>,
}

impl std::fmt::Debug for HighlightCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HighlightCache")
            .field("language", &self.language_name())
            .field("cached_lines", &self.states.len())
            .finish()
    }
}

impl HighlightCache {
    /// A cache for `highlighter`, or for plain text when it is `None`.
    #[must_use]
    pub fn new(highlighter: Option<&'static Highlighter>) -> Self {
        Self {
            highlighter,
            states: Vec::new(),
        }
    }

    /// Switch languages, discarding everything computed for the old one.
    pub fn set_language(&mut self, highlighter: Option<&'static Highlighter>) {
        self.highlighter = highlighter;
        self.states.clear();
    }

    /// The language in use, if any.
    #[must_use]
    pub fn language(&self) -> Option<&'static super::Language> {
        self.highlighter.map(|h| &h.language)
    }

    /// Language name for the status bar.
    #[must_use]
    pub fn language_name(&self) -> &'static str {
        self.highlighter
            .map_or("plain", |h| h.language.name.as_str())
    }

    /// Forget the state of every line after `line`.
    pub fn invalidate_from(&mut self, line: usize) {
        // The state line `line` *begins* in cannot be affected by an edit on
        // that same line, so it stays.
        self.states.truncate(line + 1);
    }

    /// Compute states up to and including `upto`.
    ///
    /// Must be called before [`Self::highlight`] for that line. It is a separate
    /// step because rendering borrows the buffer immutably, and extending the
    /// cache is the one part of highlighting that mutates.
    pub fn ensure(&mut self, document: &Document, upto: usize) {
        let Some(highlighter) = self.highlighter else {
            return;
        };
        // A shrunk document can leave entries describing lines that no longer
        // exist.
        self.states.truncate(document.len_lines());
        if self.states.is_empty() {
            self.states.push(BlockState::Normal);
        }

        let target = upto.min(document.last_line());
        while self.states.len() <= target {
            let line = self.states.len() - 1;
            let state = self.states[line];
            let (_, next) = highlighter.highlight_line(&document.line_string(line), state);
            self.states.push(next);
        }
    }

    /// Spans for one line.
    ///
    /// Returns nothing when the line's state has not been computed yet, which
    /// draws it unstyled for one frame rather than blocking on a rescan.
    #[must_use]
    pub fn highlight(&self, document: &Document, line: usize) -> Vec<Highlight> {
        let Some(highlighter) = self.highlighter else {
            return Vec::new();
        };
        let Some(state) = self.states.get(line).copied() else {
            return Vec::new();
        };
        highlighter
            .highlight_line(&document.line_string(line), state)
            .0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::syntax::by_name;

    fn document(text: &str) -> Document {
        Document::from_text(text, None)
    }

    #[test]
    fn plain_text_produces_no_spans() {
        let mut cache = HighlightCache::new(None);
        let doc = document("anything");
        cache.ensure(&doc, 0);
        assert!(cache.highlight(&doc, 0).is_empty());
        assert_eq!(cache.language_name(), "plain");
    }

    #[test]
    fn block_comment_state_carries_down_the_file() {
        let mut cache = HighlightCache::new(by_name("c"));
        let doc = document("/* open\nstill\nstill\n*/ int x;");
        cache.ensure(&doc, 3);

        // Line 3 closes the comment, so `int` is highlighted as a type there and
        // the lines in between are entirely comment.
        assert_eq!(cache.highlight(&doc, 1).len(), 1);
        assert!(cache.highlight(&doc, 3).len() > 1);
    }

    #[test]
    fn ensure_only_extends_as_far_as_asked() {
        let mut cache = HighlightCache::new(by_name("rust"));
        let doc = document(&"let x = 1;\n".repeat(1000));
        cache.ensure(&doc, 10);
        assert_eq!(cache.states.len(), 11);
    }

    #[test]
    fn an_edit_invalidates_only_the_lines_below_it() {
        let mut cache = HighlightCache::new(by_name("rust"));
        let doc = document(&"let x = 1;\n".repeat(100));
        cache.ensure(&doc, 50);
        cache.invalidate_from(20);
        assert_eq!(cache.states.len(), 21);
    }

    #[test]
    fn a_shrunk_document_drops_stale_entries() {
        let mut cache = HighlightCache::new(by_name("rust"));
        let long = document(&"let x = 1;\n".repeat(50));
        cache.ensure(&long, 40);

        let short = document("let x = 1;\n");
        cache.ensure(&short, 1);
        assert!(cache.states.len() <= short.len_lines());
    }
}
