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

pub mod persist;

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

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

/// Directory for what the editor writes rather than what the user writes:
/// undo histories, swap files, sessions, command history.
///
/// `TERMI_DATA_DIR` overrides the platform default, for the same reasons as
/// `TERMI_CONFIG_DIR`.
#[must_use]
pub fn data_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("TERMI_DATA_DIR") {
        return PathBuf::from(dir);
    }
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("termi")
}

/// The config file itself.
#[must_use]
pub fn config_file() -> PathBuf {
    config_dir().join("config.toml")
}

/// Directory searched for custom theme files.
#[must_use]
pub fn themes_dir() -> PathBuf {
    config_dir().join("themes")
}

/// Key mappings, one table per mode, from key notation to what the keys do:
/// an action name, a `:command`, or other keys to type instead.
#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct KeyTables {
    /// Mappings for normal mode.
    pub normal: BTreeMap<String, String>,
    /// Mappings for insert mode.
    pub insert: BTreeMap<String, String>,
    /// Mappings for the three visual modes.
    pub visual: BTreeMap<String, String>,
}

/// Settings for every file in one language, under `[languages.<name>]`.
///
/// They sit between the global settings and the file's own: a project's
/// `.editorconfig`, the indentation a file already has, and `:set` still win.
#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct LanguageConfig {
    /// Width of a tab stop and of one indentation level.
    pub tab_width: Option<usize>,
    /// Indent with spaces rather than tabs.
    pub expand_tabs: Option<bool>,
    /// Carry indentation onto new lines.
    pub auto_indent: Option<bool>,
    /// Close brackets and quotes as they are typed.
    pub auto_pairs: Option<bool>,
    /// Strip trailing whitespace on save.
    pub trim_trailing_whitespace: Option<bool>,
    /// End the file with a line break on save.
    pub insert_final_newline: Option<bool>,
}

impl LanguageConfig {
    /// Lay these settings over `config`.
    pub fn apply(&self, config: &mut Config) {
        if let Some(width) = self.tab_width {
            config.tab_width = width.clamp(1, 16);
        }
        if let Some(expand) = self.expand_tabs {
            config.expand_tabs = expand;
        }
        if let Some(indent) = self.auto_indent {
            config.auto_indent = indent;
        }
        if let Some(pairs) = self.auto_pairs {
            config.auto_pairs = pairs;
        }
        if let Some(trim) = self.trim_trailing_whitespace {
            config.trim_trailing_whitespace = trim;
        }
        if let Some(newline) = self.insert_final_newline {
            config.insert_final_newline = newline;
        }
    }
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
    /// Close brackets and quotes as they are typed.
    pub auto_pairs: bool,
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
    /// End every file with a line break on save.
    pub insert_final_newline: bool,
    /// Work out each file's indentation from its contents when it opens.
    pub detect_indent: bool,
    /// Read the `.editorconfig` files a project ships.
    pub editorconfig: bool,
    /// Keep unsaved changes in swap files, for recovery after a crash.
    pub swap_files: bool,
    /// Files larger than this many megabytes open without highlighting,
    /// swap files or persistent undo, so they stay quick to edit.
    pub large_file_mb: u64,
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
    /// Most undo states kept per buffer.
    pub undo_limit: usize,
    /// Keep each file's undo history on disk, so it survives a restart.
    pub persistent_undo: bool,
    /// Draw no background of its own, so the terminal's — a colour, an image,
    /// transparency — shows through.
    pub transparent_background: bool,
    /// Reopen the files, windows and places of the last session in this
    /// directory when started without files.
    pub restore_session: bool,
    /// Key mappings.
    pub keys: KeyTables,
    /// Settings per language, keyed by the language's name.
    pub languages: HashMap<String, LanguageConfig>,
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
            auto_pairs: true,
            word_wrap: false,
            highlight_current_line: true,
            scrolloff: 3,
            syntax_highlighting: true,
            show_tabs: true,
            watch_files: true,
            trim_trailing_whitespace: false,
            insert_final_newline: false,
            detect_indent: true,
            editorconfig: true,
            swap_files: true,
            large_file_mb: 16,
            system_clipboard: true,
            mouse: true,
            undo_limit: 1000,
            persistent_undo: true,
            transparent_background: false,
            restore_session: true,
            keys: KeyTables::default(),
            languages: HashMap::new(),
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
        Self::load_from(&config_file())
    }

    /// Read a config file from somewhere other than the usual place, as
    /// `--config` asks.
    #[must_use]
    pub fn load_from(path: &Path) -> (Self, Option<String>) {
        let path = path.to_path_buf();
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
        self.undo_limit = self.undo_limit.clamp(1, 100_000);
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
    fn key_tables_and_language_sections() {
        let config = Config::parse(
            "[keys.normal]\n\"<Space>w\" = \":w\"\n[keys.insert]\njk = \"normal_mode\"\n\n[languages.python]\ntab_width = 2\n",
        )
        .expect("valid config");
        assert_eq!(
            config.keys.normal.get("<Space>w").map(String::as_str),
            Some(":w")
        );
        assert_eq!(
            config.keys.insert.get("jk").map(String::as_str),
            Some("normal_mode")
        );
        assert_eq!(config.languages["python"].tab_width, Some(2));
        assert!(Config::parse("[languages.python]\ntabwidth = 2").is_err());
    }

    #[test]
    fn the_example_file_is_a_valid_config() {
        let config = Config::parse(include_str!("../../config.example.toml"))
            .expect("config.example.toml must parse");
        assert_eq!(config.theme, Config::default().theme);
    }

    #[test]
    fn dangerous_values_are_clamped() {
        let config = Config::parse("tab_width = 0\nscrolloff = 9999").expect("valid config");
        assert_eq!(config.tab_width, 1);
        assert_eq!(config.scrolloff, 32);
    }
}
