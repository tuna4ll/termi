//! # Minimap
//!
//! **Purpose:** show the shape of the whole file beside the window, with the
//! part on screen marked.
//!
//! **Responsibility:** squeeze the text into braille: each cell holds a 2×4
//! grid of dots, each dot standing for a few columns of one line, lit when
//! anything but whitespace is there. The result reads like a zoomed-out
//! view of the code's indentation and density, which is all a minimap is for.
//!
//! **Public API:** [`Minimap`], [`WIDTH`].

use ratatui::buffer::Buffer as Surface;
use ratatui::layout::Rect;
use ratatui::widgets::Widget;

use crate::editor::buffer::Buffer;
use crate::editor::window::Window;
use crate::theme::Theme;

/// Columns a minimap takes, its separating space included.
pub const WIDTH: u16 = 12;

/// Text columns one braille dot stands for.
const COLUMNS_PER_DOT: usize = 2;

/// Lines one row of cells covers: a braille cell is four dots tall.
const LINES_PER_ROW: usize = 4;

/// The miniature of one window's buffer.
pub struct Minimap<'a> {
    /// What to draw.
    pub buffer: &'a Buffer,
    /// The window beside it, whose view is marked.
    pub window: &'a Window,
    /// Colours.
    pub theme: &'a Theme,
}

/// The bit for the dot at `(x, y)` in a braille cell, `x` in `0..2` and `y`
/// in `0..4`, per the Unicode braille layout.
const fn dot(x: usize, y: usize) -> u32 {
    match (x, y) {
        (0, 0) => 0x01,
        (0, 1) => 0x02,
        (0, 2) => 0x04,
        (1, 0) => 0x08,
        (1, 1) => 0x10,
        (1, 2) => 0x20,
        (0, 3) => 0x40,
        _ => 0x80,
    }
}

impl Widget for Minimap<'_> {
    fn render(self, area: Rect, surface: &mut Surface) {
        if area.width < 2 || area.height == 0 {
            return;
        }
        let document = &self.buffer.document;
        let cells = usize::from(area.width - 1);
        let rows = usize::from(area.height);
        let capacity = rows * LINES_PER_ROW;
        let view_top = self.window.view.top_line;
        let view_height = usize::from(self.window.area.height);

        // Scroll the miniature along with the window once the file is longer
        // than it can show, keeping the view centred where possible.
        let total = document.len_lines();
        let first = if total <= capacity {
            0
        } else {
            let centred = view_top.saturating_sub((capacity.saturating_sub(view_height)) / 2);
            centred.min(total - capacity)
        };

        for row in 0..rows {
            let y = area.y + u16::try_from(row).unwrap_or(u16::MAX);
            let top_line = first + row * LINES_PER_ROW;
            let in_view = top_line + LINES_PER_ROW > view_top && top_line < view_top + view_height;
            let style = if in_view {
                self.theme.text.patch(self.theme.cursor_line)
            } else {
                self.theme.text
            };
            let mut bits = vec![0u32; cells];
            for offset in 0..LINES_PER_ROW {
                let line = top_line + offset;
                if line >= total {
                    break;
                }
                for (column, ch) in document.line(line).chars().enumerate() {
                    let dot_column = column / COLUMNS_PER_DOT;
                    let cell = dot_column / 2;
                    if cell >= cells {
                        break;
                    }
                    if !ch.is_whitespace() {
                        bits[cell] |= dot(dot_column % 2, offset);
                    }
                }
            }
            if let Some(cell) = surface.cell_mut((area.x, y)) {
                cell.set_char(' ').set_style(self.theme.gutter);
            }
            for (index, value) in bits.into_iter().enumerate() {
                let x = area.x + 1 + u16::try_from(index).unwrap_or(u16::MAX);
                let glyph = char::from_u32(0x2800 + value).unwrap_or(' ');
                if let Some(cell) = surface.cell_mut((x, y)) {
                    cell.set_char(glyph)
                        .set_style(style.patch(self.theme.whitespace));
                }
            }
        }
    }
}
