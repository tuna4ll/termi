//! # Text objects
//!
//! **Purpose:** answer "which span is *this thing* the caret is in?" — the
//! word, the quoted string, the bracketed block, the paragraph, the tag.
//!
//! **Responsibility:** pure functions from a document and a caret position to
//! a [`Range`]. The operators decide what to do with the span; this module only
//! finds it, which is what lets `diw`, `ci(`, `ya"` and `vip` share one
//! implementation each.
//!
//! **Public API:** [`TextObject`], [`select`].

use crate::editor::cursor::scan::{self, is_blank_line};
use crate::editor::cursor::{Position, word};
use crate::editor::document::Document;
use crate::editor::selection::Range;

/// The thing a text object names, independent of `i` or `a`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextObject {
    /// `w` — a run of keyword characters, or of punctuation, or of blanks.
    Word,
    /// `W` — a run of anything but blanks.
    BigWord,
    /// `p` — lines up to the next blank line.
    Paragraph,
    /// `(`, `[`, `{`, `<` and their closing halves — a bracketed block.
    Pair(char, char),
    /// `"`, `'` and `` ` `` — a quoted string on the current line.
    Quote(char),
    /// `t` — an XML or HTML element.
    Tag,
}

impl TextObject {
    /// The object a key names after `i` or `a`.
    #[must_use]
    pub const fn from_key(key: char) -> Option<Self> {
        Some(match key {
            'w' => Self::Word,
            'W' => Self::BigWord,
            'p' => Self::Paragraph,
            '(' | ')' | 'b' => Self::Pair('(', ')'),
            '[' | ']' => Self::Pair('[', ']'),
            '{' | '}' | 'B' => Self::Pair('{', '}'),
            '<' | '>' => Self::Pair('<', '>'),
            '"' | '\'' | '`' => Self::Quote(key),
            't' => Self::Tag,
            _ => return None,
        })
    }
}

/// A span found by [`select`], and whether it covers whole lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Selection {
    /// The characters covered.
    pub range: Range,
    /// Whether the span is made of whole lines, as `ip` and `ap` are.
    pub linewise: bool,
}

impl Selection {
    const fn chars(range: Range) -> Self {
        Self {
            range,
            linewise: false,
        }
    }
}

/// The span of `object` around `at`; `inner` is `i`, otherwise `a`.
///
/// `count` widens bracket objects outwards and extends words and paragraphs
/// forwards, the way `2di(` and `d3aw` do.
#[must_use]
pub fn select(
    doc: &Document,
    at: Position,
    object: TextObject,
    inner: bool,
    count: usize,
) -> Option<Selection> {
    let count = count.max(1);
    match object {
        TextObject::Word => words(doc, at, inner, count, false),
        TextObject::BigWord => words(doc, at, inner, count, true),
        TextObject::Paragraph => paragraph(doc, at, inner, count),
        TextObject::Pair(open, close) => pair(doc, at, open, close, inner, count),
        TextObject::Quote(quote) => quoted(doc, at, quote, inner),
        TextObject::Tag => tag(doc, at, inner),
    }
}

fn words(doc: &Document, at: Position, inner: bool, count: usize, big: bool) -> Option<Selection> {
    let index = doc.pos_to_char(at);
    let (mut start, mut end) = word::run_at(doc, index, big)?;
    let started_on_blank = word::is_blank_at(doc, index);

    for round in 0..count {
        if round > 0 {
            let (_, next) = word::run_at(doc, end, big)?;
            end = next;
        }
        if inner {
            continue;
        }
        // `aw` takes the blanks after the word, or — on a word followed by
        // nothing — the ones before it, so deleting it leaves one gap behind
        // rather than two words glued together.
        if started_on_blank {
            if let Some((_, next)) = word::run_at(doc, end, big) {
                end = next;
            }
        } else if word::is_blank_at(doc, end) {
            let (_, next) = word::run_at(doc, end, big)?;
            end = next;
        } else if round == 0 && start > 0 && word::is_blank_at(doc, start - 1) {
            let (previous, _) = word::run_at(doc, start - 1, big)?;
            start = previous;
        }
    }
    Some(Selection::chars(Range::new(start, end)))
}

