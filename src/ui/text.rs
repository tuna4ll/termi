//! # Display layout
//!
//! **Purpose:** translate a line of text into terminal cells.
//!
//! **Responsibility:** the one place that knows a tab is not one column wide and
//! that a CJK glyph is two. Everything above works in *character* indices;
//! everything drawn works in *display columns*; this module is the bridge, and
//! keeping it in one file is what stops off-by-one column bugs from spreading.
//!
//! **Public API:** [`DisplayLine`], [`DisplayCell`].

use unicode_width::UnicodeWidthChar;

/// One terminal cell of a rendered line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DisplayCell {
    /// Glyph to draw, or `None` for the trailing half of a double-width glyph.
    pub glyph: Option<char>,
    /// Index of the source character this cell came from.
    ///
    /// Syntax spans, selections and search matches are all expressed in
    /// character indices, so carrying it here makes styling a lookup rather than
    /// a second pass of column arithmetic.
    pub char_index: usize,
}

/// A line expanded into display cells.
#[derive(Debug, Clone, Default)]
pub struct DisplayLine {
    /// One entry per terminal column.
    pub cells: Vec<DisplayCell>,
    /// Display column at which each source character starts.
    ///
    /// Has one extra entry for the position just past the last character, which
    /// is where the caret sits at end of line.
    pub char_columns: Vec<usize>,
}

impl DisplayLine {
    /// Expand `line`, turning tabs into padding up to the next tab stop.
    #[must_use]
    pub fn new(line: &str, tab_width: usize) -> Self {
        let tab_width = tab_width.max(1);
        let mut cells = Vec::with_capacity(line.len());
        let mut char_columns = Vec::with_capacity(line.len() + 1);

        for (char_index, ch) in line.chars().enumerate() {
            char_columns.push(cells.len());
            match ch {
                '\t' => {
                    // A tab advances to the next multiple of `tab_width`, so its
                    // width depends on where it starts.
                    let stop = tab_width - (cells.len() % tab_width);
                    for _ in 0..stop {
                        cells.push(DisplayCell {
                            glyph: Some(' '),
                            char_index,
                        });
                    }
                }
                _ => {
                    // Control characters have no width of their own; showing a
                    // placeholder beats silently misaligning the rest of the line.
                    let (glyph, width) = match ch.width() {
                        Some(0) | None => ('\u{00b7}', 1),
                        Some(width) => (ch, width),
                    };
                    cells.push(DisplayCell {
                        glyph: Some(glyph),
                        char_index,
                    });
                    for _ in 1..width {
                        cells.push(DisplayCell {
                            glyph: None,
                            char_index,
                        });
                    }
                }
            }
        }
        char_columns.push(cells.len());

        Self {
            cells,
            char_columns,
        }
    }

    /// Total width of the line in columns.
    #[must_use]
    pub fn width(&self) -> usize {
        self.cells.len()
    }

    /// Display column where character `char_index` starts.
    ///
    /// Indices past the end clamp to the end of the line, which is what a caret
    /// resting past the last character needs.
    #[must_use]
    pub fn column_of(&self, char_index: usize) -> usize {
        self.char_columns
            .get(char_index)
            .copied()
            .unwrap_or_else(|| self.width())
    }

    /// Source character at display column `column`.
    ///
    /// The inverse of [`DisplayLine::column_of`], and what turns a mouse click
    /// into a caret position. A column past the end of the line gives the
    /// position just after the last character, which is where a click in the
    /// empty space to the right of a line belongs. The trailing half of a wide
    /// glyph reports the glyph itself, so clicking either of its columns picks
    /// the same character.
    #[must_use]
    pub fn char_at(&self, column: usize) -> usize {
        self.cells
            .get(column)
            .map_or(self.char_columns.len() - 1, |cell| cell.char_index)
    }

