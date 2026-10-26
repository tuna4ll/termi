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
//! Undo is a tree rather than a stack. Undoing and then typing something new
//! starts a branch instead of throwing the undone edits away, so nothing the
//! user typed is ever lost: `u` and `Ctrl+R` walk up and down the current
//! branch, while `g-` and `g+` walk every state the buffer has been in, in the
//! order it was in them, crossing branches as they go.
//!
//! **Public API:** [`Change`], [`Transaction`], [`History`], [`Snapshot`].

pub mod persist;

use serde::{Deserialize, Serialize};

use crate::editor::cursor::Position;
use crate::editor::document::Document;

/// One replacement of `removed` with `inserted` at a character index.
///
/// Both sides may be empty: an insertion has no `removed`, a deletion has no
/// `inserted`. Swapping the two fields inverts the change, which is the whole
/// trick behind undo.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Transaction {
    changes: Vec<Change>,
    /// Caret position before the transaction, restored on undo.
    before: Position,
    /// Caret position after the transaction, restored on redo.
    after: Position,
}

impl Transaction {
    /// Revert every change, last first.
    fn revert(&self, document: &mut Document) {
        // Changes were applied front to back, so they must be reverted back to
        // front for the offsets to line up.
        for change in self.changes.iter().rev() {
            change.inverted().apply(document);
        }
    }

    /// Apply every change again, first first.
    fn replay(&self, document: &mut Document) {
        for change in &self.changes {
            change.apply(document);
        }
    }

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

/// One state of the buffer: the edit that led to it, and where it sits in the
/// tree.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Node {
    /// The state this one was reached from; the root points at itself.
    parent: usize,
    /// The edit that turns the parent's text into this node's.
    transaction: Transaction,
    /// The branch `redo` follows — the child most recently left or made.
    redo_child: Option<usize>,
}

/// A buffer's history, as it is written to disk for persistent undo.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    nodes: Vec<Node>,
    current: usize,
}

/// Every state one buffer has been in.
#[derive(Debug)]
pub struct History {
    /// Node 0 is the oldest state still reachable; indices grow with time, so
    /// a node's index is also its place in chronological order.
    nodes: Vec<Node>,
    /// The state the document is in now.
    current: usize,
    /// The state the file on disk is in, when it is one of these.
    saved: Option<usize>,
    /// Whether the newest transaction may still absorb further changes.
    open: bool,
    /// Maximum number of states kept.
    limit: usize,
    /// How many [`History::begin_group`] calls are still open.
    group_depth: usize,
    /// Whether the open group has recorded nothing yet.
    group_fresh: bool,
}

impl Default for History {
    fn default() -> Self {
        Self::with_limit(1000)
    }
}

impl History {
    /// An empty history keeping at most `limit` states.
    #[must_use]
    pub fn with_limit(limit: usize) -> Self {
        Self {
            nodes: vec![Node {
                parent: 0,
                transaction: Transaction::default(),
                redo_child: None,
            }],
            current: 0,
            saved: Some(0),
            open: false,
            limit: limit.max(1),
            group_depth: 0,
            group_fresh: false,
        }
    }

    /// Record a change that has already been applied to the document.
    ///
    /// `before` and `after` are the caret positions around the edit, so undo can
    /// put the caret back where the user was working.
    pub fn record(&mut self, change: Change, before: Position, after: Position) {
        if change.removed.is_empty() && change.inserted.is_empty() {
            return;
        }
        let extend = if self.group_depth > 0 {
            !std::mem::replace(&mut self.group_fresh, false)
        } else {
            self.open && self.nodes[self.current].transaction.absorbs(&change)
        };
        // A state that was saved and then extended is no longer the saved one.
        if extend && self.current != 0 && self.saved != Some(self.current) {
            let top = &mut self.nodes[self.current].transaction;
            top.changes.push(change);
            top.after = after;
            return;
        }

        let parent = self.current;
        self.nodes.push(Node {
            parent,
            transaction: Transaction {
                changes: vec![change],
                before,
                after,
            },
            redo_child: None,
        });
        self.current = self.nodes.len() - 1;
        self.nodes[parent].redo_child = Some(self.current);
        self.open = self.group_depth == 0;
        self.prune();
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
        if self.current == 0 {
            return None;
        }
        let node = &self.nodes[self.current];
        node.transaction.revert(document);
        let position = node.transaction.before;
        let parent = node.parent;
        self.nodes[parent].redo_child = Some(self.current);
        self.current = parent;
        self.open = false;
        Some(position)
    }