fn paragraph(doc: &Document, at: Position, inner: bool, count: usize) -> Option<Selection> {
    let last = doc.last_line();
    let blank = is_blank_line(doc, at.line);
    let mut first = at.line;
    while first > 0 && is_blank_line(doc, first - 1) == blank {
        first -= 1;
    }
    let mut end = at.line;
    let mut kind = blank;
    let mut rounds = if inner { count } else { count * 2 };
    loop {
        while end < last && is_blank_line(doc, end + 1) == kind {
            end += 1;
        }
        rounds -= 1;
        if rounds == 0 || end >= last {
            break;
        }
        end += 1;
        kind = !kind;
    }
    let start = doc.line_start(first);
    let stop = if end >= last {
        doc.len_chars()
    } else {
        doc.line_start(end + 1)
    };
    Some(Selection {
        range: Range::new(start, stop),
        linewise: true,
    })
}

/// The unmatched `open` at or before `index`, skipping over balanced pairs.
fn enclosing_open(doc: &Document, index: usize, open: char, close: char) -> Option<usize> {
    let text = doc.text();
    if text.get_char(index) == Some(open) {
        return Some(index);
    }
    let mut depth = 0usize;
    let mut at = index;
    let mut chars = text.chars_at(index);
    // A caret on the closing bracket is inside the pair it closes.
    if text.get_char(index) == Some(close) {
        return scan::matching_bracket(doc, index);
    }
    while at > 0 {
        let ch = chars.prev()?;
        at -= 1;
        if ch == close {
            depth += 1;
        } else if ch == open {
            if depth == 0 {
                return Some(at);
            }
            depth -= 1;
        }
    }
    None
}

fn pair(
    doc: &Document,
    at: Position,
    open: char,
    close: char,
    inner: bool,
    count: usize,
) -> Option<Selection> {
    let mut index = doc.pos_to_char(at);
    let mut start = enclosing_open(doc, index, open, close)?;
    for _ in 1..count {
        index = start.checked_sub(1)?;
        start = enclosing_open(doc, index, open, close)?;
    }
    let end = scan::matching_bracket(doc, start)?;
    if !inner {
        return Some(Selection::chars(Range::new(start, end + 1)));
    }

    let text = doc.text();
    let mut inner_start = start + 1;
    let mut inner_end = end;
    // A block spread over several lines keeps its bracket lines: `di{` empties
    // the body but leaves `{` and `}` where they were.
    if text.get_char(inner_start) == Some('\n') {
        inner_start += 1;
        let close_line = doc.char_to_pos(end).line;
        let close_line_start = doc.line_start(close_line);
        if close_line_start > inner_start
            && doc.slice_string(close_line_start, end).trim().is_empty()
        {
            inner_end = close_line_start;
        }
    }
    Some(Selection::chars(Range::new(
        inner_start,
        inner_end.max(inner_start),
    )))
}

fn quoted(doc: &Document, at: Position, quote: char, inner: bool) -> Option<Selection> {
    let chars: Vec<char> = doc.line_string(at.line).chars().collect();
    let mut quotes = Vec::new();
    let mut escaped = false;
    for (col, ch) in chars.iter().enumerate() {
        if escaped {
            escaped = false;
        } else if *ch == '\\' {
            escaped = true;
        } else if *ch == quote {
            quotes.push(col);
        }
    }
    // Pair the quotes up from the start of the line, then take the pair around
    // the caret, or failing that the first one after it.
    let pairs: Vec<(usize, usize)> = quotes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|[open, close]| (*open, *close))
        .collect();
    let (open, close) = pairs
        .iter()
        .copied()
        .find(|(open, close)| at.col >= *open && at.col <= *close)
        .or_else(|| pairs.iter().copied().find(|(open, _)| *open > at.col))?;

    let line_start = doc.line_start(at.line);
    if inner {
        return Some(Selection::chars(Range::new(
            line_start + open + 1,
            line_start + close,
        )));
    }
    let mut end = close + 1;
    while end < chars.len() && chars[end].is_whitespace() {
        end += 1;
    }
    let mut start = open;
    if end == close + 1 {
        while start > 0 && chars[start - 1].is_whitespace() {
            start -= 1;
        }
    }
    Some(Selection::chars(Range::new(
        line_start + start,
        line_start + end,
    )))
}

/// One `<name …>` or `</name>` found while scanning for tags.
struct TagMark {
    start: usize,
    end: usize,
    name: String,
    closing: bool,
}

