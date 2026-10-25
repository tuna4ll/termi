//! # Undo history
//!
//! **Purpose:** let every edit be taken back.
//!
//! **Responsibility:** record edits as *inverse operations* and replay them.
//! Snapshotting the whole document per keystroke would be simpler but costs
//! O(file) memory per edit; storing what changed costs O(edit), which is what
//! makes undo affordable on a large file.
//!
//! Typing produces one change per character, so consecutive edits that continue
//! each other are merged into a single transaction — otherwise undo would step
//! backwards one letter at a time. A [`History::checkpoint`] closes the current
//! transaction; the editor calls it on mode changes and cursor motions, which is
//! exactly where a user expects an undo step to end.
//!
//! **Public API:** [`Change`], [`Transaction`], [`History`].

use crate::editor::cursor::Position;
use crate::editor::document::Document;

/// One replacement of `removed` with `inserted` at a character index.
///
/// Both sides may be empty: an insertion has no `removed`, a deletion has no
/// `inserted`. Swapping the two fields inverts the change, which is the whole
/// trick behind undo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    /// Character index the change starts at.
    pub at: usize,
    /// Text that was there before.
    pub removed: String,
    /// Text that is there now.
    pub inserted: String,
}

impl Change {
    /// An insertion of `text` at `at`.
    #[must_use]
    pub fn insertion(at: usize, text: impl Into<String>) -> Self {
        Self {
            at,
            removed: String::new(),
            inserted: text.into(),
        }
    }

    /// A deletion of `text` starting at `at`.
    #[must_use]
    pub fn deletion(at: usize, text: impl Into<String>) -> Self {
        Self {
            at,
            removed: text.into(),
            inserted: String::new(),
        }
    }

    /// The change that undoes this one.
    #[must_use]
    fn inverted(&self) -> Self {
        Self {
            at: self.at,
            removed: self.inserted.clone(),
            inserted: self.removed.clone(),
        }
    }

    /// Apply to a document.
    fn apply(&self, document: &mut Document) {
        if !self.removed.is_empty() {
            document.remove(self.at, self.at + self.removed.chars().count());
        }
        if !self.inserted.is_empty() {
            document.insert(self.at, &self.inserted);
        }
    }
}

/// A group of changes undone and redone as one step.
#[derive(Debug, Clone)]
pub struct Transaction {
    changes: Vec<Change>,
    /// Caret position before the transaction, restored on undo.
    before: Position,
    /// Caret position after the transaction, restored on redo.
    after: Position,
}

impl Transaction {
    /// Whether `change` continues the last one in this transaction.
    ///
    /// Only straight-line typing and straight-line backspacing merge; anything
    /// else — moving the caret, deleting a selection, pasting — starts a new
    /// undo step.
    fn absorbs(&self, change: &Change) -> bool {
        let Some(last) = self.changes.last() else {
            return false;
        };
        let typing = last.removed.is_empty()
            && change.removed.is_empty()
            && change.at == last.at + last.inserted.chars().count();
        let backspacing = last.inserted.is_empty()
            && change.inserted.is_empty()
            && change.at + change.removed.chars().count() == last.at;
        typing || backspacing
    }
}

/// The undo and redo stacks for one buffer.
#[derive(Debug)]
pub struct History {
    undo: Vec<Transaction>,
    redo: Vec<Transaction>,
    /// Whether the newest transaction may still absorb further changes.
    open: bool,
    /// Maximum number of undo steps kept.
    limit: usize,
    /// How many [`History::begin_group`] calls are still open.
    group_depth: usize,
    /// Whether the open group has recorded nothing yet.
    group_fresh: bool,
}

impl Default for History {
    fn default() -> Self {
        Self {
            undo: Vec::new(),
            redo: Vec::new(),
            open: false,
            limit: 1000,
            group_depth: 0,
            group_fresh: false,
        }
    }
}

impl History {
    /// Record a change that has already been applied to the document.
    ///
    /// `before` and `after` are the caret positions around the edit, so undo can
    /// put the caret back where the user was working.
    pub fn record(&mut self, change: Change, before: Position, after: Position) {
        if change.removed.is_empty() && change.inserted.is_empty() {
            return;
        }
        // Any new edit invalidates the redo branch.
        self.redo.clear();

        if self.group_depth > 0 {
            if !self.group_fresh
                && let Some(top) = self.undo.last_mut()
            {
                top.changes.push(change);
                top.after = after;
                return;
            }
            self.group_fresh = false;
            self.undo.push(Transaction {
                changes: vec![change],
                before,
                after,
            });
            return;
        }

        if self.open
            && let Some(top) = self.undo.last_mut()
            && top.absorbs(&change)
        {
            top.changes.push(change);
            top.after = after;
            return;
        }

        self.undo.push(Transaction {
            changes: vec![change],
            before,
            after,
        });
        self.open = true;

        if self.undo.len() > self.limit {
            self.undo.remove(0);
        }
    }

    /// Close the current transaction so the next edit starts a new undo step.
    pub fn checkpoint(&mut self) {
        self.open = false;
    }

