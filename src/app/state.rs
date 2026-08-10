//! # Application state
//!
//! **Purpose:** the single mutable value that the whole editor operates on.
//!
//! **Responsibility:** owns the open buffers, the window looking at them, the
//! mode, the resolved config and theme, and the transient message shown in the
//! command bar. Every subsystem reads from and writes to this struct; nothing
//! else in the editor keeps global mutable state.
//!
//! Buffers and windows are held side by side rather than nested: a buffer is a
//! file, a window is a view onto one, and the two have different lifetimes.
//! Everything that needs both at once goes through [`App::edit`].
//!
//! **Public API:** [`App`], [`Status`].

use std::path::{Path, PathBuf};

use anyhow::Result;

use super::mode::Mode;
use crate::clipboard::Clipboard;
use crate::config::{self, Config};
use crate::editor::buffer::{Buffer, BufferId};
use crate::editor::cursor::Motion;
use crate::editor::document::Document;
use crate::editor::edit::Edit;
use crate::editor::window::Window;
use crate::filesystem::tree::Tree;
use crate::search::Search;
use crate::theme::Theme;

/// A one-line message shown in the command bar until the next keystroke.
#[derive(Debug, Clone, Default)]
pub struct Status {
    /// Text to display; empty means nothing to show.
    pub text: String,
    /// Render with the error style.
    pub is_error: bool,
}

/// Editor-wide state.
#[derive(Debug)]
pub struct App {
    /// Current modal state.
    pub mode: Mode,
    /// Resolved user settings.
    pub config: Config,
    /// Resolved colour scheme.
    pub theme: Theme,
    /// Open buffers, in tab order. Never empty.
    pub buffers: Vec<Buffer>,
    /// The window the keyboard is aimed at.
    pub window: Window,
    /// Text typed after `:` while in command mode.
    pub command_line: String,
    /// Message shown in the command bar.
    pub status: Status,
    /// Yank register, backed by the system clipboard when one is available.
    pub clipboard: Clipboard,
    /// Incremental search state, kept across searches so `n` can repeat one.
    pub search: Search,
    /// File browser, built on first use and kept afterwards so the expanded
    /// directories survive toggling the panel.
    pub tree: Option<Tree>,
    /// Highlighted row in the file tree.
    pub tree_selected: usize,
    /// Whether the file tree panel is drawn.
    pub tree_visible: bool,
    /// Modal message: title and body. Any key dismisses it.
    pub popup: Option<(String, String)>,
    quit: bool,
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

impl App {
    /// Start with configuration loaded from disk and one empty buffer.
    ///
    /// Configuration problems become a status message rather than a failure —
    /// the editor is what the user needs in order to fix them.
    #[must_use]
    pub fn new() -> Self {
        let (config, config_error) = Config::load();
        let (theme, theme_error) = Theme::load(&config.theme, &config::themes_dir());

        let clipboard = Clipboard::new(config.system_clipboard);
        let mut app = Self {
            mode: Mode::default(),
            config,
            theme,
            buffers: vec![Buffer::empty()],
            window: Window::new(0),
            command_line: String::new(),
            status: Status::default(),
            clipboard,
            search: Search::default(),
            tree: None,
            tree_selected: 0,
            tree_visible: false,
            popup: None,
            quit: false,
        };
        if let Some(message) = config_error.or(theme_error) {
            app.error(message);
        }
        app
    }

    /// The buffer the focused window is showing.
    #[must_use]
    pub fn buffer(&self) -> &Buffer {
        &self.buffers[self.window.buffer]
    }

    /// Mutable access to the focused buffer.
    pub fn buffer_mut(&mut self) -> &mut Buffer {
        let index = self.window.buffer;
        &mut self.buffers[index]
    }

    /// The focused window.
    #[must_use]
    pub const fn window(&self) -> &Window {
        &self.window
    }

    /// Mutable access to the focused window.
    pub fn window_mut(&mut self) -> &mut Window {
        &mut self.window
    }

    /// The focused buffer and window, borrowed together for one edit.
    pub fn edit(&mut self) -> Edit<'_> {
        let index = self.window.buffer;
        Edit::new(&mut self.buffers[index], &mut self.window)
    }

    /// An edit alongside the settings, borrowed as disjoint fields so an
    /// operation can read the config while mutating the text.
    pub fn edit_and_config(&mut self) -> (Edit<'_>, &Config) {
        let index = self.window.buffer;
        let Self {
            buffers,
            window,
            config,
            ..
        } = self;
        (Edit::new(&mut buffers[index], window), config)
    }

