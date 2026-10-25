//! # Comments
//!
//! **Purpose:** answer "what does toggling comments on these lines change?".
//!
//! **Responsibility:** a pure function from a document, a line range and the
//! language's comment tokens to a list of replacements. Applying them is the
//! editing layer's job, which keeps undo in one place.
//!
//! The rule is the one most editors settled on: if every non-blank line is
//! already commented, uncomment them all; otherwise comment them all, with the
//! marker at the shallowest indentation so the block keeps its shape.
//!
//! **Public API:** [`Tokens`], [`toggle`].

use crate::editor::document::Document;
use crate::editor::selection::Range;

/// The comment syntax of a language.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Tokens<'a> {
    /// The marker that comments out the rest of a line, such as `//`.
    pub line: Option<&'a str>,
    /// Markers around a block comment, such as `/*` and `*/`, used when the
    /// language has no line comment.
    pub block: Option<(&'a str, &'a str)>,
}

/// The replacements, in document order, that toggle comments on
/// `first..=last`. Empty when the language has no comment syntax.
#[must_use]
pub fn toggle(
    doc: &Document,
    first: usize,
    last: usize,
    tokens: Tokens<'_>,
) -> Vec<(Range, String)> {
    let lines: Vec<(usize, String)> = (first..=last.min(doc.last_line()))
        .map(|line| (line, doc.line_string(line)))
        .filter(|(_, text)| !text.trim().is_empty())
        .collect();
    if lines.is_empty() {
        return Vec::new();
    }
    if let Some(marker) = tokens.line {
        return toggle_line(doc, &lines, marker);
    }
    if let Some((open, close)) = tokens.block {
        return toggle_block(doc, &lines, open, close);
    }
    Vec::new()
}

fn indent_width(text: &str) -> usize {
    text.chars()
        .take_while(|ch| *ch == ' ' || *ch == '\t')
        .count()
}

fn toggle_line(doc: &Document, lines: &[(usize, String)], marker: &str) -> Vec<(Range, String)> {
    let commented = lines
        .iter()
        .all(|(_, text)| text.trim_start().starts_with(marker));
    let marker_len = marker.chars().count();

    if commented {
        return lines
            .iter()
            .map(|(line, text)| {
                let indent = indent_width(text);
                let after: String = text.chars().skip(indent + marker_len).collect();
                let remove = marker_len + usize::from(after.starts_with(' '));
                let start = doc.line_start(*line) + indent;
                (Range::new(start, start + remove), String::new())
            })
            .collect();
    }
    let column = lines
        .iter()
        .map(|(_, text)| indent_width(text))
        .min()
        .unwrap_or(0);
    lines
        .iter()
        .map(|(line, _)| {
            let at = doc.line_start(*line) + column;
            (Range::new(at, at), format!("{marker} "))
        })
        .collect()
}

fn toggle_block(
    doc: &Document,
    lines: &[(usize, String)],
    open: &str,
    close: &str,
) -> Vec<(Range, String)> {
    let commented = lines.iter().all(|(_, text)| {
        let trimmed = text.trim();
        trimmed.starts_with(open)
            && trimmed.ends_with(close)
            && trimmed.len() >= open.len() + close.len()
    });
    let mut edits = Vec::new();
    for (line, text) in lines {
        let start = doc.line_start(*line);
        let indent = indent_width(text);
        let content_end = text.trim_end().chars().count();
        if commented {
            let inner: String = text
                .chars()
                .skip(indent)
                .take(content_end - indent)
                .collect();
            let body = inner
                .strip_prefix(open)
                .and_then(|rest| rest.strip_suffix(close))
                .unwrap_or(&inner);
            let body = body.strip_prefix(' ').unwrap_or(body);
            let body = body.strip_suffix(' ').unwrap_or(body);
            edits.push((
                Range::new(start + indent, start + content_end),
                body.to_string(),
            ));
        } else {
            let inner: String = text
                .chars()
                .skip(indent)
                .take(content_end - indent)
                .collect();
            edits.push((
                Range::new(start + indent, start + content_end),
                format!("{open} {inner} {close}"),
            ));
        }
    }
    edits
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apply(text: &str, first: usize, last: usize, tokens: Tokens<'_>) -> String {
        let mut doc = Document::from_text(text, None);
        for (range, replacement) in toggle(&doc, first, last, tokens).into_iter().rev() {
            doc.remove(range.start, range.end);
            doc.insert(range.start, &replacement);
        }
        doc.text().to_string()
    }

    const SLASHES: Tokens<'static> = Tokens {
        line: Some("//"),
        block: Some(("/*", "*/")),
    };

    #[test]
    fn comments_at_the_shallowest_indent_and_skips_blank_lines() {
        assert_eq!(
            apply("    a\n\n  b\n", 0, 2, SLASHES),
            "  //   a\n\n  // b\n"
        );
    }

    #[test]
    fn a_fully_commented_block_is_uncommented() {
        assert_eq!(apply("  // a\n  //b\n", 0, 1, SLASHES), "  a\n  b\n");
    }

    #[test]
    fn a_partly_commented_block_is_commented_again() {
        assert_eq!(apply("// a\nb\n", 0, 1, SLASHES), "// // a\n// b\n");
    }

    #[test]
    fn languages_without_line_comments_wrap_each_line() {
        let html = Tokens {
            line: None,
            block: Some(("<!--", "-->")),
        };
        assert_eq!(apply("  <p>x</p>\n", 0, 0, html), "  <!-- <p>x</p> -->\n");
        assert_eq!(apply("  <!-- <p>x</p> -->\n", 0, 0, html), "  <p>x</p>\n");
    }
}