fn tag(doc: &Document, at: Position, inner: bool) -> Option<Selection> {
    let text = doc.text().to_string();
    let caret = doc.pos_to_char(at);
    let pattern = regex::Regex::new(r"<(/?)([A-Za-z][\w:.-]*)[^<>]*?(/?)>").ok()?;

    // Byte offsets from the regex, converted to characters once at the end.
    let mut stack: Vec<TagMark> = Vec::new();
    let mut best: Option<(usize, usize, usize, usize)> = None;
    for found in pattern.captures_iter(&text) {
        let whole = found.get(0)?;
        if found.get(3).is_some_and(|m| !m.as_str().is_empty()) {
            continue;
        }
        let mark = TagMark {
            start: whole.start(),
            end: whole.end(),
            name: found.get(2)?.as_str().to_string(),
            closing: !found.get(1)?.as_str().is_empty(),
        };
        if !mark.closing {
            stack.push(mark);
            continue;
        }
        let Some(position) = stack.iter().rposition(|open| open.name == mark.name) else {
            continue;
        };
        let open = stack.remove(position);
        stack.truncate(position);
        let (outer_start, outer_end) = (char_index(&text, open.start), char_index(&text, mark.end));
        if caret >= outer_start && caret < outer_end {
            // The innermost element containing the caret is the smallest one.
            let span = (
                outer_start,
                char_index(&text, open.end),
                char_index(&text, mark.start),
                outer_end,
            );
            if best.is_none_or(|current| span.3 - span.0 < current.3 - current.0) {
                best = Some(span);
            }
        }
    }
    let (outer_start, inner_start, inner_end, outer_end) = best?;
    Some(Selection::chars(if inner {
        Range::new(inner_start, inner_end)
    } else {
        Range::new(outer_start, outer_end)
    }))
}

fn char_index(text: &str, byte: usize) -> usize {
    text[..byte].chars().count()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pick(text: &str, col: usize, object: TextObject, inner: bool) -> String {
        let document = Document::from_text(text, None);
        let found = select(&document, document.char_to_pos(col), object, inner, 1)
            .expect("object should be found");
        document.slice_string(found.range.start, found.range.end)
    }

    #[test]
    fn words_inner_and_around() {
        assert_eq!(pick("one two three", 5, TextObject::Word, true), "two");
        assert_eq!(pick("one two three", 5, TextObject::Word, false), "two ");
        // The last word takes the blanks before it instead.
        assert_eq!(pick("one two", 5, TextObject::Word, false), " two");
        assert_eq!(pick("foo.bar baz", 1, TextObject::BigWord, true), "foo.bar");
    }

    #[test]
    fn brackets_nest_and_span_lines() {
        assert_eq!(
            pick("f(a, (b), c)", 3, TextObject::Pair('(', ')'), true),
            "a, (b), c"
        );
        assert_eq!(
            pick("f(a, (b), c)", 6, TextObject::Pair('(', ')'), false),
            "(b)"
        );
        assert_eq!(
            pick("{\n    body\n}", 6, TextObject::Pair('{', '}'), true),
            "    body\n"
        );
    }

    #[test]
    fn quotes_pair_up_from_the_start_of_the_line() {
        assert_eq!(
            pick(r#"say "hi \"you\"" now"#, 6, TextObject::Quote('"'), true),
            r#"hi \"you\""#
        );
        assert_eq!(
            pick(r#"a "b" c"#, 0, TextObject::Quote('"'), false),
            "\"b\" "
        );
    }

    #[test]
    fn paragraphs_cover_whole_lines() {
        let document = Document::from_text("a\nb\n\nc\n", None);
        let found = select(
            &document,
            Position::new(0, 0),
            TextObject::Paragraph,
            false,
            1,
        )
        .expect("paragraph");
        assert!(found.linewise);
        assert_eq!(
            document.slice_string(found.range.start, found.range.end),
            "a\nb\n\n"
        );
    }

    #[test]
    fn tags_find_the_innermost_element() {
        let text = "<div><p>hi</p></div>";
        assert_eq!(pick(text, 9, TextObject::Tag, true), "hi");
        assert_eq!(pick(text, 9, TextObject::Tag, false), "<p>hi</p>");
        assert_eq!(pick(text, 2, TextObject::Tag, true), "<p>hi</p>");
    }
}
