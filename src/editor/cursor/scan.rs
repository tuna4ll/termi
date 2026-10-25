//! Motions that look further than the next word: paragraphs, bracket pairs and
//! a character on the current line.

use crate::editor::cursor::Position;
use crate::editor::document::Document;

/// How far `%` looks for a partner before giving up.
///
/// A bracket without a match should cost a bounded scan, not a walk through a
/// megabyte of text on every keystroke.
const BRACKET_SCAN_LIMIT: usize = 200_000;

/// Whether `line` holds nothing but whitespace.
#[must_use]
pub fn is_blank_line(doc: &Document, line: usize) -> bool {
    doc.line(line).chars().all(char::is_whitespace)
}

/// The next blank line after a run of text, or the end of the document — `}`.
#[must_use]
pub fn paragraph_forward(doc: &Document, from: Position) -> Position {
    let last = doc.last_line();
    let mut line = from.line;
    // Leave the blank lines we are standing in first, then the paragraph.
    while line < last && is_blank_line(doc, line) {
        line += 1;
    }
    while line < last && !is_blank_line(doc, line) {
        line += 1;
    }
    if line == last && !is_blank_line(doc, line) {
        return Position::new(last, doc.line_len(last));
    }
    Position::new(line, 0)
}

/// The blank line before the current paragraph, or the start — `{`.
#[must_use]
pub fn paragraph_backward(doc: &Document, from: Position) -> Position {
    let mut line = from.line;
    while line > 0 && is_blank_line(doc, line) {
        line -= 1;
    }
    while line > 0 && !is_blank_line(doc, line) {
        line -= 1;
    }
    Position::new(line, 0)
}

/// The bracket that pairs with `ch`, and whether it lies forward of it.
#[must_use]
pub const fn partner(ch: char) -> Option<(char, bool)> {
    Some(match ch {
        '(' => (')', true),
        '[' => (']', true),
        '{' => ('}', true),
        '<' => ('>', true),
        ')' => ('(', false),
        ']' => ('[', false),
        '}' => ('{', false),
        '>' => ('<', false),
        _ => return None,
    })
}

/// The character index of the bracket matching the one at `index`.
#[must_use]
pub fn matching_bracket(doc: &Document, index: usize) -> Option<usize> {
    let text = doc.text();
    let ch = text.get_char(index)?;
    let (other, forward) = partner(ch)?;
    let mut depth = 0usize;
    if forward {
        let end = text.len_chars().min(index + BRACKET_SCAN_LIMIT);
        for (offset, current) in text.chars_at(index).enumerate() {
            let at = index + offset;
            if at >= end {
                break;
            }
            if current == ch {
                depth += 1;
            } else if current == other {
                depth -= 1;
                if depth == 0 {
                    return Some(at);
                }
            }
        }
    } else {
        let start = index.saturating_sub(BRACKET_SCAN_LIMIT);
        let mut at = index + 1;
        let mut chars = text.chars_at(at);
        while at > start {
            let current = chars.prev()?;
            at -= 1;
            if current == ch {
                depth += 1;
            } else if current == other {
                depth -= 1;
                if depth == 0 {
                    return Some(at);
                }
            }
        }
    }
    None
}

/// `%`: the partner of the bracket under the caret, or of the first bracket
/// after it on the same line.
#[must_use]
pub fn bracket_partner_from(doc: &Document, from: Position) -> Option<Position> {
    let line_start = doc.line_start(from.line);
    let line_end = line_start + doc.line_len(from.line);
    let mut index = doc.pos_to_char(from);
    while index < line_end {
        let ch = doc.text().get_char(index)?;
        // `<` and `>` are comparison operators far more often than brackets, so
        // they only count when the caret is already on one.
        if partner(ch).is_some() && (!matches!(ch, '<' | '>') || index == doc.pos_to_char(from)) {
            return matching_bracket(doc, index).map(|at| doc.char_to_pos(at));
        }
        index += 1;
    }
    None
}

/// `f`, `t`, `F` and `T`: the `count`th `target` on the caret's line.
///
/// `till` stops one short of it, on the side the search came from.
#[must_use]
pub fn find_in_line(
    doc: &Document,
    from: Position,
    target: char,
    forward: bool,
    till: bool,
    count: usize,
) -> Option<Position> {
    let chars: Vec<char> = doc.line_string(from.line).chars().collect();
    let mut col = from.col;
    for round in 0..count.max(1) {
        // Repeating `t` from right beside its target would never move, so a
        // repeat steps over the character it is already up against.
        let skip = usize::from(till && round == 0 && count > 1);
        let found = if forward {
            let start = col + 1 + skip;
            (start..chars.len()).find(|&at| chars[at] == target)?
        } else {
            let end = col.checked_sub(skip)?;
            (0..end).rev().find(|&at| chars[at] == target)?
        };
        col = found;
    }
    let col = match (till, forward) {
        (false, _) => col,
        (true, true) => col.checked_sub(1)?,
        (true, false) => col + 1,
    };
    Some(Position::new(from.line, col))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(text: &str) -> Document {
        Document::from_text(text, None)
    }

    #[test]
    fn paragraphs_are_separated_by_blank_lines() {
        let document = doc("a\nb\n\nc\nd\n\ne");
        assert_eq!(
            paragraph_forward(&document, Position::new(0, 0)),
            Position::new(2, 0)
        );
        assert_eq!(
            paragraph_forward(&document, Position::new(2, 0)),
            Position::new(5, 0)
        );
        assert_eq!(
            paragraph_backward(&document, Position::new(4, 0)),
            Position::new(2, 0)
        );
    }

    #[test]
    fn brackets_match_across_nesting_and_lines() {
        let document = doc("f(a, (b)) {\n  x\n}");
        assert_eq!(matching_bracket(&document, 1), Some(8));
        assert_eq!(matching_bracket(&document, 8), Some(1));
        assert_eq!(
            bracket_partner_from(&document, Position::new(0, 9)),
            Some(Position::new(2, 0))
        );
    }

    #[test]
    fn find_and_till_on_the_current_line() {
        let document = doc("a,b,c,d");
        let origin = Position::new(0, 0);
        assert_eq!(
            find_in_line(&document, origin, ',', true, false, 2),
            Some(Position::new(0, 3))
        );
        assert_eq!(
            find_in_line(&document, origin, ',', true, true, 1),
            Some(Position::new(0, 0))
        );
        assert_eq!(
            find_in_line(&document, Position::new(0, 6), ',', false, false, 1),
            Some(Position::new(0, 5))
        );
        assert_eq!(find_in_line(&document, origin, 'z', true, false, 1), None);
    }
}
