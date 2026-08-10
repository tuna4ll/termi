//! # Window
//!
//! **Purpose:** one view onto a buffer.
//!
//! **Responsibility:** everything that is a property of *looking at* a file
//! rather than of the file itself — which buffer is on show, where it is
//! scrolled to, and where the cursors are. Splitting this out of
//! [`Buffer`](crate::editor::buffer::Buffer) is what lets the same file be open
//! in two windows with independent carets while sharing one text and one undo
//! history.
//!
//! The document is passed in to the methods that need it instead of being held
//! here, because a window borrows its buffer from the editor's buffer list and
//! cannot own it.
//!
//! **Public API:** [`Window`], [`View`].

pub mod tree;
pub mod view;

use std::collections::HashMap;

use crate::editor::buffer::BufferId;
use crate::editor::cursor::{Cursor, Motion, Position};
use crate::editor::document::Document;

pub use tree::{Axis, Side, WindowId, Windows};
pub use view::View;

/// A rectangle of terminal cells.
///
/// Plain numbers rather than the renderer's rectangle type: a window has to
/// remember where it was drawn — page motions need its height, directional
/// focus needs its position, and a mouse click needs to be matched against it —
/// but the editor layer stays free of any dependency on the terminal.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Area {
    /// Column of the left edge.
    pub x: u16,
    /// Row of the top edge.
    pub y: u16,
    /// Width in cells.
    pub width: u16,
    /// Height in cells.
    pub height: u16,
}

impl Area {
    /// Build an area from its origin and size.
    #[must_use]
    pub const fn new(x: u16, y: u16, width: u16, height: u16) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// One past the rightmost column.
    #[must_use]
    pub const fn right(&self) -> u16 {
        self.x + self.width
    }

    /// One past the bottom row.
    #[must_use]
    pub const fn bottom(&self) -> u16 {
        self.y + self.height
    }

    /// Middle of the area, rounded down.
    #[must_use]
    pub const fn centre(&self) -> (u16, u16) {
        (self.x + self.width / 2, self.y + self.height / 2)
    }

    /// Whether `(x, y)` falls inside.
    #[must_use]
    pub const fn contains(&self, x: u16, y: u16) -> bool {
        x >= self.x && x < self.right() && y >= self.y && y < self.bottom()
    }
}

/// Where a window was looking when it last showed a particular buffer.
#[derive(Debug, Clone)]
struct Anchor {
    view: View,
    cursors: Vec<Cursor>,
    primary: usize,
}

/// A viewport onto a buffer, with its own cursors.
#[derive(Debug, Clone)]
pub struct Window {
    /// Which buffer this window shows.
    pub buffer: BufferId,
    /// Scroll position.
    pub view: View,
    /// Every cursor, kept sorted by head position and never empty.
    ///
    /// Multi-cursor is modelled from the start rather than bolted on: every edit
    /// already iterates this vector, so adding a second cursor is a UI change
    /// rather than an editing-core change. Document order matters because edits
    /// are applied back-to-front, which keeps earlier offsets valid.
    cursors: Vec<Cursor>,
    /// Index into `cursors` of the one the viewport follows.
    primary: usize,
    /// Where this window's text area was on the last frame.
    ///
    /// Only knowable at render time, so the renderer records it here for the
    /// next key press to use: page motions need the height, moving the focus
    /// between splits needs the position, and a mouse click needs both.
    pub area: Area,
    /// Caret and scroll for buffers this window showed earlier.
    ///
    /// Cursors belong to the window now, so without this, switching to another
    /// file and back would drop the reader at the top of it. Vi keeps this per
    /// window rather than per file for the same reason two splits on one file
    /// scroll independently.
    anchors: HashMap<BufferId, Anchor>,
}

impl Window {
    /// A window on `buffer`, with a single cursor at the top.
    #[must_use]
    pub fn new(buffer: BufferId) -> Self {
        Self {
            buffer,
            view: View::default(),
            cursors: vec![Cursor::at(Position::ZERO)],
            primary: 0,
            area: Area::new(0, 0, 1, 1),
            anchors: HashMap::new(),
        }
    }

    /// Show `buffer`, remembering where this window was in the current one and
    /// returning to wherever it last was in the new one.
    pub fn show(&mut self, buffer: BufferId) {
        if self.buffer == buffer {
            return;
        }
        self.anchors.insert(
            self.buffer,
            Anchor {
                view: self.view,
                cursors: self.cursors.clone(),
                primary: self.primary,
            },
        );
        self.load(buffer);
    }

