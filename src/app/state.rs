//! # Application state
//!
//! **Purpose:** the single mutable value that the whole editor operates on.
//!
//! **Responsibility:** owns the open buffers, the windows looking at them, the
//! mode, the resolved config and theme, and the transient message shown in the
//! command bar. Every subsystem reads from and writes to this struct; nothing
//! else in the editor keeps global mutable state.
//!
//! Buffers and windows are held side by side rather than nested: a buffer is a
//! file, a window is a view onto one, and several windows can share a buffer.
//! Everything that needs both at once goes through [`App::edit`].
//!
//! **Public API:** [`App`], [`Status`].

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::Result;

use super::mode::Mode;
use crate::clipboard::Clipboard;
use crate::config::{self, Config};
use crate::editor::buffer::{Buffer, BufferId};
use crate::editor::cursor::{Cursor, Motion, Position};
use crate::editor::document::Document;
use crate::editor::edit::Edit;
use crate::editor::window::{Window, WindowId, Windows};
use crate::filesystem::tree::Tree;
use crate::search::Search;
use crate::terminal::Terminal;
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
    /// Open windows and the way they divide the screen. Never empty.
    pub windows: Windows,
    /// Terminal sessions keyed by the editor window they replace on screen.
    ///
    /// Keeping this in the application layer lets the generic window tree stay
    /// independent of processes while editor and terminal panes share all of
    /// its focus, split and resize behaviour.
    terminals: HashMap<WindowId, Terminal>,
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

        let mut app = Self::assemble(config, theme);
        if let Some(message) = config_error.or(theme_error) {
            app.error(message);
        }
        app
    }

    /// Start from `config` without reading anything from disk.
    ///
    /// Test-only: an editor built by [`App::new`] would behave differently
    /// depending on whose machine it ran on.
    #[cfg(test)]
    #[must_use]
    pub fn with_config(config: Config) -> Self {
        let theme = Theme::builtin(&config.theme);
        Self::assemble(config, theme)
    }

    fn assemble(config: Config, theme: Theme) -> Self {
        let clipboard = Clipboard::new(config.system_clipboard);
        Self {
            mode: Mode::default(),
            config,
            theme,
            buffers: vec![Buffer::empty()],
            windows: Windows::new(Window::new(0)),
            terminals: HashMap::new(),
            command_line: String::new(),
            status: Status::default(),
            clipboard,
            search: Search::default(),
            tree: None,
            tree_selected: 0,
            tree_visible: false,
            popup: None,
            quit: false,
        }
    }

    /// The buffer the focused window is showing.
    #[must_use]
    pub fn buffer(&self) -> &Buffer {
        &self.buffers[self.windows.focused().buffer]
    }

    /// Mutable access to the focused buffer.
    pub fn buffer_mut(&mut self) -> &mut Buffer {
        let index = self.windows.focused().buffer;
        &mut self.buffers[index]
    }

    /// The focused window.
    #[must_use]
    pub fn window(&self) -> &Window {
        self.windows.focused()
    }

    /// Mutable access to the focused window.
    pub fn window_mut(&mut self) -> &mut Window {
        self.windows.focused_mut()
    }

    /// Whether `id` currently displays an embedded terminal.
    #[must_use]
    pub fn is_terminal(&self, id: WindowId) -> bool {
        self.terminals.contains_key(&id)
    }

    /// Whether the focused window displays an embedded terminal.
    #[must_use]
    pub fn terminal_focused(&self) -> bool {
        self.is_terminal(self.windows.focus())
    }

    /// The terminal displayed by `id`, if that window has one.
    #[must_use]
    pub fn terminal(&self, id: WindowId) -> Option<&Terminal> {
        self.terminals.get(&id)
    }

    /// Mutable access to the terminal displayed by `id`.
    pub fn terminal_mut(&mut self, id: WindowId) -> Option<&mut Terminal> {
        self.terminals.get_mut(&id)
    }

    /// Open a terminal below the focused window and aim the keyboard at it.
    ///
    /// # Errors
    /// Returns an error without changing the window layout when the process or
    /// pseudo-terminal cannot be started.
    pub fn open_terminal(&mut self, command: Option<&str>) -> Result<WindowId> {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let terminal = Terminal::spawn(command, &cwd)?;
        let id = self.windows.split(crate::editor::window::Axis::Vertical);
        self.terminals.insert(id, terminal);
        self.mode = Mode::Terminal;
        Ok(id)
    }

    /// Send a key press to the focused terminal.
    ///
    /// # Errors
    /// Returns an error if the process has closed its input stream.
    pub fn send_terminal_key(&mut self, key: crossterm::event::KeyEvent) -> Result<()> {
        let id = self.windows.focus();
        if let Some(terminal) = self.terminals.get_mut(&id) {
            terminal.send_key(key)?;
        }
        Ok(())
    }

    /// Send pasted text to the focused terminal.
    ///
    /// # Errors
    /// Returns an error if the process has closed its input stream.
    pub fn paste_terminal(&mut self, text: &str) -> Result<()> {
        let id = self.windows.focus();
        if let Some(terminal) = self.terminals.get_mut(&id) {
            terminal.paste(text)?;
        }
        Ok(())
    }

    /// Scroll one embedded terminal through its saved output.
    pub fn scroll_terminal(&mut self, id: WindowId, delta: isize) {
        if let Some(terminal) = self.terminals.get_mut(&id) {
            terminal.scroll(delta);
        }
    }

    /// Split the focused pane, leaving the new pane as an editor window.
    pub fn split_window(&mut self, axis: crate::editor::window::Axis) -> WindowId {
        let was_terminal = self.terminal_focused();
        let id = self.windows.split(axis);
        if was_terminal {
            self.mode = Mode::Normal;
        }
        id
    }

    /// Close a pane, or reveal its editor view when it is the last terminal.
    pub fn close_window(&mut self, id: WindowId) -> bool {
        if self.windows.count() == 1 && self.terminals.remove(&id).is_some() {
            self.mode = Mode::Normal;
            return true;
        }
        if !self.windows.close(id) {
            return false;
        }
        let removed_terminal = self.terminals.remove(&id).is_some();
        if self.terminal_focused() {
            self.mode = Mode::Terminal;
        } else if removed_terminal || self.mode == Mode::Terminal {
            self.mode = Mode::Normal;
        }
        true
    }

    /// Close every pane except the focused one and stop their terminal sessions.
    pub fn close_other_windows(&mut self) {
        let focus = self.windows.focus();
        self.windows.close_others();
        self.terminals.retain(|id, _| *id == focus);
        if self.terminal_focused() {
            self.mode = Mode::Terminal;
        } else if self.mode == Mode::Terminal {
            self.mode = Mode::Normal;
        }
    }

    /// Match the modal state to the kind of content in the focused pane.
    pub fn sync_mode_to_focus(&mut self) {
        self.mode = if self.terminal_focused() {
            Mode::Terminal
        } else {
            Mode::Normal
        };
    }

    /// The focused buffer and window, borrowed together for one edit.
    pub fn edit(&mut self) -> Edit<'_> {
        let index = self.windows.focused().buffer;
        let Self {
            buffers, windows, ..
        } = self;
        Edit::new(&mut buffers[index], windows.focused_mut())
    }

    /// An edit alongside the settings, borrowed as disjoint fields so an
    /// operation can read the config while mutating the text.
    pub fn edit_and_config(&mut self) -> (Edit<'_>, &Config) {
        let index = self.windows.focused().buffer;
        let Self {
            buffers,
            windows,
            config,
            ..
        } = self;
        (
            Edit::new(&mut buffers[index], windows.focused_mut()),
            config,
        )
    }

    /// Move every cursor in the focused window.
    ///
    /// A window cannot reach its own document, so the two are paired up here
    /// rather than at every call site.
    pub fn move_cursors(&mut self, motion: Motion, extend: bool, allow_eol: bool) {
        let index = self.windows.focused().buffer;
        let Self {
            buffers, windows, ..
        } = self;
        windows
            .focused_mut()
            .move_cursors(motion, &buffers[index].document, extend, allow_eol);
    }

    /// Pull the focused window's cursors back inside its document.
    pub fn clamp_cursors(&mut self, allow_eol: bool) {
        let index = self.windows.focused().buffer;
        let Self {
            buffers, windows, ..
        } = self;
        windows
            .focused_mut()
            .clamp_cursors(&buffers[index].document, allow_eol);
    }

    /// Pull the cursors of every window showing `buffer` back inside it.
    ///
    /// An edit made in one window moves the text underneath every other window
    /// on the same file, and those windows are not otherwise told about it.
    pub fn clamp_windows_on(&mut self, buffer: BufferId) {
        let Self {
            buffers, windows, ..
        } = self;
        for (_, window) in windows.iter_mut() {
            if window.buffer == buffer {
                window.clamp_cursors(&buffers[buffer].document, false);
            }
        }
    }

    /// Open `path` in the focused window, reusing the buffer if it is already
    /// open somewhere.
    ///
    /// # Errors
    /// Returns an error if the file exists but cannot be read.
    pub fn open(&mut self, path: PathBuf) -> Result<()> {
        if let Some(index) = self.index_of(&path) {
            self.windows.focused_mut().show(index);
            return Ok(());
        }
        let buffer = Buffer::new(Document::open(path)?);

        // The initial scratch buffer is a placeholder, not a document the user
        // asked for; replace it rather than accumulating an empty tab.
        if self.buffers.len() == 1 && self.is_scratch(0) {
            self.buffers[0] = buffer;
            for (_, window) in self.windows.iter_mut() {
                window.reset(0);
            }
        } else {
            self.buffers.push(buffer);
            let last = self.buffers.len() - 1;
            self.windows.focused_mut().show(last);
        }
        Ok(())
    }

    /// Close the focused buffer, keeping at least one open.
    ///
    /// Every window is repaired, not just the focused one: buffer ids are
    /// positions in the buffer list, so closing one renumbers the rest.
    pub fn close_active(&mut self) {
        let index = self.windows.focused().buffer;
        if self.buffers.len() == 1 {
            self.buffers[0] = Buffer::empty();
            for (_, window) in self.windows.iter_mut() {
                window.reset(0);
            }
            return;
        }
        self.buffers.remove(index);
        // After the removal everything above the hole has shifted down, so the
        // buffer that followed the closed one now sits at its index.
        let fallback = index.min(self.buffers.len() - 1);
        for (_, window) in self.windows.iter_mut() {
            window.buffer_removed(index, fallback);
        }
    }

    /// Show the next or previous buffer in the focused window, wrapping around.
    pub fn cycle_buffer(&mut self, forward: bool) {
        let count = self.buffers.len();
        let current = self.windows.focused().buffer;
        let next = if forward {
            (current + 1) % count
        } else {
            (current + count - 1) % count
        };
        self.windows.focused_mut().show(next);
    }

    /// Whether a modeless selection is standing in the focused window.
    ///
    /// "Modeless" is the point: a selection made with Shift or the mouse lives
    /// in whatever mode the user was already in, so this asks the cursors rather
    /// than the mode. Visual mode is excluded because its selection is a
    /// different shape — inclusive of the character under the caret — and every
    /// caller here means the exclusive one.
    #[must_use]
    pub fn has_selection(&self) -> bool {
        !self.terminal_focused()
            && !self.mode.is_visual()
            && self.window().cursors().iter().any(Cursor::has_selection)
    }

    /// Whether any buffer has unsaved changes.
    #[must_use]
    pub fn has_unsaved_changes(&self) -> bool {
        self.buffers.iter().any(|b| b.document.is_dirty())
    }

    /// Scroll one window, dragging its caret along only far enough to keep it
    /// on screen.
    ///
    /// Without the second half the caret would stay put, and the next frame
    /// would scroll straight back to it — a view that cannot be moved
    /// independently of the caret cannot be scrolled at all.
    pub fn scroll_window(&mut self, id: WindowId, delta: isize) {
        let index = self.windows.get(id).buffer;
        let scrolloff = self.config.scrolloff;
        let Self {
            buffers, windows, ..
        } = self;
        let document = &buffers[index].document;
        let window = windows.get_mut(id);

        window.view.scroll_lines(delta, document.last_line());

        let height = usize::from(window.area.height).max(1);
        let margin = scrolloff.min(height.saturating_sub(1) / 2);
        let top = window.view.top_line;
        let bottom = (top + height - 1).min(document.last_line());

        // Keep the same margin the renderer would enforce, so the caret does not
        // land somewhere that immediately scrolls the view again.
        let lowest = (top + margin).min(bottom);
        let highest = bottom.saturating_sub(margin).max(lowest);

        let head = window.cursor().head;
        let line = head.line.clamp(lowest, highest);
        if line != head.line {
            let goal = window.cursor().goal_col();
            let position = document.clamp(Position::new(line, goal), false);
            window.cursor_mut().move_to(position, false);
            window.cursor_mut().set_goal_col(goal);
        }
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

    /// Re-read the file tree, keeping the highlighted row on the same file.
    ///
    /// Rows shift when something appears above them, so following the index
    /// would move the selection under the user; following the path leaves it
    /// where they left it, and only a selection that no longer exists falls back
    /// to the index.
    pub fn refresh_tree(&mut self) {
        let Some(tree) = self.tree.as_mut() else {
            return;
        };
        let selected = tree.path_at(self.tree_selected).map(Path::to_path_buf);
        tree.refresh();
        let found = selected.and_then(|path| tree.index_of(&path));
        let last = tree.entries().len().saturating_sub(1);
        self.tree_selected = found.unwrap_or(self.tree_selected).min(last);
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
