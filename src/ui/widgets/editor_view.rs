//! # Editor widget
//!
//! **Purpose:** draw the text.
//!
//! **Responsibility:** the line-number gutter, the visible slice of the
//! document, the current-line highlight and the selection. It writes cells
//! directly instead of building `Line`/`Span` values, because a full-screen
//! redraw touches every cell every frame and the intermediate allocations are
//! the one thing that shows up on large files.
//!
//! **Public API:** [`EditorView`], [`gutter_width`], [`scroll_into_view`],
//! [`position_at`].

use ratatui::buffer::Buffer as Surface;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::widgets::Widget;

use crate::config::Config;
use crate::editor::buffer::Buffer;
use crate::editor::cursor::Position;
use crate::editor::selection::Range;
use crate::editor::window::{Area, Window};
use crate::search::{LineMatch, Search};
use crate::syntax::Highlight;
use crate::theme::Theme;
use crate::ui::text::DisplayLine;

/// Columns reserved for the line-number gutter, including its trailing space.
///
/// Returns `0` when line numbers are off, which makes the text area logic
/// uniform — there is no special case for "no gutter".
#[must_use]
pub fn gutter_width(config: &Config, line_count: usize) -> u16 {
    if !config.line_numbers {
        return 0;
    }
    let digits = line_count.max(1).ilog10() as u16 + 1;
    digits.max(3) + 2
}

/// Scroll a window so its primary caret is on screen.
///
/// Lives here rather than in `View` because the amount of usable width depends
/// on the gutter, and the caret's *display* column depends on tab expansion —
/// both of which are rendering concerns. The buffer comes in separately because
/// scrolling changes the window and only reads the text.
pub fn scroll_into_view(buffer: &Buffer, window: &mut Window, config: &Config, area: Rect) {
    let gutter = gutter_width(config, buffer.document.len_lines());
    let width = usize::from(area.width.saturating_sub(gutter));
    let height = usize::from(area.height);
    let head = window.cursor().head;

    if !config.word_wrap {
        let line = DisplayLine::new(&buffer.document.line_string(head.line), config.tab_width);
        window.view.scroll_to(
            head.line,
            line.column_of(head.col),
            height,
            width,
            config.scrolloff,
        );
        return;
    }

    // With wrapping, a line is worth an unknown number of rows, so "does the
    // caret fit?" has to be measured instead of computed. Walking the top of the
    // view forward one line at a time is bounded by the window height.
    window.view.left_col = 0;
    if head.line < window.view.top_line {
        window.view.top_line = head.line;
    }
    if width == 0 || height == 0 {
        return;
    }
    let rows_of = |line: usize| {
        DisplayLine::new(&buffer.document.line_string(line), config.tab_width)
            .wrap(width)
            .len()
    };
    while window.view.top_line < head.line {
        let rows: usize = (window.view.top_line..=head.line).map(rows_of).sum();
        if rows <= height {
            break;
        }
        window.view.top_line += 1;
    }
}

/// The document position under a screen cell, or `None` when the cell is not
/// inside `area`.
///
/// The inverse of [`EditorView::caret_position`], and what lets the mouse place
/// the caret at all: the layers above work in character indices, and only the
/// renderer knows how many columns a tab or a wide glyph took. The position is
/// returned unclamped — whether the caret may rest past the last character is a
/// question about the mode, which this layer does not know about.
///
/// A click in the gutter lands on the leftmost visible character of the line
/// beside it, and one below the last line lands on the last line.
#[must_use]
pub fn position_at(
    buffer: &Buffer,
    window: &Window,
    config: &Config,
    area: Area,
    x: u16,
    y: u16,
) -> Option<Position> {
    if !area.contains(x, y) {
        return None;
    }
    let gutter = gutter_width(config, buffer.document.len_lines());
    let width = usize::from(area.width.saturating_sub(gutter));
    let column = usize::from(x.saturating_sub(area.x + gutter));
    let row = usize::from(y - area.y);

    let (line, cell) = if config.word_wrap {
        wrapped_cell(buffer, window, config, width, row, column)?
    } else {
        (window.view.top_line + row, window.view.left_col + column)
    };
    let line = line.min(buffer.document.last_line());
    let display = DisplayLine::new(&buffer.document.line_string(line), config.tab_width);
    Some(Position::new(line, display.char_at(cell)))
}

