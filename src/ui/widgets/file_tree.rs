//! # File tree
//!
//! **Purpose:** the side panel listing the project's files.
//!
//! **Responsibility:** draw a flattened [`Tree`](crate::filesystem::tree::Tree)
//! with indentation, folder markers and a selection highlight, scrolled so the
//! selected row stays visible. The tree itself decides what is expanded; this
//! widget only draws it.
//!
//! **Public API:** [`FileTree`].

use ratatui::buffer::Buffer as Surface;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Widget};

use std::collections::HashMap;
use std::path::PathBuf;

use crate::filesystem::tree::Entry;
use crate::theme::Theme;

/// The file browser panel.
pub struct FileTree<'a> {
    /// Rows to draw, already flattened.
    pub entries: &'a [Entry],
    /// Git's status letter for changed paths.
    pub status: &'a HashMap<PathBuf, char>,
    /// Index of the highlighted row.
    pub selected: usize,
    /// Whether the panel has keyboard focus.
    pub focused: bool,
    /// Title shown on the border, usually the root directory's name.
    pub title: &'a str,
    /// Colours.
    pub theme: &'a Theme,
}

impl FileTree<'_> {
    /// First row to draw so `selected` is visible in a panel `height` tall.
    fn scroll_offset(&self, height: usize) -> usize {
        if height == 0 || self.selected < height {
            return 0;
        }
        self.selected - height + 1
    }

    /// Indentation, folder marker and name for one row.
    fn label(entry: &Entry) -> String {
        let marker = if entry.is_dir {
            if entry.is_open { "▾ " } else { "▸ " }
        } else {
            "  "
        };
        format!("{}{marker}{}", "  ".repeat(entry.depth), entry.name)
    }
}

impl Widget for FileTree<'_> {
    fn render(self, area: Rect, surface: &mut Surface) {
        if area.is_empty() {
            return;
        }
        let border_style = if self.focused {
            self.theme.popup_border
        } else {
            self.theme.gutter
        };
        let block = Block::default()
            .borders(Borders::RIGHT)
            .border_type(BorderType::Plain)
            .border_style(border_style)
            .title(Span::styled(self.title, self.theme.tree_directory));

        let inner = block.inner(area);
        surface.set_style(area, self.theme.text);
        block.render(area, surface);

        let height = inner.height as usize;
        let offset = self.scroll_offset(height);

        for (row, entry) in self.entries.iter().skip(offset).take(height).enumerate() {
            let index = offset + row;
            let mut style = if entry.is_dir {
                self.theme.tree_directory
            } else {
                self.theme.tree_file
            };
            if index == self.selected {
                style = style.patch(if self.focused {
                    self.theme.selection
                } else {
                    self.theme.cursor_line
                });
            }

            let y = inner.y + u16::try_from(row).unwrap_or(u16::MAX);
            let line = Rect {
                y,
                height: 1,
                ..inner
            };
            // Painting the row first makes the selection span the full width
            // rather than stopping at the end of the name.
            surface.set_style(line, style);
            Line::from(Span::styled(Self::label(entry), style)).render(line, surface);
            if let Some(letter) = self.status.get(&entry.path) {
                let mark = match letter {
                    '?' | 'A' => self.theme.git_added,
                    'D' => self.theme.git_removed,
                    _ => self.theme.git_modified,
                };
                let x = line.right().saturating_sub(2);
                if let Some(cell) = surface.cell_mut((x, y)) {
                    cell.set_char(*letter).set_style(style.patch(mark));
                }
            }
        }
    }
}
