//! # Folds
//!
//! **Purpose:** hide a block of lines behind its first one.
//!
//! **Responsibility:** work out, from indentation alone, which lines belong to
//! the block a line opens — every following line indented deeper than it —
//! and answer the questions rendering and movement ask about a set of closed
//! folds: is this line hidden, and what is the next line that is not.
//!
//! Folding by indentation needs no grammar, so it works the same in every
//! language and in plain text, which is what a regex-era editor can promise.
//!
//! **Public API:** [`range_at`], [`enclosing`], [`closed_ranges`],
//! [`hidden_by`], [`visible_after`].

use std::collections::BTreeSet;

use crate::editor::document::Document;

/// How many lines `zM` looks at before giving up on finding more folds.
const CLOSE_ALL_LIMIT: usize = 50_000;

fn indent(doc: &Document, line: usize, tab_width: usize) -> Option<usize> {
    let mut width = 0;
    for ch in doc.line(line).chars() {
        match ch {
            ' ' => width += 1,
            '\t' => width += tab_width,
            '\n' | '\r' => return None,
            _ => return Some(width),
        }
    }
    None
}

/// The fold `line` opens: itself and every line after it indented deeper,
/// blank lines inside included, blank lines after it not.
#[must_use]
pub fn range_at(doc: &Document, line: usize, tab_width: usize) -> Option<(usize, usize)> {
    let own = indent(doc, line, tab_width)?;
    let mut end = line;
    for next in line + 1..doc.len_lines() {
        match indent(doc, next, tab_width) {
            None => {}
            Some(deeper) if deeper > own => end = next,
            Some(_) => break,
        }
    }
    (end > line).then_some((line, end))
}

/// The innermost fold containing `line`: the one it opens, or the one an
/// enclosing, shallower line opens.
#[must_use]
pub fn enclosing(doc: &Document, line: usize, tab_width: usize) -> Option<(usize, usize)> {
    if let Some(range) = range_at(doc, line, tab_width) {
        return Some(range);
    }
    let own = (line..doc.len_lines())
        .find_map(|next| indent(doc, next, tab_width))
        .unwrap_or(0);
    (0..line).rev().find_map(|above| {
        let width = indent(doc, above, tab_width)?;
        if width >= own {
            return None;
        }
        range_at(doc, above, tab_width).filter(|(_, end)| *end >= line)
    })
}

/// The spans hidden by a set of closed fold starts, outermost first and
/// without the ones nested inside another closed fold.
#[must_use]
pub fn closed_ranges(
    doc: &Document,
    starts: &BTreeSet<usize>,
    tab_width: usize,
) -> Vec<(usize, usize)> {
    let mut ranges: Vec<(usize, usize)> = Vec::new();
    for &start in starts {
        if ranges.last().is_some_and(|(_, end)| start <= *end) {
            continue;
        }
        if let Some(range) = range_at(doc, start, tab_width) {
            ranges.push(range);
        }
    }
    ranges
}

/// Every line that opens a fold, for `zM`.
#[must_use]
pub fn all_starts(doc: &Document, tab_width: usize) -> BTreeSet<usize> {
    (0..doc.len_lines().min(CLOSE_ALL_LIMIT))
        .filter(|line| range_at(doc, *line, tab_width).is_some())
        .collect()
}

/// The closed fold hiding `line`, if one does. A fold's own first line is
/// never hidden — it is what stands in for the rest.
#[must_use]
pub fn hidden_by(ranges: &[(usize, usize)], line: usize) -> Option<(usize, usize)> {
    ranges
        .iter()
        .copied()
        .find(|(start, end)| line > *start && line <= *end)
}

/// The closed fold starting at `line`, if one does.
#[must_use]
pub fn starting_at(ranges: &[(usize, usize)], line: usize) -> Option<(usize, usize)> {
    ranges.iter().copied().find(|(start, _)| *start == line)
}

/// The first visible line after `line`.
#[must_use]
pub fn visible_after(ranges: &[(usize, usize)], line: usize) -> usize {
    match starting_at(ranges, line) {
        Some((_, end)) => end + 1,
        None => line + 1,
    }
}

/// How many screen lines lie between `from` and `to`, counting each closed
/// fold as one.
#[must_use]
pub fn visible_between(ranges: &[(usize, usize)], from: usize, to: usize) -> usize {
    let mut count = 0;
    let mut line = from;
    while line < to {
        line = visible_after(ranges, line);
        count += 1;
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc() -> Document {
        Document::from_text(
            "fn a() {\n    one\n\n    if x {\n        two\n    }\n}\nfn b() {}\n",
            None,
        )
    }

    #[test]
    fn a_fold_is_everything_indented_under_its_line() {
        let document = doc();
        assert_eq!(range_at(&document, 0, 4), Some((0, 5)));
        assert_eq!(range_at(&document, 3, 4), Some((3, 4)));
        assert_eq!(range_at(&document, 1, 4), None);
        assert_eq!(enclosing(&document, 1, 4), Some((0, 5)));
        assert_eq!(enclosing(&document, 4, 4), Some((3, 4)));
    }

    #[test]
    fn closed_folds_hide_lines_and_count_as_one() {
        let document = doc();
        let starts: BTreeSet<usize> = [0, 3].into_iter().collect();
        let ranges = closed_ranges(&document, &starts, 4);
        assert_eq!(
            ranges,
            vec![(0, 5)],
            "the nested fold is inside the outer one"
        );
        assert_eq!(hidden_by(&ranges, 2), Some((0, 5)));
        assert_eq!(hidden_by(&ranges, 0), None);
        assert_eq!(visible_after(&ranges, 0), 6);
        assert_eq!(visible_between(&ranges, 0, 7), 2);
    }
}
