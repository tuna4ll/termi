//! # Picker widget
//!
//! **Purpose:** draw a searchable modal list.
//!
//! **Responsibility:** show the query, ranked matches, selection and keyboard
//! hints without changing picker state.
//!
//! **Public API:** [`PickerView`].

use ratatui::buffer::Buffer as Surface;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Widget};

use crate::picker::Picker;
use crate::theme::Theme;

pub struct PickerView<'a> {
    pub picker: &'a Picker,
    pub theme: &'a Theme,
}

impl PickerView<'_> {
    #[must_use]
    pub fn area(&self, outer: Rect) -> Rect {
        let width = (outer.width * 4 / 5).clamp(20, 90).min(outer.width);
        let height = outer.height.saturating_sub(2).clamp(5, 18);
        Rect {
            x: outer.x + (outer.width.saturating_sub(width)) / 2,
            y: outer.y + 1.min(outer.height.saturating_sub(height)),
            width,
            height,
        }
    }

    #[must_use]
    pub fn caret_position(&self, outer: Rect) -> (u16, u16) {
        let area = self.area(outer);
        let query_width = self.picker.query.chars().count();
        let offset = u16::try_from(query_width).unwrap_or(u16::MAX);
        (
            area.x
                .saturating_add(3)
                .saturating_add(offset)
                .min(area.right().saturating_sub(2)),
            area.y + 1,
        )
    }
}

impl Widget for PickerView<'_> {
    fn render(self, outer: Rect, surface: &mut Surface) {
        if outer.is_empty() {
            return;
        }
        let area = self.area(outer);
        Clear.render(area, surface);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(self.theme.popup_border)
            .style(self.theme.popup)
            .title(Span::styled(
                format!(" {} ", self.picker.title),
                self.theme.popup_border,
            ));
        let inner = block.inner(area);
        block.render(area, surface);
        if inner.is_empty() {
            return;
        }

        Line::from(vec![
            Span::styled("> ", self.theme.popup_border),
            Span::styled(&self.picker.query, self.theme.popup),
        ])
        .render(Rect { height: 1, ..inner }, surface);

        let list_height = usize::from(inner.height.saturating_sub(2));
        let matches: Vec<_> = self.picker.matches().collect();
        if matches.is_empty() && inner.height > 1 {
            let message = if self.picker.query.is_empty() {
                "nothing to select".to_string()
            } else {
                format!("no matches for ‘{}’", self.picker.query)
            };
            Line::from(Span::styled(message, self.theme.gutter)).render(
                Rect {
                    y: inner.y + 1,
                    height: 1,
                    ..inner
                },
                surface,
            );
        }

        let offset = if self.picker.selected < list_height {
            0
        } else {
            self.picker.selected - list_height + 1
        };
        for (row, item) in matches.iter().skip(offset).take(list_height).enumerate() {
            let index = offset + row;
            let style = if index == self.picker.selected {
                self.theme.selection
            } else {
                self.theme.popup
            };
            let line = Rect {
                y: inner.y + 1 + u16::try_from(row).unwrap_or(u16::MAX),
                height: 1,
                ..inner
            };
            surface.set_style(line, style);
            Line::from(Span::styled(format!("  {}", item.label), style)).render(line, surface);
        }

        if inner.height > 1 {
            Line::from(Span::styled(
                "↑↓ select  Enter open  Esc close",
                self.theme.gutter,
            ))
            .right_aligned()
            .render(
                Rect {
                    y: inner.bottom() - 1,
                    height: 1,
                    ..inner
                },
                surface,
            );
        }
    }
}