/// Which line, and which cell within it, a screen row addresses when wrapping
/// is on.
///
/// Rows are counted rather than subtracted, because a line is worth as many of
/// them as it needs. A column past the end of a wrapped chunk stays on that
/// chunk instead of running on into the one drawn below it.
fn wrapped_cell(
    buffer: &Buffer,
    window: &Window,
    config: &Config,
    width: usize,
    row: usize,
    column: usize,
) -> Option<(usize, usize)> {
    if width == 0 {
        return None;
    }
    let mut remaining = row;
    let mut line = window.view.top_line;

    while line <= buffer.document.last_line() {
        let display = DisplayLine::new(&buffer.document.line_string(line), config.tab_width);
        let chunks = display.wrap(width);
        if let Some(&(start, end)) = chunks.get(remaining) {
            // The final chunk runs to the end of the line, where the caret may
            // legitimately sit past the last character; an earlier one ends at a
            // character that is drawn on the next row.
            let last = if end == display.width() { end } else { end - 1 };
            return Some((line, (start + column).min(last)));
        }
        remaining -= chunks.len();
        line += 1;
    }
    // Below the last line: the end of the document is the nearest text there is.
    Some((buffer.document.last_line(), usize::MAX))
}

/// One screen row's slice of a document line.
///
/// Grouped into a struct because the row, the line and the cell range always
/// travel together, and passing them separately made the drawing routine's
/// signature hard to read.
#[derive(Debug, Clone, Copy)]
struct Row {
    /// Terminal row to draw on.
    y: u16,
    /// Document line being drawn.
    line: usize,
    /// Half-open range of display cells this row shows.
    cells: (usize, usize),
}

/// The text area and its gutter.
pub struct EditorView<'a> {
    /// Buffer to draw.
    pub buffer: &'a Buffer,
    /// The window looking at it, which supplies the scroll offset and cursors.
    pub window: &'a Window,
    /// Whether this window has the keyboard, so the caret line is only
    /// highlighted where typing would land.
    pub focused: bool,
    /// Colours.
    pub theme: &'a Theme,
    /// Tab width, gutter and highlight settings.
    pub config: &'a Config,
    /// Span to paint with the selection style, if any.
    pub selection: Option<Range>,
    /// Active search, used to highlight matches on the visible lines only.
    pub search: Option<&'a Search>,
    /// The match the caret is sitting on, painted more strongly than the rest.
    pub active_match: Option<Range>,
}

