//! # Built-in themes
//!
//! **Purpose:** ship two themes that work out of the box.
//!
//! **Responsibility:** hold the palettes and assemble them into [`Theme`]
//! values. Only this module contains literal colours — everywhere else asks the
//! theme for a slot.
//!
//! Both palettes use 24-bit RGB. Terminals without truecolor degrade these to
//! the nearest colour themselves, which looks better than hand-picking ANSI
//! indices that clash with the user's own palette.
//!
//! **Public API:** [`dark`], [`light`].
//!
//! The light palette is not the dark one inverted: readable light themes need
//! *darker*, more saturated accents, because a pastel that reads well on black
//! disappears on white.

use ratatui::style::{Color, Modifier, Style};

use super::{SyntaxStyles, Theme};

/// A dark theme in the One Dark / Gruvbox family.
#[must_use]
pub fn dark() -> Theme {
    let bg = Color::Rgb(0x1e, 0x22, 0x27);
    let bg_soft = Color::Rgb(0x26, 0x2b, 0x32);
    let bg_lift = Color::Rgb(0x2f, 0x35, 0x3d);
    let fg = Color::Rgb(0xc5, 0xcd, 0xd9);
    let fg_dim = Color::Rgb(0x5c, 0x66, 0x70);

    Theme {
        name: "dark".to_string(),
        text: Style::new().fg(fg).bg(bg),
        gutter: Style::new().fg(fg_dim).bg(bg),
        gutter_active: Style::new()
            .fg(Color::Rgb(0xd7, 0xba, 0x7d))
            .bg(bg_soft)
            .add_modifier(Modifier::BOLD),
        cursor_line: Style::new().bg(bg_soft),
        selection: Style::new().bg(Color::Rgb(0x3b, 0x4a, 0x5f)),
        status: Style::new().fg(fg).bg(bg_lift),
        status_mode: Style::new()
            .fg(bg)
            .bg(Color::Rgb(0x7f, 0xb9, 0x6e))
            .add_modifier(Modifier::BOLD),
        status_dirty: Style::new().fg(Color::Rgb(0xe0, 0x8f, 0x68)),
        command: Style::new().fg(fg).bg(bg),
        command_error: Style::new()
            .fg(Color::Rgb(0xe5, 0x5f, 0x5f))
            .add_modifier(Modifier::BOLD),
        tab_active: Style::new().fg(fg).bg(bg_lift).add_modifier(Modifier::BOLD),
        tab_inactive: Style::new().fg(fg_dim).bg(bg),
        popup: Style::new().fg(fg).bg(bg_lift),
        popup_border: Style::new().fg(Color::Rgb(0x56, 0x9c, 0xd6)).bg(bg_lift),
        tree_directory: Style::new()
            .fg(Color::Rgb(0x56, 0x9c, 0xd6))
            .add_modifier(Modifier::BOLD),
        tree_file: Style::new().fg(fg),
        search: Style::new().fg(bg).bg(Color::Rgb(0xd7, 0xba, 0x7d)),
        search_active: Style::new()
            .fg(bg)
            .bg(Color::Rgb(0xe0, 0x8f, 0x68))
            .add_modifier(Modifier::BOLD),
        syntax: SyntaxStyles {
            keyword: Style::new().fg(Color::Rgb(0xc5, 0x86, 0xc0)),
            type_name: Style::new().fg(Color::Rgb(0x4e, 0xc9, 0xb0)),
            function: Style::new().fg(Color::Rgb(0x61, 0xaf, 0xef)),
            string: Style::new().fg(Color::Rgb(0x98, 0xc3, 0x79)),
            number: Style::new().fg(Color::Rgb(0xd1, 0x9a, 0x66)),
            comment: Style::new()
                .fg(Color::Rgb(0x5c, 0x66, 0x70))
                .add_modifier(Modifier::ITALIC),
            constant: Style::new().fg(Color::Rgb(0xd1, 0x9a, 0x66)),
            operator: Style::new().fg(Color::Rgb(0x56, 0xb6, 0xc2)),
            punctuation: Style::new().fg(Color::Rgb(0xab, 0xb2, 0xbf)),
            attribute: Style::new().fg(Color::Rgb(0xd7, 0xba, 0x7d)),
            macro_call: Style::new().fg(Color::Rgb(0xc5, 0x86, 0xc0)),
            heading: Style::new()
                .fg(Color::Rgb(0x61, 0xaf, 0xef))
                .add_modifier(Modifier::BOLD),
            emphasis: Style::new().add_modifier(Modifier::ITALIC),
            link: Style::new()
                .fg(Color::Rgb(0x56, 0xb6, 0xc2))
                .add_modifier(Modifier::UNDERLINED),
            diff_added: Style::new().fg(Color::Rgb(0x7f, 0xb9, 0x6e)),
            diff_removed: Style::new().fg(Color::Rgb(0xe5, 0x5f, 0x5f)),
            diff_hunk: Style::new()
                .fg(Color::Rgb(0x56, 0xb6, 0xc2))
                .add_modifier(Modifier::BOLD),
            diff_meta: Style::new()
                .fg(Color::Rgb(0xd7, 0xba, 0x7d))
                .add_modifier(Modifier::BOLD),
        },
    }
}

