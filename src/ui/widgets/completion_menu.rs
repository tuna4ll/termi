//! # Completion menu
//!
//! **Purpose:** show what a language server offers to complete at the caret.
//!
//! **Responsibility:** a small box under the caret — or above it, when the
//! caret is near the bottom — listing the matching items with their details,
//! the highlighted one marked and scrolled into view.
//!
//! **Public API:** [`CompletionMenuView`].

use ratatui::buffer::Buffer as Surface;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Widget};
use unicode_width::UnicodeWidthStr;

use crate::app::lsp::CompletionMenu;
use crate::theme::Theme;

/// Rows the menu shows at most.
const ROWS: usize = 10;

/// The completion menu, anchored at the caret.
pub struct CompletionMenuView<'a> {
    /// What is on offer.
    pub menu: &'a CompletionMenu,
    /// The screen cell of the caret.
    pub caret: (u16, u16),
    /// Colours.
    pub theme: &'a Theme,
}

impl CompletionMenuView<'_> {
    /// Where the menu goes inside `screen`.
    fn area(&self, screen: Rect) -> Rect {
        let rows = self.menu.shown.len().min(ROWS);
        let widest = self
            .menu
            .shown
            .iter()
            .filter_map(|index| self.menu.items.get(*index))
            .map(|item| item.label.width() + item.detail.width().min(30) + 4)
            .max()
            .unwrap_or(10);
        let width = u16::try_from(widest.clamp(12, 60))
            .unwrap_or(60)
            .min(screen.width);
        let height = u16::try_from(rows).unwrap_or(1);
        let (x, y) = self.caret;
        let below = y + 1 + height <= screen.bottom();
        Rect {
            x: x.min(screen.right().saturating_sub(width)),
            y: if below {
                y + 1
            } else {
                y.saturating_sub(height)
            },
            width,
            height: height.min(screen.height),
        }
    }
}

impl Widget for CompletionMenuView<'_> {
    fn render(self, screen: Rect, surface: &mut Surface) {
        if self.menu.shown.is_empty() || screen.is_empty() {
            return;
        }
        let area = self.area(screen);
        Clear.render(area, surface);
        let first = self
            .menu
            .selected
            .saturating_sub(usize::from(area.height).saturating_sub(1));
        for (row, index) in self
            .menu
            .shown
            .iter()
            .skip(first)
            .take(usize::from(area.height))
            .enumerate()
        {
            let Some(item) = self.menu.items.get(*index) else {
                continue;
            };
            let selected = first + row == self.menu.selected;
            let style = if selected {
                self.theme.popup.patch(self.theme.selection)
            } else {
                self.theme.popup
            };
            let line = Rect {
                y: area.y + u16::try_from(row).unwrap_or(0),
                height: 1,
                ..area
            };
            surface.set_style(line, style);
            let detail: String = item.detail.chars().take(30).collect();
            let label_width = usize::from(area.width).saturating_sub(detail.width() + 3);
            let label: String = item.label.chars().take(label_width).collect();
            Line::from(vec![
                Span::styled(format!(" {label:<label_width$}"), style),
                Span::styled(format!(" {detail} "), style.patch(self.theme.gutter)),
            ])
            .render(line, surface);
        }
    }
}