    /// Move every cursor in the focused window.
    ///
    /// A window cannot reach its own document, so the two are paired up here
    /// rather than at every call site.
    pub fn move_cursors(&mut self, motion: Motion, extend: bool, allow_eol: bool) {
        let index = self.window.buffer;
        let Self {
            buffers, window, ..
        } = self;
        window.move_cursors(motion, &buffers[index].document, extend, allow_eol);
    }

    /// Pull the focused window's cursors back inside its document.
    pub fn clamp_cursors(&mut self, allow_eol: bool) {
        let index = self.window.buffer;
        let Self {
            buffers, window, ..
        } = self;
        window.clamp_cursors(&buffers[index].document, allow_eol);
    }

    /// Pull the cursors of every window showing `buffer` back inside it.
    ///
    /// A reload or an edit made elsewhere can shrink a file underneath windows
    /// that did not make the change and are not otherwise told about it.
    pub fn clamp_windows_on(&mut self, buffer: BufferId) {
        if self.window.buffer == buffer {
            self.clamp_cursors(false);
        }
    }

    /// Open `path`, focusing it if it is already open.
    ///
    /// # Errors
    /// Returns an error if the file exists but cannot be read.
    pub fn open(&mut self, path: PathBuf) -> Result<()> {
        if let Some(index) = self.index_of(&path) {
            self.window.show(index);
            return Ok(());
        }
        let buffer = Buffer::new(Document::open(path)?);

        // The initial scratch buffer is a placeholder, not a document the user
        // asked for; replace it rather than accumulating an empty tab.
        if self.buffers.len() == 1 && self.is_scratch(0) {
            self.buffers[0] = buffer;
            self.window.reset(0);
        } else {
            self.buffers.push(buffer);
            self.window.show(self.buffers.len() - 1);
        }
        Ok(())
    }

    /// Close the focused buffer, keeping at least one open.
    pub fn close_active(&mut self) {
        let index = self.window.buffer;
        if self.buffers.len() == 1 {
            self.buffers[0] = Buffer::empty();
            self.window.reset(0);
            return;
        }
        self.buffers.remove(index);
        // After the removal everything above the hole has shifted down, so the
        // buffer that followed the closed one now sits at its index.
        let fallback = index.min(self.buffers.len() - 1);
        self.window.buffer_removed(index, fallback);
    }

    /// Focus the next or previous tab, wrapping around.
    pub fn cycle_buffer(&mut self, forward: bool) {
        let count = self.buffers.len();
        let current = self.window.buffer;
        let next = if forward {
            (current + 1) % count
        } else {
            (current + count - 1) % count
        };
        self.window.show(next);
    }

    /// Whether any buffer has unsaved changes.
    #[must_use]
    pub fn has_unsaved_changes(&self) -> bool {
        self.buffers.iter().any(|b| b.document.is_dirty())
    }

    /// Show an informational message.
    pub fn info(&mut self, text: impl Into<String>) {
        self.status = Status {
            text: text.into(),
            is_error: false,
        };
    }

    /// Show an error message.
    pub fn error(&mut self, text: impl Into<String>) {
        self.status = Status {
            text: text.into(),
            is_error: true,
        };
    }

    /// Clear any message currently on show.
    pub fn clear_status(&mut self) {
        self.status = Status::default();
    }

    /// Show a modal message.
    pub fn show_popup(&mut self, title: impl Into<String>, body: impl Into<String>) {
        self.popup = Some((title.into(), body.into()));
    }

    /// The directory the file tree should show.
    ///
    /// The active file's own directory is the useful default; falling back to
    /// the working directory keeps the panel usable for an unnamed buffer.
    pub fn tree_root(&self) -> PathBuf {
        self.buffer()
            .document
            .path()
            .and_then(Path::parent)
            .map(Path::to_path_buf)
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
    }

    /// Request shutdown; the event loop exits once the current event is done.
    pub fn quit(&mut self) {
        self.quit = true;
    }

    /// Whether the event loop should stop.
    #[must_use]
    pub const fn should_quit(&self) -> bool {
        self.quit
    }

    fn index_of(&self, path: &Path) -> Option<BufferId> {
        self.buffers
            .iter()
            .position(|b| b.document.path() == Some(path))
    }

    /// An untouched, unnamed buffer — the one opened at startup.
    fn is_scratch(&self, index: BufferId) -> bool {
        let document = &self.buffers[index].document;
        document.path().is_none() && !document.is_dirty() && document.len_chars() == 0
    }
}