    /// Point this window at `buffer`, restoring a remembered position if there
    /// is one and starting at the top otherwise.
    fn load(&mut self, buffer: BufferId) {
        self.buffer = buffer;
        match self.anchors.remove(&buffer) {
            Some(anchor) => {
                self.view = anchor.view;
                self.cursors = anchor.cursors;
                self.primary = anchor.primary;
            }
            None => {
                self.view = View::default();
                self.cursors = vec![Cursor::at(Position::ZERO)];
                self.primary = 0;
            }
        }
    }

    /// Point this window at `buffer` and start from the top, forgetting any
    /// position remembered for it.
    ///
    /// Used when a buffer is replaced in place — the remembered caret belongs to
    /// text that no longer exists.
    pub fn reset(&mut self, buffer: BufferId) {
        self.anchors.remove(&buffer);
        self.buffer = buffer;
        self.view = View::default();
        self.cursors = vec![Cursor::at(Position::ZERO)];
        self.primary = 0;
    }

    /// Repair this window after the buffer at `removed` left the buffer list.
    ///
    /// Buffer ids are positions in that list, so everything above the hole
    /// shifts down by one. `fallback` is where a window that was showing the
    /// removed buffer should land, already expressed in the new numbering.
    pub fn buffer_removed(&mut self, removed: BufferId, fallback: BufferId) {
        let shift = |id: BufferId| if id > removed { id - 1 } else { id };
        self.anchors = std::mem::take(&mut self.anchors)
            .into_iter()
            .filter(|(id, _)| *id != removed)
            .map(|(id, anchor)| (shift(id), anchor))
            .collect();

        if self.buffer == removed {
            self.load(fallback);
        } else {
            self.buffer = shift(self.buffer);
        }
    }

    /// The primary cursor — the one the viewport follows and the status bar
    /// reports.
    #[must_use]
    pub fn cursor(&self) -> Cursor {
        self.cursors[self.primary]
    }

    /// Mutable access to the primary cursor.
    pub fn cursor_mut(&mut self) -> &mut Cursor {
        &mut self.cursors[self.primary]
    }

    /// All cursors, in document order.
    #[must_use]
    pub fn cursors(&self) -> &[Cursor] {
        &self.cursors
    }

    /// Mutable access to every cursor, in document order.
    ///
    /// Editing walks this by index so it can move each cursor as it applies the
    /// edit at it; the order that indexing depends on is restored afterwards by
    /// [`Window::resort`]. A slice rather than the vector, because the *set* of
    /// cursors only changes through the methods below.
    pub fn cursors_mut(&mut self) -> &mut [Cursor] {
        &mut self.cursors
    }

    /// Move every cursor by the same motion.
    pub fn move_cursors(
        &mut self,
        motion: Motion,
        document: &Document,
        extend: bool,
        allow_eol: bool,
    ) {
        for cursor in &mut self.cursors {
            cursor.apply(motion, document, extend, allow_eol);
        }
        self.resort();
    }

    /// Drop every selection, leaving the carets where they are.
    pub fn collapse_selections(&mut self) {
        for cursor in &mut self.cursors {
            cursor.collapse();
        }
    }

    /// Start a selection at every caret, as entering visual mode does.
    pub fn anchor_selections(&mut self) {
        for cursor in &mut self.cursors {
            cursor.anchor_here();
        }
    }

    /// Add a secondary cursor, ignoring one that already exists there.
    pub fn add_cursor(&mut self, cursor: Cursor) {
        if self.cursors.iter().any(|c| c.head == cursor.head) {
            return;
        }
        self.cursors.push(cursor);
        self.resort();
    }

    /// Collapse back to a single cursor, keeping the primary one.
    pub fn clear_secondary_cursors(&mut self) {
        let primary = self.cursors[self.primary];
        self.cursors.clear();
        self.cursors.push(primary);
        self.primary = 0;
    }

    /// Indices of all cursors, last in the document first.
    ///
    /// Editing from the end backwards means each edit only shifts text *after*
    /// the cursors already handled, so no offset fix-up pass is needed.
    #[must_use]
    pub fn edit_order(&self) -> Vec<usize> {
        (0..self.cursors.len()).rev().collect()
    }

    /// Pull every cursor back inside the document.
    ///
    /// Called after any edit that can shrink the text — undo, reload, deleting a
    /// selection — so no cursor is left pointing past the end. It also runs on
    /// the *other* windows showing an edited buffer, which are not otherwise
    /// told that the text moved underneath them.
    pub fn clamp_cursors(&mut self, document: &Document, allow_eol: bool) {
        for cursor in &mut self.cursors {
            cursor.head = document.clamp(cursor.head, allow_eol);
            cursor.anchor = document.clamp(cursor.anchor, allow_eol);
        }
        self.resort();
    }