impl EditorView<'_> {
    /// Screen position of the primary caret, or `None` when it is scrolled out
    /// of view.
    #[must_use]
    pub fn caret_position(&self, area: Rect) -> Option<(u16, u16)> {
        let head = self.window.cursor().head;
        let view = self.window.view;
        let gutter = gutter_width(self.config, self.buffer.document.len_lines());
        let width = usize::from(area.width.saturating_sub(gutter));
        let height = usize::from(area.height);

        let display = DisplayLine::new(
            &self.buffer.document.line_string(head.line),
            self.config.tab_width,
        );
        let target = display.column_of(head.col);

        let (row, column) = if self.config.word_wrap {
            // Every line above the caret may occupy several rows, so the row has
            // to be counted rather than subtracted.
            let mut row = 0usize;
            for line in view.top_line..head.line {
                row += self.rows_of(line, width).1.len();
                if row >= height {
                    return None;
                }
            }
            let chunks = display.wrap(width);
            let index = chunks
                .iter()
                .position(|(start, end)| target >= *start && target < *end)
                // A caret resting past the last character belongs on the final row.
                .unwrap_or(chunks.len() - 1);
            (row + index, target - chunks[index].0)
        } else {
            (
                head.line.checked_sub(view.top_line)?,
                target.checked_sub(view.left_col)?,
            )
        };

        if row >= height || column >= width {
            return None;
        }
        Some((
            area.x + gutter + u16::try_from(column).ok()?,
            area.y + u16::try_from(row).ok()?,
        ))
    }

    /// Draw the line number for one row.
    fn render_gutter(&self, surface: &mut Surface, area: Rect, y: u16, line: usize, caret: usize) {
        let width = gutter_width(self.config, self.buffer.document.len_lines());
        if width == 0 {
            return;
        }
        let is_caret_line = line == caret;
        let number = if self.config.relative_line_numbers && !is_caret_line {
            line.abs_diff(caret)
        } else {
            line + 1
        };
        let style = if is_caret_line {
            self.theme.gutter_active
        } else {
            self.theme.gutter
        };

        // Right-align inside the gutter, leaving the final column as padding.
        let label = format!("{number:>width$} ", width = usize::from(width) - 1);
        for (offset, ch) in label.chars().enumerate() {
            let Ok(offset) = u16::try_from(offset) else {
                break;
            };
            if offset >= width {
                break;
            }
            if let Some(cell) = surface.cell_mut((area.x + offset, y)) {
                cell.set_char(ch).set_style(style);
            }
        }
    }

    /// Draw one screen row.
    ///
    /// `first_cell` is where in the expanded line the row starts: the horizontal
    /// scroll offset when wrapping is off, and the start of a wrapped chunk when
    /// it is on. Having one routine for both is what keeps selection, search and
    /// syntax styling identical in the two modes.
    fn render_row(&self, surface: &mut Surface, area: Rect, row: Row, caret: usize) {
        let Row {
            y,
            line,
            cells: (first_cell, last_cell),
        } = row;
        let gutter = gutter_width(self.config, self.buffer.document.len_lines());
        let x0 = area.x + gutter;
        let width = area.width.saturating_sub(gutter);

        // Only the focused window highlights its caret line: with splits open,
        // two highlighted lines would leave the caret's whereabouts ambiguous.
        let base = if line == caret && self.config.highlight_current_line && self.focused {
            self.theme.text.patch(self.theme.cursor_line)
        } else {
            self.theme.text
        };

        let text = self.buffer.document.line_string(line);
        let display = DisplayLine::new(&text, self.config.tab_width);
        let line_start = self.buffer.document.line_start(line);
        let left = first_cell;

        // Matching only the visible lines is what keeps search highlighting free
        // on a large file: the cost is bounded by the window, not the document.
        let matches = self
            .search
            .map(|search| search.matches_in_line(&text))
            .unwrap_or_default();
        let syntax = if self.config.syntax_highlighting {
            self.buffer.syntax.highlight(&self.buffer.document, line)
        } else {
            Vec::new()
        };

        for column in 0..width {
            let Some(cell) = surface.cell_mut((x0 + column, y)) else {
                continue;
            };
            let index = left + usize::from(column);
            // A wrapped row stops at its chunk boundary; the rest of the row is
            // padding, not the next chunk's text.
            match display.cells.get(index).filter(|_| index < last_cell) {
                Some(display_cell) => {
                    let style = self.style_for(
                        base,
                        line_start,
                        display_cell.char_index,
                        &matches,
                        &syntax,
                    );
                    match display_cell.glyph {
                        Some(glyph) => {
                            cell.set_char(glyph).set_style(style);
                        }
                        // Trailing half of a wide glyph: the glyph itself already
                        // covers this column, unless it was scrolled off the left
                        // edge, in which case draw a space to keep alignment.
                        None if column == 0 => {
                            cell.set_char(' ').set_style(style);
                        }
                        None => {
                            cell.set_symbol("").set_style(style);
                        }
                    }
                }
                // Past the end of the line: still paint the background so the
                // current-line highlight spans the full width.
                None => {
                    cell.set_char(' ').set_style(base);
                }
            }
        }
    }

    /// Layer the styles that can apply to one character.
    ///
    /// Order matters: the selection is what the user is acting on, so it wins
    /// over a search match, and the match under the caret wins over the rest.
    fn style_for(
        &self,
        base: Style,
        line_start: usize,
        column: usize,
        matches: &[LineMatch],
        syntax: &[Highlight],
    ) -> Style {
        let index = line_start + column;
        if self.selection.is_some_and(|range| range.contains(index)) {
            return base.patch(self.theme.selection);
        }
        if self.active_match.is_some_and(|range| range.contains(index)) {
            return base.patch(self.theme.search_active);
        }
        if matches
            .iter()
            .any(|found| column >= found.start && column < found.end)
        {
            return base.patch(self.theme.search);
        }
        // Syntax is the bottom layer: it colours text, while everything above
        // colours the background, so a keyword inside a selection keeps both.
        match syntax
            .iter()
            .find(|span| column >= span.start && column < span.end)
        {
            Some(span) => base.patch(self.theme.syntax.style_for(span.kind)),
            None => base,
        }
    }

    /// The expanded form of a line, and how it splits into screen rows.
    fn rows_of(&self, line: usize, width: usize) -> (DisplayLine, Vec<(usize, usize)>) {
        let display = DisplayLine::new(
            &self.buffer.document.line_string(line),
            self.config.tab_width,
        );
        let rows = display.wrap(width);
        (display, rows)
    }

    /// Mark rows past the end of the document, like vi's tildes.
    fn render_filler(&self, surface: &mut Surface, area: Rect, from_row: u16) {
        for row in from_row..area.height {
            if let Some(cell) = surface.cell_mut((area.x, area.y + row)) {
                cell.set_char('~').set_style(self.theme.gutter);
            }
        }
    }

    /// One document line per screen row, scrolled horizontally.
    fn render_unwrapped(&self, surface: &mut Surface, area: Rect, caret: usize) {
        let top = self.window.view.top_line;
        let left = self.window.view.left_col;

        for row in 0..area.height {
            let line = top + usize::from(row);
            if line >= self.buffer.document.len_lines() {
                self.render_filler(surface, area, row);
                return;
            }
            let y = area.y + row;
            self.render_gutter(surface, area, y, line, caret);
            self.render_row(
                surface,
                area,
                Row {
                    y,
                    line,
                    cells: (left, usize::MAX),
                },
                caret,
            );
        }
    }

    /// Long lines continue on the next row; the gutter is only numbered once
    /// per document line, so a wrapped line still reads as one line.
    fn render_wrapped(&self, surface: &mut Surface, area: Rect, caret: usize) {
        let width = usize::from(
            area.width
                .saturating_sub(gutter_width(self.config, self.buffer.document.len_lines())),
        );
        let mut row = 0u16;
        let mut line = self.window.view.top_line;

        while row < area.height {
            if line >= self.buffer.document.len_lines() {
                self.render_filler(surface, area, row);
                return;
            }
            let (_, chunks) = self.rows_of(line, width);
            for (index, (start, end)) in chunks.into_iter().enumerate() {
                if row >= area.height {
                    break;
                }
                let y = area.y + row;
                if index == 0 {
                    self.render_gutter(surface, area, y, line, caret);
                } else {
                    self.render_gutter_continuation(surface, area, y);
                }
                self.render_row(
                    surface,
                    area,
                    Row {
                        y,
                        line,
                        cells: (start, end),
                    },
                    caret,
                );
                row += 1;
            }
            line += 1;
        }
    }

    /// Blank gutter for the second and later rows of a wrapped line.
    fn render_gutter_continuation(&self, surface: &mut Surface, area: Rect, y: u16) {
        let width = gutter_width(self.config, self.buffer.document.len_lines());
        for offset in 0..width {
            if let Some(cell) = surface.cell_mut((area.x + offset, y)) {
                cell.set_char(if offset + 2 == width { '·' } else { ' ' })
                    .set_style(self.theme.gutter);
            }
        }
    }
}

impl Widget for EditorView<'_> {
    fn render(self, area: Rect, surface: &mut Surface) {
        if area.is_empty() {
            return;
        }
        surface.set_style(area, self.theme.text);
        let caret = self.window.cursor().head.line;

        if self.config.word_wrap {
            self.render_wrapped(surface, area, caret);
        } else {
            self.render_unwrapped(surface, area, caret);
        }
    }
}
