//! # Bracket pairs
//!
//! **Purpose:** answer "what does typing a bracket or a quote here mean?".
//!
//! **Responsibility:** pure functions over a [`Document`], in the same spirit as
//! [`indent`](crate::editor::document::indent). Nothing here remembers which
//! brackets the editor inserted: the decision is taken from the text around the
//! caret every time, so it survives undo, a reload, and a file someone else
//! edited. That costs a little precision — closing a bracket the user typed
//! themselves also steps over it — and buys behaviour that never disagrees with
//! what is on screen.
//!
//! **Public API:** [`Insertion`], [`resolve`], [`surrounds`], [`closing_for`].

use crate::editor::cursor::Position;
use crate::editor::document::Document;

/// The pairs that are completed automatically.
///
/// Quotes are the awkward case, and the reason this is a list of pairs rather
/// than of brackets: they are their own closing half, so `'` may equally well
/// be an apostrophe.
const PAIRS: [(char, char); 6] = [
    ('(', ')'),
    ('[', ']'),
    ('{', '}'),
    ('"', '"'),
    ('\'', '\''),
    ('`', '`'),
];

/// What typing one character should actually do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Insertion {
    /// Insert the character and nothing else.
    Plain,
    /// Insert the character with its closing half, leaving the caret between.
    Pair(char),
    /// Insert nothing, and step over the character already there.
    Skip,
}

/// The closing half of `open`, if it has one.
#[must_use]
pub fn closing_for(open: char) -> Option<char> {
    PAIRS
        .iter()
        .find(|(candidate, _)| *candidate == open)
        .map(|(_, close)| *close)
}

/// Decide what typing `ch` at `pos` means.
#[must_use]
pub fn resolve(doc: &Document, pos: Position, ch: char) -> Insertion {
    let next = char_at(doc, pos);

    // Typing a closing bracket that is already there steps over it, so finishing
    // a call by typing `)` leaves one bracket rather than two.
    if is_closing(ch) && next == Some(ch) {
        return Insertion::Skip;
    }
    let Some(close) = closing_for(ch) else {
        return Insertion::Plain;
    };

    // Opening in front of a word would bracket text the user never selected:
    // typing `(` before `foo` means "wrap what I am about to write", not
    // "(foo)".
    if next.is_some_and(is_word) {
        return Insertion::Plain;
    }

    // A quote closes itself, so it needs a test the brackets do not: directly
    // after a word character it is an apostrophe or the end of something far
    // more often than the start of a string. After a backslash it is escaped.
    if close == ch {
        let previous = char_before(doc, pos);
        if previous.is_some_and(|previous| is_word(previous) || previous == ch || previous == '\\')
        {
            return Insertion::Plain;
        }
    }
    Insertion::Pair(close)
}

/// Whether `pos` sits between the two halves of a pair.
///
/// What backspace asks before deciding to remove both, and what Enter asks
/// before pushing the closing half down onto its own line.
#[must_use]
pub fn surrounds(doc: &Document, pos: Position) -> bool {
    match (char_before(doc, pos), char_at(doc, pos)) {
        (Some(open), Some(close)) => closing_for(open) == Some(close),
        _ => false,
    }
}

/// Whether `ch` is the closing half of some pair.
fn is_closing(ch: char) -> bool {
    PAIRS.iter().any(|(_, close)| *close == ch)
}

/// Whether `ch` is the kind of character an identifier is made of.
fn is_word(ch: char) -> bool {
    ch.is_alphanumeric() || ch == '_'
}

/// The character the caret is on, or `None` at end of line.
fn char_at(doc: &Document, pos: Position) -> Option<char> {
    (pos.col < doc.line_len(pos.line)).then(|| doc.line(pos.line).char(pos.col))
}

/// The character before the caret, or `None` at the start of a line.
fn char_before(doc: &Document, pos: Position) -> Option<char> {
    (pos.col > 0).then(|| doc.line(pos.line).char(pos.col - 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What typing `ch` means with the caret at `col` of a one-line document.
    fn typing(text: &str, col: usize, ch: char) -> Insertion {
        let doc = Document::from_text(text, None);
        resolve(&doc, Position::new(0, col), ch)
    }

    #[test]
    fn a_bracket_typed_at_the_end_of_a_line_is_closed() {
        assert_eq!(typing("call", 4, '('), Insertion::Pair(')'));
        assert_eq!(typing("", 0, '{'), Insertion::Pair('}'));
        assert_eq!(typing("", 0, '['), Insertion::Pair(']'));
    }

    #[test]
    fn a_bracket_typed_in_front_of_a_word_is_left_alone() {
        // `(` before `foo` would otherwise produce `()foo`.
        assert_eq!(typing("foo", 0, '('), Insertion::Plain);
        // Whitespace and other brackets are not words, so those still close.
        assert_eq!(typing(" foo", 0, '('), Insertion::Pair(')'));
        assert_eq!(typing(")", 0, '('), Insertion::Pair(')'));
    }

    #[test]
    fn typing_a_closing_bracket_steps_over_the_one_already_there() {
        assert_eq!(typing("()", 1, ')'), Insertion::Skip);
        assert_eq!(typing("{}", 1, '}'), Insertion::Skip);
        // With nothing to step over it is an ordinary character.
        assert_eq!(typing("", 0, ')'), Insertion::Plain);
    }

    #[test]
    fn a_quote_opens_a_pair_where_a_string_could_start() {
        assert_eq!(typing("let s = ", 8, '"'), Insertion::Pair('"'));
        assert_eq!(typing("", 0, '\''), Insertion::Pair('\''));
    }

    #[test]
    fn an_apostrophe_inside_a_word_stays_a_single_character() {
        assert_eq!(typing("don", 3, '\''), Insertion::Plain);
        assert_eq!(typing("it", 2, '"'), Insertion::Plain);
    }

    #[test]
    fn an_escaped_quote_is_not_a_pair() {
        assert_eq!(typing("\"a\\", 3, '"'), Insertion::Plain);
    }

    #[test]
    fn the_closing_quote_of_a_pair_is_stepped_over() {
        assert_eq!(typing("\"\"", 1, '"'), Insertion::Skip);
    }

    #[test]
    fn a_third_quote_does_not_open_yet_another_pair() {
        // Caret after `""`: another quote here is closing something, not
        // opening a string inside an empty one.
        assert_eq!(typing("\"\"", 2, '"'), Insertion::Plain);
    }

    #[test]
    fn a_caret_between_the_halves_of_a_pair_is_recognised() {
        let doc = Document::from_text("()", None);
        assert!(surrounds(&doc, Position::new(0, 1)));
        assert!(!surrounds(&doc, Position::new(0, 0)));
        assert!(!surrounds(&doc, Position::new(0, 2)));

        let mismatched = Document::from_text("(]", None);
        assert!(!surrounds(&mismatched, Position::new(0, 1)));
    }
}
