//! # Configuration
//!
//! **Purpose:** every user-tunable knob in one place.
//!
//! **Responsibility:** define the settings, their defaults, and how they are
//! read from TOML. Nothing else parses config files, and no module reaches for a
//! default of its own — if a value is configurable it lives here.
//!
//! Every field has a `Default`, and `#[serde(default)]` means a config file only
//! needs to mention what it changes. Unknown keys are rejected so a typo is
//! reported instead of silently ignored.
//!
//! **Public API:** [`Config`].

use std::path::PathBuf;

use serde::Deserialize;

/// Directory holding `config.toml` and `themes/`.
///
/// `TERMI_CONFIG_DIR` overrides the platform default, which keeps tests and
/// portable installs from touching the real user configuration.
#[must_use]
pub fn config_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("TERMI_CONFIG_DIR") {
        return PathBuf::from(dir);
    }
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("termi")
}

/// Directory searched for custom theme files.
#[must_use]
pub fn themes_dir() -> PathBuf {
    config_dir().join("themes")
}

/// User settings.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// Theme name: `dark`, `light`, or the stem of a file in the themes
    /// directory.
    pub theme: String,
    /// Width of a tab stop in columns.
    pub tab_width: usize,
    /// Insert spaces instead of a literal tab character.
    pub expand_tabs: bool,
    /// Show the line-number gutter.
    pub line_numbers: bool,
    /// Number lines relative to the cursor, with the cursor's own line absolute.
    pub relative_line_numbers: bool,
    /// Copy the previous line's indentation onto new lines.
    pub auto_indent: bool,
    /// Wrap long lines instead of scrolling horizontally.
    pub word_wrap: bool,
    /// Highlight the line the cursor is on.
    pub highlight_current_line: bool,
    /// Lines of context to keep above and below the cursor while scrolling.
    pub scrolloff: usize,
    /// Enable syntax highlighting.
    pub syntax_highlighting: bool,
    /// Show the tab strip when more than one file is open.
    pub show_tabs: bool,
    /// Watch open files and warn when they change on disk.
    pub watch_files: bool,
    /// Strip trailing whitespace from every line on save.
    pub trim_trailing_whitespace: bool,
    /// Use the system clipboard for yank and paste. When `false`, an internal
    /// register is used instead.
    pub system_clipboard: bool,
    /// Let the mouse place the caret, select by dragging, focus windows and
    /// scroll them.
    ///
    /// Capturing the mouse takes drag-to-select away from the *terminal*, and
    /// with it copying to the X or Wayland primary selection, so this is worth
    /// turning off for anyone who selects text that way; most terminals still
    /// offer their own selection under Shift.
    pub mouse: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            theme: "dark".to_string(),
            tab_width: 4,
            expand_tabs: true,
            line_numbers: true,
            relative_line_numbers: false,
            auto_indent: true,
            word_wrap: false,
            highlight_current_line: true,
            scrolloff: 3,
            syntax_highlighting: true,
            show_tabs: true,
            watch_files: true,
            trim_trailing_whitespace: false,
            system_clipboard: true,
            mouse: true,
        }
    }
}

impl Config {
    /// Read `<config_dir>/config.toml`.
    ///
    /// Returns the defaults plus a human-readable warning when the file is
    /// missing pieces or malformed. A broken config must never stop the editor
    /// from opening — the user needs an editor to fix it with.
    #[must_use]
    pub fn load() -> (Self, Option<String>) {
        let path = config_dir().join("config.toml");
        if !path.is_file() {
            return (Self::default(), None);
        }
        match crate::filesystem::read_file(&path).and_then(|text| Ok(Self::parse(&text)?)) {
            Ok(config) => (config, None),
            Err(error) => (
                Self::default(),
                Some(format!("{}: {error}", path.display())),
            ),
        }
    }

    /// Parse a config from a string.
    ///
    /// # Errors
    /// Returns an error if the TOML is invalid or contains unknown keys.
    pub fn parse(text: &str) -> Result<Self, toml::de::Error> {
        let mut config: Self = toml::from_str(text)?;
        config.sanitise();
        Ok(config)
    }

    /// Fix up values that would break rendering if taken literally.
    ///
    /// A `tab_width` of zero would make column arithmetic divide by zero, and an
    /// enormous `scrolloff` would pin the cursor to the middle of the screen.
    /// Clamping beats rejecting: the editor still starts.
    fn sanitise(&mut self) {
        self.tab_width = self.tab_width.clamp(1, 16);
        self.scrolloff = self.scrolloff.min(32);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_partial_file_keeps_the_remaining_defaults() {
        let config = Config::parse("theme = \"light\"\ntab_width = 2").expect("valid config");
        assert_eq!(config.theme, "light");
        assert_eq!(config.tab_width, 2);
        assert!(config.auto_indent);
        assert!(config.line_numbers);
    }

    #[test]
    fn an_empty_file_is_the_default_config() {
        let config = Config::parse("").expect("valid config");
        assert_eq!(config.theme, Config::default().theme);
    }

    #[test]
    fn unknown_keys_are_reported_rather_than_ignored() {
        let error = Config::parse("tab_wdith = 4").expect_err("typo must be rejected");
        assert!(error.to_string().contains("tab_wdith"));
    }

    #[test]
    fn dangerous_values_are_clamped() {
        let config = Config::parse("tab_width = 0\nscrolloff = 9999").expect("valid config");
        assert_eq!(config.tab_width, 1);
        assert_eq!(config.scrolloff, 32);
    }
}
