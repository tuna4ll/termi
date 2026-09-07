//! Word boundaries: classify character runs and walk over them.

use crate::editor::cursor::Position;
use crate::editor::document::Document;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Class {
    Whitespace,
    Word,
    Punctuation,
}

impl Class {
    fn of(ch: char) -> Self {
        if ch.is_whitespace() {
            Self::Whitespace
        } else if ch.is_alphanumeric() || ch == '_' {
            Self::Word
        } else {
            Self::Punctuation
        }
    }
}

fn class_at(doc: &Document, index: usize) -> Option<Class> {
    doc.text().get_char(index).map(Class::of)
}

#[must_use]
pub fn next_word_start(doc: &Document, from: Position) -> Position {
    let len = doc.len_chars();
    let mut index = doc.pos_to_char(from);

    if let Some(start_class) = class_at(doc, index)
        && start_class != Class::Whitespace
    {
        while class_at(doc, index) == Some(start_class) {
            index += 1;
        }
    }
    while index < len && class_at(doc, index) == Some(Class::Whitespace) {
        index += 1;
    }
    doc.char_to_pos(index.min(len))
}

#[must_use]
pub fn prev_word_start(doc: &Document, from: Position) -> Position {
    let mut index = doc.pos_to_char(from);
    if index == 0 {
        return Position::ZERO;
    }
    index -= 1;

    while index > 0 && class_at(doc, index) == Some(Class::Whitespace) {
        index -= 1;
    }
    if let Some(class) = class_at(doc, index)
        && class != Class::Whitespace
    {
        while index > 0 && class_at(doc, index - 1) == Some(class) {
            index -= 1;
        }
    }
    doc.char_to_pos(index)
}

#[must_use]
pub fn word_end(doc: &Document, from: Position) -> Position {
    let len = doc.len_chars();
    let mut index = doc.pos_to_char(from);
    if index + 1 >= len {
        return doc.char_to_pos(len.saturating_sub(1));
    }
    index += 1;

    while index < len && class_at(doc, index) == Some(Class::Whitespace) {
        index += 1;
    }
    if let Some(class) = class_at(doc, index) {
        while class_at(doc, index + 1) == Some(class) {
            index += 1;
        }
    }
    doc.char_to_pos(index.min(len.saturating_sub(1)))
}

fn char_at(doc: &Document, index: usize) -> Option<char> {
    doc.text().get_char(index)
}

/// A run of blanks is its own unit, so one chord clears a whole gap.
#[must_use]
pub fn delete_start(doc: &Document, from: Position) -> Position {
    let mut index = doc.pos_to_char(from);
    let Some(previous) = index.checked_sub(1).and_then(|at| char_at(doc, at)) else {
        return from;
    };
    if previous == '\n' {
        return doc.char_to_pos(index - 1);
    }
    if previous.is_whitespace() {
        while matches!(index.checked_sub(1).and_then(|at| char_at(doc, at)), Some(ch) if ch.is_whitespace() && ch != '\n')
        {
            index -= 1;
        }
    } else {
        let class = Class::of(previous);
        while index > 0 && class_at(doc, index - 1) == Some(class) {
            index -= 1;
        }
    }
    doc.char_to_pos(index)
}

#[must_use]
pub fn delete_end(doc: &Document, from: Position) -> Position {
    let len = doc.len_chars();
    let mut index = doc.pos_to_char(from);
    let Some(current) = char_at(doc, index) else {
        return from;
    };
    if current == '\n' {
        return doc.char_to_pos(index + 1);
    }
    if current.is_whitespace() {
        while matches!(char_at(doc, index), Some(ch) if ch.is_whitespace() && ch != '\n') {
            index += 1;
        }
    } else {
        let class = Class::of(current);
        while index < len && class_at(doc, index) == Some(class) {
            index += 1;
        }
    }
    doc.char_to_pos(index.min(len))
}
