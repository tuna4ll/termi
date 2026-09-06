//! # Selection
//!
//! **Purpose:** describe a span of text as a pair of character offsets.
//!
//! **Responsibility:** normalise a [`Cursor`]'s anchor and head into an ordered,
//! half-open range, and widen it to whole lines for line-wise visual mode.
//! Ranges are stored as character indices rather than as positions because
//! every consumer — deleting, yanking, highlighting, replacing — ultimately
//! needs offsets, and converting once here avoids doing it in four places.
//!
//! **Public API:** [`Range`].

use crate::editor::cursor::Cursor;
use crate::editor::document::Document;

/// A half-open span `[start, end)` of character indices.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Range {
    /// First character in the span.
    pub start: usize,
    /// One past the last character in the span.
    pub end: usize,
}

impl Range {
    /// An ordered range from two offsets, in either order.
    #[must_use]
    pub const fn new(a: usize, b: usize) -> Self {
        if a <= b {
            Self { start: a, end: b }
        } else {
            Self { start: b, end: a }
        }
    }

    /// The span a cursor covers in character-wise visual mode.
    ///
    /// Visual mode is *inclusive* of the character under the caret — selecting a
    /// single character and pressing `d` must delete it — so the head end is
    /// widened by one.
    #[must_use]
    pub fn of(cursor: &Cursor, doc: &Document) -> Self {
        let anchor = doc.pos_to_char(cursor.anchor);
        let head = doc.pos_to_char(cursor.head);
        let mut range = Self::new(anchor, head);
        range.end = (range.end + 1).min(doc.len_chars());
        range
    }

    /// The span between a cursor's anchor and head, with neither end widened.
    ///
    /// This is the *exclusive* selection a modeless editor makes: the caret sits
    /// between two characters, so Shift and one arrow key covers exactly one of
    /// them. [`Range::of`] is the other convention, where the caret covers a
    /// character and visual mode's selection includes it. The editor has both
    /// because it offers both ways of selecting, and each has to behave the way
    /// the people who reach for it expect.
    #[must_use]
    pub fn between(cursor: &Cursor, doc: &Document) -> Self {
        Self::new(doc.pos_to_char(cursor.anchor), doc.pos_to_char(cursor.head))
    }

    /// The span a cursor covers in line-wise visual mode: every line it touches,
    /// including the trailing newline of the last one.
    #[must_use]
    pub fn of_lines(cursor: &Cursor, doc: &Document) -> Self {
        let (first, last) = if cursor.anchor.line <= cursor.head.line {
            (cursor.anchor.line, cursor.head.line)
        } else {
            (cursor.head.line, cursor.anchor.line)
        };
        let start = doc.line_start(first);
        let end = if last >= doc.last_line() {
            doc.len_chars()
        } else {
            doc.line_start(last + 1)
        };
        Self { start, end }
    }

    /// Whether the range covers nothing.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.start >= self.end
    }

    /// Whether `index` falls inside the range.
    #[must_use]
    pub const fn contains(&self, index: usize) -> bool {
        index >= self.start && index < self.end
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::cursor::Position;

    fn doc(text: &str) -> Document {
        Document::from_text(text, None)
    }

    fn cursor(anchor: Position, head: Position) -> Cursor {
        let mut cursor = Cursor::at(anchor);
        cursor.head = head;
        cursor
    }

    #[test]
    fn a_backwards_selection_is_normalised() {
        let document = doc("hello world");
        let range = Range::of(&cursor(Position::new(0, 8), Position::new(0, 2)), &document);
        assert_eq!(range, Range { start: 2, end: 9 });
    }

    #[test]
    fn a_collapsed_cursor_still_covers_one_character() {
        let document = doc("abc");
        let range = Range::of(&cursor(Position::new(0, 1), Position::new(0, 1)), &document);
        assert_eq!(range.end - range.start, 1);
        assert!(range.contains(1));
        assert!(!range.contains(2));
    }

    #[test]
    fn a_selection_at_the_very_end_does_not_run_past_it() {
        let document = doc("abc");
        let range = Range::of(&cursor(Position::new(0, 2), Position::new(0, 2)), &document);
        assert_eq!(range.end, 3);
    }

    #[test]
    fn a_modeless_selection_excludes_the_character_under_the_head() {
        let document = doc("hello world");
        // Shift and one arrow key: anchor at 0, head at 1, one character.
        let range = Range::between(&cursor(Position::new(0, 0), Position::new(0, 1)), &document);
        assert_eq!(range, Range { start: 0, end: 1 });
        assert!(range.contains(0));
        assert!(!range.contains(1));
    }

    #[test]
    fn a_modeless_selection_can_run_to_the_end_of_a_line() {
        let document = doc("abc\ndef");
        // The head rests past the last character, where a bar cursor sits.
        let range = Range::between(&cursor(Position::new(0, 0), Position::new(0, 3)), &document);
        assert_eq!(range, Range { start: 0, end: 3 });
    }

    #[test]
    fn a_collapsed_cursor_selects_nothing_at_all() {
        let document = doc("abc");
        let range = Range::between(&cursor(Position::new(0, 1), Position::new(0, 1)), &document);
        assert!(range.is_empty());
    }

    #[test]
    fn line_wise_selection_covers_whole_lines() {
        let document = doc("aaa\nbbb\nccc\n");
        let range = Range::of_lines(&cursor(Position::new(0, 2), Position::new(1, 1)), &document);
        assert_eq!(range, Range { start: 0, end: 8 });
    }

    #[test]
    fn line_wise_selection_on_the_last_line_stops_at_the_end() {
        let document = doc("aaa\nbbb");
        let range = Range::of_lines(&cursor(Position::new(1, 0), Position::new(1, 2)), &document);
        assert_eq!(range, Range { start: 4, end: 7 });
    }
}