    /// Redo one transaction, returning the caret position to restore.
    pub fn redo(&mut self, document: &mut Document) -> Option<Position> {
        let child = self.nodes[self.current].redo_child?;
        let node = &self.nodes[child];
        node.transaction.replay(document);
        self.current = child;
        self.open = false;
        Some(node.transaction.after)
    }

    /// Step `count` states back in time — `g-` — whichever branch they are on.
    pub fn earlier(&mut self, document: &mut Document, count: usize) -> Option<Position> {
        let target = self.current.saturating_sub(count.max(1));
        (target != self.current).then(|| self.travel(document, target))
    }

    /// Step `count` states forward in time — `g+`.
    pub fn later(&mut self, document: &mut Document, count: usize) -> Option<Position> {
        let target = (self.current + count.max(1)).min(self.nodes.len() - 1);
        (target != self.current).then(|| self.travel(document, target))
    }

    /// Move the document to the state of `target`, undoing up to the branch
    /// point the two states share and redoing down from it.
    fn travel(&mut self, document: &mut Document, target: usize) -> Position {
        let ancestors = |mut node: usize| {
            let mut chain = vec![node];
            while node != 0 {
                node = self.nodes[node].parent;
                chain.push(node);
            }
            chain
        };
        let from_current = ancestors(self.current);
        let from_target = ancestors(target);
        let common = *from_current
            .iter()
            .find(|node| from_target.contains(node))
            .unwrap_or(&0);

        let mut position = self.nodes[self.current].transaction.after;
        for &node in from_current.iter().take_while(|node| **node != common) {
            self.nodes[node].transaction.revert(document);
            position = self.nodes[node].transaction.before;
        }
        let downward: Vec<usize> = from_target
            .iter()
            .take_while(|node| **node != common)
            .copied()
            .collect();
        for &node in downward.iter().rev() {
            self.nodes[node].transaction.replay(document);
            let parent = self.nodes[node].parent;
            self.nodes[parent].redo_child = Some(node);
            position = self.nodes[node].transaction.after;
        }
        self.current = target;
        self.open = false;
        position
    }

    /// Remember that the document now matches the file on disk.
    pub fn mark_saved(&mut self) {
        self.saved = Some(self.current);
        self.open = false;
    }

    /// Whether the document is in the state that was last saved.
    #[must_use]
    pub fn is_saved(&self) -> bool {
        self.saved == Some(self.current)
    }

    /// Change how many states are kept.
    pub fn set_limit(&mut self, limit: usize) {
        self.limit = limit.max(1);
        self.prune();
    }

    /// Drop the oldest states once there are too many.
    ///
    /// Pruning a tree by age would cut branches in half, so instead the
    /// history collapses to the straight line of states leading to the current
    /// one, keeping the newest `limit` of them; the state just before them
    /// becomes the new root. It runs rarely — only once the tree has grown to
    /// twice the limit — so the copying is amortised away.
    fn prune(&mut self) {
        if self.nodes.len() <= self.limit.saturating_mul(2) {
            return;
        }
        let mut line = vec![self.current];
        let mut node = self.current;
        while node != 0 {
            node = self.nodes[node].parent;
            line.push(node);
        }
        line.reverse();
        let keep = line.len().min(self.limit + 1);
        let kept = &line[line.len() - keep..];

        let mut nodes = Vec::with_capacity(kept.len());
        for (index, old) in kept.iter().enumerate() {
            let mut copy = self.nodes[*old].clone();
            copy.parent = index.saturating_sub(1);
            copy.redo_child = (index + 1 < kept.len()).then_some(index + 1);
            if index == 0 {
                copy.transaction = Transaction::default();
            }
            nodes.push(copy);
        }
        self.saved = self
            .saved
            .and_then(|saved| kept.iter().position(|old| *old == saved));
        self.current = nodes.len() - 1;
        self.nodes = nodes;
    }