    /// Gather every change until the matching [`History::end_group`] into one
    /// undo step, however they were made — `dw` at five cursors, a `:s` over
    /// the whole file, `ci(` before the typing that follows it.
    ///
    /// Groups nest; only the outermost one decides where the step begins and
    /// ends.
    pub fn begin_group(&mut self) {
        if self.group_depth == 0 {
            self.open = false;
            self.group_fresh = true;
        }
        self.group_depth += 1;
    }

    /// Close a group opened by [`History::begin_group`].
    pub fn end_group(&mut self) {
        self.group_depth = self.group_depth.saturating_sub(1);
        if self.group_depth == 0 {
            self.open = false;
        }
    }

    /// Undo one transaction, returning the caret position to restore.
    pub fn undo(&mut self, document: &mut Document) -> Option<Position> {
        let transaction = self.undo.pop()?;
        // Changes were applied front to back, so they must be reverted back to
        // front for the offsets to line up.
        for change in transaction.changes.iter().rev() {
            change.inverted().apply(document);
        }
        let position = transaction.before;
        self.redo.push(transaction);
        self.open = false;
        Some(position)
    }

    /// Redo one transaction, returning the caret position to restore.
    pub fn redo(&mut self, document: &mut Document) -> Option<Position> {
        let transaction = self.redo.pop()?;
        for change in &transaction.changes {
            change.apply(document);
        }
        let position = transaction.after;
        self.undo.push(transaction);
        self.open = false;
        Some(position)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(text: &str) -> Document {
        Document::from_text(text, None)
    }

    /// Type `text` one character at a time, as the editor does.
    fn type_text(history: &mut History, document: &mut Document, at: usize, text: &str) {
        for (offset, ch) in text.chars().enumerate() {
            let index = at + offset;
            document.insert(index, &ch.to_string());
            history.record(
                Change::insertion(index, ch.to_string()),
                Position::new(0, index),
                Position::new(0, index + 1),
            );
        }
    }

    #[test]
    fn consecutive_typing_undoes_as_one_step() {
        let mut document = doc("");
        let mut history = History::default();
        type_text(&mut history, &mut document, 0, "hello");
        assert_eq!(document.text().to_string(), "hello");

        history.undo(&mut document);
        assert_eq!(document.text().to_string(), "");
        assert!(history.undo(&mut document).is_none());
    }

    #[test]
    fn a_checkpoint_splits_typing_into_separate_steps() {
        let mut document = doc("");
        let mut history = History::default();
        type_text(&mut history, &mut document, 0, "ab");
        history.checkpoint();
        type_text(&mut history, &mut document, 2, "cd");

        history.undo(&mut document);
        assert_eq!(document.text().to_string(), "ab");
        history.undo(&mut document);
        assert_eq!(document.text().to_string(), "");
    }

    #[test]
    fn redo_replays_what_undo_took_back() {
        let mut document = doc("");
        let mut history = History::default();
        type_text(&mut history, &mut document, 0, "abc");

        history.undo(&mut document);
        history.redo(&mut document);
        assert_eq!(document.text().to_string(), "abc");
    }

    #[test]
    fn a_new_edit_discards_the_redo_branch() {
        let mut document = doc("");
        let mut history = History::default();
        type_text(&mut history, &mut document, 0, "abc");
        history.undo(&mut document);

        type_text(&mut history, &mut document, 0, "x");
        assert!(history.redo(&mut document).is_none());
    }

    #[test]
    fn a_group_undoes_as_one_step() {
        let mut history = History::default();
        let mut document = doc("abc");
        history.begin_group();
        document.remove(0, 1);
        history.record(Change::deletion(0, "a"), Position::ZERO, Position::ZERO);
        document.insert(0, "xy");
        history.record(Change::insertion(0, "xy"), Position::ZERO, Position::ZERO);
        history.end_group();
        assert_eq!(document.text().to_string(), "xybc");
        history.undo(&mut document);
        assert_eq!(document.text().to_string(), "abc");
    }

    #[test]
    fn undo_restores_the_caret_position() {
        let mut document = doc("abc");
        let mut history = History::default();
        document.remove(1, 2);
        history.record(
            Change::deletion(1, "b"),
            Position::new(0, 2),
            Position::new(0, 1),
        );

        assert_eq!(history.undo(&mut document), Some(Position::new(0, 2)));
        assert_eq!(document.text().to_string(), "abc");
    }

    #[test]
    fn a_multi_change_transaction_reverts_back_to_front() {
        let mut document = doc("aXbXc");
        let mut history = History::default();
        // A multi-cursor delete edits back to front so earlier offsets stay
        // valid, and records the changes in that same order.
        document.remove(3, 4);
        document.remove(1, 2);
        assert_eq!(document.text().to_string(), "abc");

        history.undo.push(Transaction {
            changes: vec![Change::deletion(3, "X"), Change::deletion(1, "X")],
            before: Position::ZERO,
            after: Position::ZERO,
        });

        history.undo(&mut document);
        assert_eq!(document.text().to_string(), "aXbXc");
    }
}
