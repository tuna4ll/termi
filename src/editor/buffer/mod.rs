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
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::Result;

use crate::config::Config;
use crate::editor::cursor::Position;
use crate::editor::document::Document;
use crate::filesystem::editorconfig::Properties;
use crate::syntax::{self, HighlightCache};
use crate::undo::{History, persist};

/// Position of a buffer in the editor's buffer list.
///
/// Buffers are only ever appended to and removed from that list through
/// [`App`](crate::app::App), which repairs the window references as it goes, so
/// a plain index is enough of an identifier.
pub type BufferId = usize;

/// Settings that belong to one file rather than to the editor: what its
/// `.editorconfig` says, how it turned out to be indented, and whatever
/// `:set` changed while it was open. `None` defers to the global config.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LocalOptions {
    /// Width of a tab stop and of one indentation level.
    pub tab_width: Option<usize>,
    /// Indent with spaces rather than tabs.
    pub expand_tabs: Option<bool>,
    /// Strip trailing whitespace on save.
    pub trim_trailing_whitespace: Option<bool>,
    /// End the file with a line break on save.
    pub insert_final_newline: Option<bool>,
}

impl LocalOptions {
    /// Fill in what `.editorconfig` says, where nothing more specific has.
    pub fn absorb(&mut self, properties: &Properties) {
        if let Some(width) = properties.indent_width() {
            self.tab_width = Some(width);
        }
        if let Some(expand) = properties.expand_tabs {
            self.expand_tabs = Some(expand);
        }
        if let Some(trim) = properties.trim_trailing_whitespace {
            self.trim_trailing_whitespace = Some(trim);
        }
        if let Some(newline) = properties.insert_final_newline {
            self.insert_final_newline = Some(newline);
        }
    }

    /// The global config with these overrides on top.
    #[must_use]
    pub fn apply(&self, global: &Config) -> Config {
        let mut config = global.clone();
        if let Some(width) = self.tab_width {
            config.tab_width = width;
        }
        if let Some(expand) = self.expand_tabs {
            config.expand_tabs = expand;
        }
        if let Some(trim) = self.trim_trailing_whitespace {
            config.trim_trailing_whitespace = trim;
        }
        if let Some(newline) = self.insert_final_newline {
            config.insert_final_newline = newline;
        }
        config
    }
}

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
    /// Settings that apply to this file only.
    pub options: LocalOptions,
    /// Whether the file is big enough that highlighting, swap files and the
    /// like are switched off for it.
    pub large: bool,
    /// The revision last written to the swap file.
    pub swap_revision: Option<u64>,
    /// When the swap file was last written.
    pub swap_written: Option<Instant>,
}

impl Buffer {
    /// Wrap a document in a fresh buffer.
    #[must_use]
    pub fn new(document: Document) -> Self {
        let syntax = HighlightCache::new(syntax::detect_document(
            document.path(),
            &document.line_string(0),
        ));
        Self {
            document,
            history: History::default(),
            syntax,
            marks: HashMap::new(),
            options: LocalOptions::default(),
            large: false,
            swap_revision: None,
            swap_written: None,
        }
    }

    /// An empty scratch buffer.
    #[must_use]
    pub fn empty() -> Self {
        Self::new(Document::new())
    }

    /// Re-run language detection, after the file has been renamed or reloaded.
    pub fn detect_language(&mut self) {
        self.syntax.set_language(syntax::detect_document(
            self.document.path(),
            &self.document.line_string(0),
        ));
    }

    /// Write the buffer to its file and remember that state as the saved one.
    ///
    /// When `undo_dir` is given the history is written there too, so it is
    /// still there the next time the file is opened.
    ///
    /// # Errors
    /// Returns an error if the document has no path or the write fails.
    pub fn save(&mut self, undo_dir: Option<&Path>) -> Result<()> {
        self.prepare_save();
        self.document.save()?;
        self.after_save(undo_dir);
        Ok(())
    }

    /// Write the buffer to `path`, adopting it as the buffer's file.
    ///
    /// # Errors
    /// Returns an error if the write fails.
    pub fn save_as(&mut self, path: PathBuf, undo_dir: Option<&Path>) -> Result<()> {
        self.prepare_save();
        self.document.save_as(path)?;
        self.after_save(undo_dir);
        Ok(())
    }

    /// End the file with a line break when this file asks for one.
    fn prepare_save(&mut self) {
        if self.options.insert_final_newline != Some(true) {
            return;
        }
        let length = self.document.len_chars();
        if length == 0 || self.document.text().char(length - 1) == '\n' {
            return;
        }
        let end = self.document.char_to_pos(length);
        self.document.insert(length, "\n");
        self.history
            .record(crate::undo::Change::insertion(length, "\n"), end, end);
        self.history.checkpoint();
    }

    /// The settings in force for this file.
    #[must_use]
    pub fn config(&self, global: &Config) -> Config {
        self.options.apply(global)
    }

    fn after_save(&mut self, undo_dir: Option<&Path>) {
        self.history.mark_saved();
        if let (Some(directory), Some(path)) = (undo_dir, self.document.path()) {
            // Losing the history on disk is not worth failing a save over.
            let _ = persist::save(directory, path, &self.document.disk_text(), &self.history);
        }
    }

    /// Pick up the history saved for this file last time, if the file has not
    /// changed since.
    pub fn restore_history(&mut self, undo_dir: &Path, limit: usize) {
        let Some(path) = self.document.path() else {
            return;
        };
        if let Some(history) = persist::load(undo_dir, path, &self.document.disk_text(), limit) {
            self.history = history;
        } else {
            self.history.set_limit(limit);
        }
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