    /// Restore document order, drop cursors that collided, and follow the
    /// primary one to its new index.
    pub fn resort(&mut self) {
        if self.cursors.len() == 1 {
            self.primary = 0;
            return;
        }
        let primary_head = self.cursors[self.primary].head;
        self.cursors.sort_by_key(|c| c.head);
        self.cursors.dedup_by_key(|c| c.head);
        self.primary = self
            .cursors
            .iter()
            .position(|c| c.head == primary_head)
            .unwrap_or(0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window() -> Window {
        Window::new(0)
    }

    #[test]
    fn a_new_window_has_exactly_one_cursor() {
        let win = window();
        assert_eq!(win.cursors().len(), 1);
        assert_eq!(win.cursor().head, Position::ZERO);
    }

    #[test]
    fn cursors_stay_in_document_order() {
        let mut win = window();
        win.add_cursor(Cursor::at(Position::new(2, 1)));
        win.add_cursor(Cursor::at(Position::new(1, 1)));
        let heads: Vec<_> = win.cursors().iter().map(|c| c.head).collect();
        assert_eq!(
            heads,
            vec![Position::ZERO, Position::new(1, 1), Position::new(2, 1)]
        );
        // The primary cursor followed its position through the sort.
        assert_eq!(win.cursor().head, Position::ZERO);
    }

    #[test]
    fn duplicate_cursors_are_rejected() {
        let mut win = window();
        win.add_cursor(Cursor::at(Position::ZERO));
        assert_eq!(win.cursors().len(), 1);
    }

    #[test]
    fn collapsing_cursors_keeps_the_primary_one() {
        let mut win = window();
        win.add_cursor(Cursor::at(Position::new(1, 2)));
        win.clear_secondary_cursors();
        assert_eq!(win.cursors().len(), 1);
        assert_eq!(win.cursor().head, Position::ZERO);
    }

    #[test]
    fn merged_cursors_do_not_leave_a_dangling_primary() {
        let document = Document::from_text("abc", None);
        let mut win = window();
        win.add_cursor(Cursor::at(Position::new(0, 1)));
        // Both cursors run into the same end-of-line position and merge.
        win.move_cursors(Motion::LineEnd, &document, false, false);
        assert_eq!(win.cursors().len(), 1);
        assert_eq!(win.cursor().head, Position::new(0, 2));
    }

    #[test]
    fn edit_order_runs_backwards_through_the_document() {
        let mut win = window();
        win.add_cursor(Cursor::at(Position::new(1, 0)));
        win.add_cursor(Cursor::at(Position::new(2, 0)));
        assert_eq!(win.edit_order(), vec![2, 1, 0]);
    }

    #[test]
    fn clamping_pulls_cursors_back_inside_a_shrunken_document() {
        let mut win = window();
        win.add_cursor(Cursor::at(Position::new(9, 9)));
        win.clamp_cursors(&Document::from_text("ab", None), false);
        let heads: Vec<_> = win.cursors().iter().map(|c| c.head).collect();
        assert_eq!(heads, vec![Position::ZERO, Position::new(0, 1)]);
    }

    #[test]
    fn switching_buffers_and_back_returns_to_the_same_place() {
        let mut win = window();
        win.cursor_mut().move_to(Position::new(4, 2), false);
        win.view.top_line = 3;

        win.show(1);
        assert_eq!(win.cursor().head, Position::ZERO);
        assert_eq!(win.view.top_line, 0);

        win.show(0);
        assert_eq!(win.cursor().head, Position::new(4, 2));
        assert_eq!(win.view.top_line, 3);
    }

    #[test]
    fn showing_the_buffer_already_on_screen_does_nothing() {
        let mut win = window();
        win.cursor_mut().move_to(Position::new(2, 2), false);
        win.show(0);
        assert_eq!(win.cursor().head, Position::new(2, 2));
    }

    #[test]
    fn closing_a_buffer_shifts_the_ids_above_it_down() {
        let mut win = Window::new(2);
        win.buffer_removed(0, 0);
        assert_eq!(win.buffer, 1);
    }

    #[test]
    fn closing_the_shown_buffer_falls_back_to_another() {
        let mut win = Window::new(1);
        win.cursor_mut().move_to(Position::new(3, 0), false);
        win.buffer_removed(1, 0);
        assert_eq!(win.buffer, 0);
        // The fallback buffer was never shown here, so it starts at the top.
        assert_eq!(win.cursor().head, Position::ZERO);
    }

    #[test]
    fn a_remembered_position_survives_an_unrelated_buffer_closing() {
        let mut win = Window::new(0);
        win.cursor_mut().move_to(Position::new(5, 1), false);
        // Look at buffer 2, which parks buffer 0's caret in the anchor map.
        win.show(2);
        // Buffer 1 closes; buffer 2 becomes buffer 1 and buffer 0 stays put.
        win.buffer_removed(1, 0);
        assert_eq!(win.buffer, 1);
        win.show(0);
        assert_eq!(win.cursor().head, Position::new(5, 1));
    }
}
