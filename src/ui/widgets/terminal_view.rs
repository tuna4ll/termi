//! # Terminal widgets
//!
//! **Purpose:** paint one emulated terminal and describe its focused session.
//!
//! **Responsibility:** translate vt100 cells into ratatui cells, expose the
//! child application's cursor position and render the terminal status bar. PTY
//! input, output and process lifetime stay in [`crate::terminal`].
//!
//! **Public API:** [`TerminalView`], [`TerminalStatusBar`].

use ratatui::buffer::Buffer as Surface;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;

use crate::terminal::Terminal;
use crate::theme::Theme;

/// The visible cells of one terminal session.
pub struct TerminalView<'a> {
    /// Session whose screen is rendered.
    pub terminal: &'a Terminal,
    /// Whether this pane owns the application's visible cursor.
    pub focused: bool,
    /// Base colours used for terminal-default foreground and background.
    pub theme: &'a Theme,
}

impl TerminalView<'_> {
    /// Screen position requested by the child application, when visible.
    #[must_use]
    pub fn caret_position(&self, area: Rect) -> Option<(u16, u16)> {
        if !self.focused || area.is_empty() {
            return None;
        }
        self.terminal.with_screen(|screen| {
            if screen.hide_cursor() || screen.scrollback() > 0 {
                return None;
            }
            let (row, col) = screen.cursor_position();
            (row < area.height && col < area.width).then_some((area.x + col, area.y + row))
        })
    }
}

impl Widget for TerminalView<'_> {
    fn render(self, area: Rect, surface: &mut Surface) {
        if area.is_empty() {
            return;
        }
        surface.set_style(area, self.theme.text);
        self.terminal.with_screen(|screen| {
            let (rows, cols) = screen.size();
            for row in 0..rows.min(area.height) {
                for col in 0..cols.min(area.width) {
                    let Some(source) = screen.cell(row, col) else {
                        continue;
                    };
                    let Some(target) = surface.cell_mut((area.x + col, area.y + row)) else {
                        continue;
                    };
                    let symbol = if source.is_wide_continuation() {
                        ""
                    } else if source.has_contents() {
                        source.contents()
                    } else {
                        " "
                    };
                    target
                        .set_symbol(symbol)
                        .set_style(cell_style(source, self.theme.text));
                }
            }
        });
    }
}

/// One-line context for the focused terminal.
pub struct TerminalStatusBar<'a> {
    /// Command or shell name.
    pub name: &'a str,
    /// Whether the process still owns the PTY output stream.
    pub running: bool,
    /// Colours shared with the editor status bar.
    pub theme: &'a Theme,
}

impl Widget for TerminalStatusBar<'_> {
    fn render(self, area: Rect, surface: &mut Surface) {
        if area.is_empty() {
            return;
        }
        surface.set_style(area, self.theme.status);
        Line::from(vec![
            Span::styled(" TERMINAL ", self.theme.status_mode),
            Span::styled(format!(" {}", self.name), self.theme.status),
        ])
        .render(area, surface);

        let state = if self.running { "running" } else { "exited" };
        Line::from(Span::styled(
            format!(" {state}  Ctrl+W: windows "),
            self.theme.status,
        ))
        .right_aligned()
        .render(area, surface);
    }
}

fn cell_style(cell: &vt100::Cell, base: Style) -> Style {
    let mut foreground = terminal_color(cell.fgcolor()).or(base.fg);
    let mut background = terminal_color(cell.bgcolor()).or(base.bg);
    if cell.inverse() {
        std::mem::swap(&mut foreground, &mut background);
    }

    let mut style = base;
    if let Some(color) = foreground {
        style = style.fg(color);
    }
    if let Some(color) = background {
        style = style.bg(color);
    }
    if cell.bold() {
        style = style.add_modifier(Modifier::BOLD);
    }
    if cell.dim() {
        style = style.add_modifier(Modifier::DIM);
    }
    if cell.italic() {
        style = style.add_modifier(Modifier::ITALIC);
    }
    if cell.underline() {
        style = style.add_modifier(Modifier::UNDERLINED);
    }
    style
}

const fn terminal_color(color: vt100::Color) -> Option<Color> {
    match color {
        vt100::Color::Default => None,
        vt100::Color::Idx(index) => Some(Color::Indexed(index)),
        vt100::Color::Rgb(red, green, blue) => Some(Color::Rgb(red, green, blue)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ansi_colours_and_attributes_become_ratatui_styles() {
        let mut parser = vt100::Parser::new(1, 1, 0);
        parser.process(b"\x1b[1;3;4;38;5;123;48;2;1;2;3mX");
        let cell = parser.screen().cell(0, 0).expect("the parser has one cell");
        let style = cell_style(cell, Style::new().fg(Color::White).bg(Color::Black));

        assert_eq!(style.fg, Some(Color::Indexed(123)));
        assert_eq!(style.bg, Some(Color::Rgb(1, 2, 3)));
        assert!(style.add_modifier.contains(Modifier::BOLD));
        assert!(style.add_modifier.contains(Modifier::ITALIC));
        assert!(style.add_modifier.contains(Modifier::UNDERLINED));
    }
}