    /// Split the line into visual rows no wider than `width`.
    ///
    /// Returns half-open cell ranges. Breaks happen after the last space that
    /// fits; a single word longer than the window is broken hard, because
    /// refusing to break it would push it off screen entirely.
    ///
    /// An empty line still yields one row, so a blank line occupies a row like
    /// any other.
    #[must_use]
    pub fn wrap(&self, width: usize) -> Vec<(usize, usize)> {
        if width == 0 || self.cells.len() <= width {
            return vec![(0, self.cells.len())];
        }
        let mut rows = Vec::new();
        let mut start = 0;

        while start < self.cells.len() {
            let limit = start + width;
            if limit >= self.cells.len() {
                rows.push((start, self.cells.len()));
                break;
            }
            // Break *after* the space so trailing spaces stay on the row above,
            // which is what keeps the next row starting on a word.
            let end = self.cells[start..limit]
                .iter()
                .rposition(|cell| cell.glyph == Some(' '))
                .map(|offset| start + offset + 1)
                .filter(|&candidate| candidate > start)
                .unwrap_or(limit);
            rows.push((start, end));
            start = end;
        }
        rows
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn glyphs(line: &DisplayLine) -> String {
        line.cells.iter().filter_map(|c| c.glyph).collect()
    }

    #[test]
    fn tabs_advance_to_the_next_tab_stop() {
        let line = DisplayLine::new("\tx", 4);
        assert_eq!(line.width(), 5);
        assert_eq!(glyphs(&line), "    x");
        assert_eq!(line.column_of(1), 4);
    }

    #[test]
    fn a_tab_after_text_fills_only_the_remaining_columns() {
        let line = DisplayLine::new("ab\tc", 4);
        // "ab" occupies 0..2, so the tab only needs two columns to reach 4.
        assert_eq!(line.column_of(3), 4);
        assert_eq!(line.width(), 5);
    }

    #[test]
    fn wide_glyphs_occupy_two_columns() {
        let line = DisplayLine::new("a漢b", 4);
        assert_eq!(line.width(), 4);
        assert_eq!(line.column_of(2), 3);
        // The second column of the wide glyph is a continuation cell.
        assert_eq!(line.cells[2].glyph, None);
        assert_eq!(line.cells[2].char_index, 1);
    }

    #[test]
    fn control_characters_get_a_visible_placeholder() {
        let line = DisplayLine::new("a\u{7}b", 4);
        assert_eq!(glyphs(&line), "a\u{00b7}b");
    }

    #[test]
    fn the_caret_position_past_the_end_is_addressable() {
        let line = DisplayLine::new("abc", 4);
        assert_eq!(line.column_of(3), 3);
        assert_eq!(line.column_of(99), 3);
    }

    #[test]
    fn a_column_maps_back_to_the_character_that_drew_it() {
        let line = DisplayLine::new("ab\tc", 4);
        assert_eq!(line.char_at(0), 0);
        // Every column of the tab belongs to the tab.
        assert_eq!(line.char_at(2), 2);
        assert_eq!(line.char_at(3), 2);
        assert_eq!(line.char_at(4), 3);
    }

    #[test]
    fn both_columns_of_a_wide_glyph_pick_the_same_character() {
        let line = DisplayLine::new("a漢b", 4);
        assert_eq!(line.char_at(1), 1);
        assert_eq!(line.char_at(2), 1);
        assert_eq!(line.char_at(3), 2);
    }

    #[test]
    fn a_column_past_the_end_lands_after_the_last_character() {
        let line = DisplayLine::new("abc", 4);
        assert_eq!(line.char_at(99), 3);
        assert_eq!(DisplayLine::new("", 4).char_at(5), 0);
    }

    #[test]
    fn a_short_line_wraps_to_a_single_row() {
        let line = DisplayLine::new("short", 4);
        assert_eq!(line.wrap(20), vec![(0, 5)]);
    }

    #[test]
    fn an_empty_line_still_occupies_one_row() {
        assert_eq!(DisplayLine::new("", 4).wrap(20), vec![(0, 0)]);
    }

    #[test]
    fn wrapping_breaks_after_a_space() {
        let line = DisplayLine::new("aaa bbb ccc", 4);
        assert_eq!(line.wrap(8), vec![(0, 8), (8, 11)]);
    }

    #[test]
    fn a_word_longer_than_the_window_is_broken_hard() {
        let line = DisplayLine::new("aaaaaaaaaa", 4);
        assert_eq!(line.wrap(4), vec![(0, 4), (4, 8), (8, 10)]);
    }

    #[test]
    fn wrapping_covers_every_cell_exactly_once() {
        let line = DisplayLine::new("the quick brown fox jumps over it", 4);
        let rows = line.wrap(10);
        assert_eq!(rows.first().map(|r| r.0), Some(0));
        assert_eq!(rows.last().map(|r| r.1), Some(line.width()));
        for pair in rows.windows(2) {
            assert_eq!(pair[0].1, pair[1].0);
        }
    }
}