/// A light theme in the Solarized Light / GitHub family.
#[must_use]
pub fn light() -> Theme {
    let bg = Color::Rgb(0xfa, 0xfa, 0xf7);
    let bg_soft = Color::Rgb(0xef, 0xef, 0xea);
    let bg_lift = Color::Rgb(0xe2, 0xe2, 0xdb);
    let fg = Color::Rgb(0x2e, 0x33, 0x38);
    let fg_dim = Color::Rgb(0x8a, 0x92, 0x99);

    Theme {
        name: "light".to_string(),
        text: Style::new().fg(fg).bg(bg),
        gutter: Style::new().fg(fg_dim).bg(bg),
        gutter_active: Style::new()
            .fg(Color::Rgb(0x8f, 0x62, 0x00))
            .bg(bg_soft)
            .add_modifier(Modifier::BOLD),
        cursor_line: Style::new().bg(bg_soft),
        selection: Style::new().bg(Color::Rgb(0xc7, 0xdb, 0xf0)),
        status: Style::new().fg(fg).bg(bg_lift),
        status_mode: Style::new()
            .fg(bg)
            .bg(Color::Rgb(0x2e, 0x7d, 0x32))
            .add_modifier(Modifier::BOLD),
        status_dirty: Style::new().fg(Color::Rgb(0xb2, 0x5e, 0x09)),
        command: Style::new().fg(fg).bg(bg),
        command_error: Style::new()
            .fg(Color::Rgb(0xc0, 0x2c, 0x2c))
            .add_modifier(Modifier::BOLD),
        tab_active: Style::new().fg(fg).bg(bg_lift).add_modifier(Modifier::BOLD),
        tab_inactive: Style::new().fg(fg_dim).bg(bg),
        popup: Style::new().fg(fg).bg(bg_lift),
        popup_border: Style::new().fg(Color::Rgb(0x1e, 0x63, 0xa8)).bg(bg_lift),
        tree_directory: Style::new()
            .fg(Color::Rgb(0x1e, 0x63, 0xa8))
            .add_modifier(Modifier::BOLD),
        tree_file: Style::new().fg(fg),
        search: Style::new().fg(fg).bg(Color::Rgb(0xf5, 0xdd, 0x8f)),
        search_active: Style::new()
            .fg(bg)
            .bg(Color::Rgb(0xd1, 0x71, 0x1c))
            .add_modifier(Modifier::BOLD),
        syntax: SyntaxStyles {
            keyword: Style::new().fg(Color::Rgb(0x8b, 0x2c, 0x9e)),
            type_name: Style::new().fg(Color::Rgb(0x0f, 0x6b, 0x63)),
            function: Style::new().fg(Color::Rgb(0x1e, 0x63, 0xa8)),
            string: Style::new().fg(Color::Rgb(0x2e, 0x6f, 0x1e)),
            number: Style::new().fg(Color::Rgb(0xa6, 0x52, 0x00)),
            comment: Style::new()
                .fg(Color::Rgb(0x8a, 0x92, 0x99))
                .add_modifier(Modifier::ITALIC),
            constant: Style::new().fg(Color::Rgb(0xa6, 0x52, 0x00)),
            operator: Style::new().fg(Color::Rgb(0x0d, 0x74, 0x89)),
            punctuation: Style::new().fg(Color::Rgb(0x55, 0x5d, 0x64)),
            attribute: Style::new().fg(Color::Rgb(0x8f, 0x62, 0x00)),
            macro_call: Style::new().fg(Color::Rgb(0x8b, 0x2c, 0x9e)),
            heading: Style::new()
                .fg(Color::Rgb(0x1e, 0x63, 0xa8))
                .add_modifier(Modifier::BOLD),
            emphasis: Style::new().add_modifier(Modifier::ITALIC),
            link: Style::new()
                .fg(Color::Rgb(0x0d, 0x74, 0x89))
                .add_modifier(Modifier::UNDERLINED),
            diff_added: Style::new().fg(Color::Rgb(0x2e, 0x7d, 0x32)),
            diff_removed: Style::new().fg(Color::Rgb(0xc0, 0x2c, 0x2c)),
            diff_hunk: Style::new()
                .fg(Color::Rgb(0x0d, 0x74, 0x89))
                .add_modifier(Modifier::BOLD),
            diff_meta: Style::new()
                .fg(Color::Rgb(0x8f, 0x62, 0x00))
                .add_modifier(Modifier::BOLD),
        },
    }
}
