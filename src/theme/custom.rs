//! # Custom themes
//!
//! **Purpose:** let users describe a theme in TOML without recompiling.
//!
//! **Responsibility:** parse colour strings, deserialise a theme file, and
//! overlay it on a built-in base. Overlaying rather than requiring a complete
//! file is deliberate — a user who only wants a different comment colour should
//! write three lines, not sixty.
//!
//! A theme file looks like:
//!
//! ```toml
//! name = "midnight"
//! base = "dark"
//!
//! [comment]
//! fg = "#4a5058"
//! italic = true
//!
//! [selection]
//! bg = "bright-blue"
//! ```
//!
//! **Public API:** [`CustomTheme`], [`StyleSpec`], [`parse_color`].

use std::collections::HashMap;

use ratatui::style::{Color, Modifier, Style};
use serde::Deserialize;

use super::Theme;

/// One slot's overrides. Every field is optional so a spec can change only the
/// foreground and leave the background alone.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StyleSpec {
    /// Foreground colour.
    pub fg: Option<String>,
    /// Background colour.
    pub bg: Option<String>,
    /// Force bold on or off.
    pub bold: Option<bool>,
    /// Force italic on or off.
    pub italic: Option<bool>,
    /// Force underline on or off.
    pub underline: Option<bool>,
}

impl StyleSpec {
    /// Apply this spec on top of an existing style.
    fn overlay(&self, mut style: Style) -> Style {
        if let Some(fg) = self.fg.as_deref().and_then(parse_color) {
            style = style.fg(fg);
        }
        if let Some(bg) = self.bg.as_deref().and_then(parse_color) {
            style = style.bg(bg);
        }
        for (flag, modifier) in [
            (self.bold, Modifier::BOLD),
            (self.italic, Modifier::ITALIC),
            (self.underline, Modifier::UNDERLINED),
        ] {
            match flag {
                Some(true) => style = style.add_modifier(modifier),
                Some(false) => style = style.remove_modifier(modifier),
                None => {}
            }
        }
        style
    }
}

/// A theme file: a base to start from plus per-slot overrides.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct CustomTheme {
    /// Name reported in the status bar; defaults to the file stem.
    pub name: Option<String>,
    /// `"dark"` or `"light"`; defaults to dark.
    pub base: Option<String>,
    /// Slot name to overrides. Unknown slot names are ignored rather than
    /// rejected, so a theme written for a newer version still loads.
    #[serde(flatten)]
    pub slots: HashMap<String, StyleSpec>,
}

impl CustomTheme {
    /// Resolve into a complete theme.
    #[must_use]
    pub fn build(self, fallback_name: &str) -> Theme {
        let mut theme = Theme::builtin(self.base.as_deref().unwrap_or("dark"));
        theme.name = self.name.unwrap_or_else(|| fallback_name.to_string());

        for (slot, spec) in &self.slots {
            if let Some(target) = slot_mut(&mut theme, slot) {
                *target = spec.overlay(*target);
            }
        }
        theme
    }
}

/// Map a slot name from a theme file onto the field it configures.
///
/// An explicit table keeps the file format documented in one place and makes an
/// unknown name a no-op rather than a panic.
fn slot_mut<'a>(theme: &'a mut Theme, slot: &str) -> Option<&'a mut Style> {
    Some(match slot {
        "text" => &mut theme.text,
        "gutter" => &mut theme.gutter,
        "gutter_active" => &mut theme.gutter_active,
        "cursor_line" => &mut theme.cursor_line,
        "selection" => &mut theme.selection,
        "status" => &mut theme.status,
        "status_mode" => &mut theme.status_mode,
        "status_dirty" => &mut theme.status_dirty,
        "command" => &mut theme.command,
        "command_error" => &mut theme.command_error,
        "tab_active" => &mut theme.tab_active,
        "tab_inactive" => &mut theme.tab_inactive,
        "popup" => &mut theme.popup,
        "popup_border" => &mut theme.popup_border,
        "tree_directory" => &mut theme.tree_directory,
        "tree_file" => &mut theme.tree_file,
        "search" => &mut theme.search,
        "search_active" => &mut theme.search_active,
        "keyword" => &mut theme.syntax.keyword,
        "type" => &mut theme.syntax.type_name,
        "function" => &mut theme.syntax.function,
        "string" => &mut theme.syntax.string,
        "number" => &mut theme.syntax.number,
        "comment" => &mut theme.syntax.comment,
        "constant" => &mut theme.syntax.constant,
        "operator" => &mut theme.syntax.operator,
        "punctuation" => &mut theme.syntax.punctuation,
        "attribute" => &mut theme.syntax.attribute,
        "macro" => &mut theme.syntax.macro_call,
        "heading" => &mut theme.syntax.heading,
        "emphasis" => &mut theme.syntax.emphasis,
        "link" => &mut theme.syntax.link,
        "diff_added" => &mut theme.syntax.diff_added,
        "diff_removed" => &mut theme.syntax.diff_removed,
        "diff_hunk" => &mut theme.syntax.diff_hunk,
        "diff_meta" => &mut theme.syntax.diff_meta,
        _ => return None,
    })
}