    /// The history in a form that can be written to disk.
    #[must_use]
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            nodes: self.nodes.clone(),
            current: self.current,
        }
    }

    /// Rebuild a history from disk, taking the current state as the saved
    /// one — it was written when the file was.
    #[must_use]
    pub fn restore(snapshot: Snapshot, limit: usize) -> Option<Self> {
        let valid = !snapshot.nodes.is_empty()
            && snapshot.current < snapshot.nodes.len()
            && snapshot.nodes.iter().enumerate().all(|(index, node)| {
                node.parent < snapshot.nodes.len() && (index == 0 || node.parent < index)
            });
        if !valid {
            return None;
        }
        let mut history = Self::with_limit(limit);
        history.nodes = snapshot.nodes;
        history.current = snapshot.current;
        history.saved = Some(snapshot.current);
        Some(history)
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
    fn a_new_edit_after_undo_starts_a_branch() {
        let mut document = doc("");
        let mut history = History::default();
        type_text(&mut history, &mut document, 0, "abc");
        history.undo(&mut document);

        type_text(&mut history, &mut document, 0, "x");
        assert!(history.redo(&mut document).is_none());

        // The undone branch is still there, one step back in time, and the
        // empty text it grew from one step before that.
        history.earlier(&mut document, 1);
        assert_eq!(document.text().to_string(), "abc");
        history.earlier(&mut document, 1);
        assert_eq!(document.text().to_string(), "");
        history.later(&mut document, 2);
        assert_eq!(document.text().to_string(), "x");
    }

    #[test]
    fn the_saved_state_is_recognised_after_undo_and_redo() {
        let mut document = doc("");
        let mut history = History::default();
        type_text(&mut history, &mut document, 0, "ab");
        history.mark_saved();
        assert!(history.is_saved());
        history.checkpoint();
        type_text(&mut history, &mut document, 2, "c");
        assert!(!history.is_saved());
        history.undo(&mut document);
        assert!(history.is_saved());
    }

    #[test]
    fn typing_after_a_save_starts_a_new_step() {
        let mut document = doc("");
        let mut history = History::default();
        type_text(&mut history, &mut document, 0, "ab");
        history.mark_saved();
        type_text(&mut history, &mut document, 2, "cd");
        history.undo(&mut document);
        assert_eq!(document.text().to_string(), "ab");
    }

    #[test]
    fn pruning_keeps_the_newest_states() {
        let mut document = doc("");
        let mut history = History::with_limit(3);
        for index in 0..10 {
            history.checkpoint();
            type_text(&mut history, &mut document, index, "x");
        }
        let mut steps = 0;
        while history.undo(&mut document).is_some() {
            steps += 1;
        }
        assert!(steps >= 3, "kept {steps}");
        assert!(steps <= 6, "kept {steps}");
    }

    #[test]
    fn a_snapshot_round_trips() {
        let mut document = doc("");
        let mut history = History::default();
        type_text(&mut history, &mut document, 0, "hey");
        let restored = History::restore(history.snapshot(), 100).expect("valid snapshot");
        let mut restored = restored;
        restored.undo(&mut document);
        assert_eq!(document.text().to_string(), "");
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

        history.begin_group();
        history.record(Change::deletion(3, "X"), Position::ZERO, Position::ZERO);
        history.record(Change::deletion(1, "X"), Position::ZERO, Position::ZERO);
        history.end_group();

        history.undo(&mut document);
        assert_eq!(document.text().to_string(), "aXbXc");
    }
}
