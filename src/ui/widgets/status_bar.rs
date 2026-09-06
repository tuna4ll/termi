//! # Status bar
//!
//! **Purpose:** tell the user where they are and what state the file is in.
//!
//! **Responsibility:** one line, split into a left segment (mode, file name,
//! unsaved marker) and a right segment (language, caret position, line ending).
//! It takes plain values rather than a reference to the application so it can be
//! rendered — and reasoned about — without the rest of the editor.
//!
//! **Public API:** [`StatusBar`].

use ratatui::buffer::Buffer as Surface;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;

use crate::app::mode::Mode;
use crate::editor::cursor::Position;
use crate::editor::document::LineEnding;
use crate::theme::Theme;

/// The bar along the bottom of the text area.
pub struct StatusBar<'a> {
    /// Current mode, shown as a coloured badge.
    pub mode: Mode,
    /// File name, or `[No Name]`.
    pub name: &'a str,
    /// Whether the buffer has unsaved changes.
    pub dirty: bool,
    /// Detected language, or `plain`.
    pub language: &'a str,
    /// Primary caret position.
    pub position: Position,
    /// Total number of lines, for the scroll percentage.
    pub line_count: usize,
    /// Line ending the file will be saved with.
    pub line_ending: LineEnding,
    /// Number of active cursors; shown only when greater than one.
    pub cursor_count: usize,
    /// Characters covered by the selection; shown only when there is one.
    ///
    /// The mode badge cannot report a selection made with Shift or the mouse —
    /// those leave the mode alone — so the count is what tells the user how much
    /// the next keystroke is about to act on.
    pub selected: usize,
    /// Colours.
    pub theme: &'a Theme,
}

impl Widget for StatusBar<'_> {
    fn render(self, area: Rect, surface: &mut Surface) {
        if area.is_empty() {
            return;
        }
        surface.set_style(area, self.theme.status);

        let mut left = vec![
            Span::styled(format!(" {} ", self.mode.name()), self.theme.status_mode),
            Span::styled(format!(" {}", self.name), self.theme.status),
        ];
        if self.dirty {
            left.push(Span::styled(" ●", self.theme.status_dirty));
        }
        if self.cursor_count > 1 {
            left.push(Span::styled(
                format!("  {} cursors", self.cursor_count),
                self.theme.status,
            ));
        }
        if self.selected > 0 {
            left.push(Span::styled(
                format!("  {} selected", self.selected),
                self.theme.status,
            ));
        }

        let right = Line::from(Span::styled(
            format!(
                " {}  {}  {}:{}  {}% ",
                self.language,
                self.line_ending.label(),
                self.position.line + 1,
                self.position.col + 1,
                self.scroll_percentage(),
            ),
            self.theme.status,
        ))
        .right_aligned();

        Line::from(left).render(area, surface);
        right.render(area, surface);
    }
}

impl StatusBar<'_> {
    /// How far through the file the caret is, as a percentage.
    ///
    /// A one-line file is "100%" rather than a division by zero.
    fn scroll_percentage(&self) -> usize {
        let last = self.line_count.saturating_sub(1);
        (self.position.line * 100).checked_div(last).unwrap_or(100)
    }
}