/// Parse `#rrggbb`, `#rgb`, an ANSI colour name, or a 0-255 palette index.
///
/// Returns `None` for anything unrecognised so the caller can keep the base
/// theme's colour instead of failing to load the whole file.
#[must_use]
pub fn parse_color(value: &str) -> Option<Color> {
    let value = value.trim();
    if let Some(hex) = value.strip_prefix('#') {
        return parse_hex(hex);
    }
    if let Ok(index) = value.parse::<u8>() {
        return Some(Color::Indexed(index));
    }
    Some(
        match value.to_ascii_lowercase().replace(['-', '_'], "").as_str() {
            "black" => Color::Black,
            "red" => Color::Red,
            "green" => Color::Green,
            "yellow" => Color::Yellow,
            "blue" => Color::Blue,
            "magenta" => Color::Magenta,
            "cyan" => Color::Cyan,
            "gray" | "grey" | "white" => Color::Gray,
            "darkgray" | "darkgrey" => Color::DarkGray,
            "brightred" => Color::LightRed,
            "brightgreen" => Color::LightGreen,
            "brightyellow" => Color::LightYellow,
            "brightblue" => Color::LightBlue,
            "brightmagenta" => Color::LightMagenta,
            "brightcyan" => Color::LightCyan,
            "brightwhite" => Color::White,
            "reset" | "default" => Color::Reset,
            _ => return None,
        },
    )
}

fn parse_hex(hex: &str) -> Option<Color> {
    match hex.len() {
        // `#f80` is shorthand for `#ff8800`, so each nibble is doubled — which
        // is exactly multiplying by 0x11.
        3 => {
            let mut nibbles = hex
                .chars()
                .map(|c| c.to_digit(16).and_then(|d| u8::try_from(d).ok()));
            let mut next = || nibbles.next().flatten().map(|d| d * 0x11);
            Some(Color::Rgb(next()?, next()?, next()?))
        }
        6 => {
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            Some(Color::Rgb(r, g, b))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_hex_colours_in_both_lengths() {
        assert_eq!(parse_color("#ff8800"), Some(Color::Rgb(0xff, 0x88, 0x00)));
        assert_eq!(parse_color("#f80"), Some(Color::Rgb(0xff, 0x88, 0x00)));
    }

    #[test]
    fn parses_names_and_palette_indices() {
        assert_eq!(parse_color("bright-blue"), Some(Color::LightBlue));
        assert_eq!(parse_color("42"), Some(Color::Indexed(42)));
        assert_eq!(parse_color("chartreuse"), None);
    }

    #[test]
    fn overrides_are_layered_on_the_base_theme() {
        // `##` delimiters: the body itself contains `"#`.
        let toml = r##"
            name = "midnight"
            base = "dark"
            [comment]
            fg = "#4a5058"
            italic = false
            [selection]
            bg = "bright-blue"
        "##;
        let custom: CustomTheme = toml::from_str(toml).expect("valid theme");
        let theme = custom.build("fallback");

        assert_eq!(theme.name, "midnight");
        assert_eq!(theme.syntax.comment.fg, Some(Color::Rgb(0x4a, 0x50, 0x58)));
        assert!(!theme.syntax.comment.add_modifier.contains(Modifier::ITALIC));
        assert_eq!(theme.selection.bg, Some(Color::LightBlue));
        // Untouched slots keep the base palette.
        assert_eq!(
            theme.syntax.keyword,
            crate::theme::builtin::dark().syntax.keyword
        );
    }

    #[test]
    fn unknown_slots_are_ignored() {
        let custom: CustomTheme =
            toml::from_str("[nonsense]\nfg = \"#000000\"").expect("valid theme");
        assert_eq!(custom.build("x").name, "x");
    }
}
