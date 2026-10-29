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

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};

use crossterm::event::KeyEvent;

use anyhow::Result;

use super::mode::Mode;
use crate::clipboard::registers::Registers;
use crate::config::{self, Config};
use crate::editor::buffer::{Buffer, BufferId};
use crate::editor::cursor::{Cursor, Motion, Position};
use crate::editor::document::Document;
use crate::editor::edit::Edit;
use crate::editor::window::{Window, WindowId, Windows};
use crate::filesystem::tree::Tree;
use crate::input::Action;
use crate::picker::{Picker, PickerItem, PickerKind, PickerTarget, project_files};
use crate::search::Search;
use crate::terminal::Terminal;
use crate::theme::Theme;

/// A write that has to go through `sudo`, waiting for the event loop, which
/// owns the terminal `sudo` needs to ask for a password on.
#[derive(Debug, Clone)]
pub struct SudoWrite {
    /// The buffer being written, to mark clean once it is.
    pub buffer: BufferId,
    /// Where to write.
    pub path: PathBuf,
    /// The encoded text.
    pub bytes: Vec<u8>,
}

/// What the command line asked for at startup that changes how the editor
/// sets itself up.
#[derive(Debug, Clone, Default)]
pub struct Startup {
    /// Read this config file instead of the usual one.
    pub config_path: Option<PathBuf>,
    /// Ignore the config file and any saved session.
    pub clean: bool,
    /// Do not restore or save a session.
    pub no_session: bool,
    /// Open every file read-only.
    pub readonly: bool,
}

/// How often a modified buffer's swap file is rewritten at most.
const SWAP_INTERVAL: std::time::Duration = std::time::Duration::from_secs(2);

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
    /// vi registers; the unnamed one is the system clipboard when available.
    pub registers: Registers,
    /// Marks `A` to `Z`, which remember a file as well as a place in it.
    pub global_marks: HashMap<char, (PathBuf, Position)>,
    /// The register a macro is being recorded into, and the keys so far.
    pub macro_recording: Option<(char, Vec<KeyEvent>)>,
    /// The macro `@@` repeats.
    pub last_macro: Option<char>,
    /// Keys waiting to be handled as though typed — a macro being played.
    pub pending_keys: VecDeque<KeyEvent>,
    /// The actions `.` repeats.
    pub last_change: Vec<Action>,
    /// The change being recorded for `.`, while its insert session lasts.
    pub change_recording: Option<Vec<Action>>,
    /// Whether `.` is replaying, so the replay does not record itself.
    pub replaying: bool,
    /// Whether a `c` operator's undo group is still waiting for its insert
    /// session to end, so the deletion and the typing undo together.
    pub insert_group_open: bool,
    /// The keys of a command still being typed, shown in the status bar.
    pub showcmd: String,
    /// A write waiting to be run through `sudo`.
    pub pending_sudo: Option<SudoWrite>,
    /// The user's key mappings.
    pub mappings: crate::input::mappings::Mappings,
    /// Whether the mappings changed since the input handler last took them.
    pub mappings_changed: bool,
    /// How the editor was started.
    pub startup: Startup,
    /// A `:s///c` asking about one match at a time.
    pub replace_session: Option<super::substitute::ReplaceSession>,
    /// A Tab completion in progress on the command line.
    pub completion: Option<super::complete::Completion>,
    /// Command lines and searches, kept across sessions.
    pub history: super::complete::History,
    /// Which history entry Up and Down are showing.
    pub history_cursor: Option<usize>,
    /// What was typed before Up began travelling through the history.
    pub history_draft: String,
    /// Language servers and what they have said.
    pub lsp: super::lsp::LspState,
    /// An editor that leaves no trace: no swap files, undo histories,
    /// sessions or history on disk, and no git or language server processes.
    /// What tests, benchmarks and fuzzing run.
    pub ephemeral: bool,
    /// Incremental search state, kept across searches so `n` can repeat one.
    pub search: Search,
    /// File browser, built on first use and kept afterwards so the expanded
    /// directories survive toggling the panel.
    pub tree: Option<Tree>,
    /// Highlighted row in the file tree.
    pub tree_selected: usize,
    /// Git's status letter for each changed path the tree may show.
    pub tree_status: HashMap<PathBuf, char>,
    /// Whether the file tree panel is drawn.
    pub tree_visible: bool,
    /// Tree entry awaiting a destination from the command line.
    pub tree_source: Option<PathBuf>,
    /// Tree entry awaiting permanent deletion.
    pub pending_delete: Option<PathBuf>,
    /// Modal message: title and body. Any key dismisses it.
    pub popup: Option<(String, String)>,
    /// Active searchable list.
    pub picker: Option<Picker>,
    recent_files: Vec<PathBuf>,
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
        Self::start(Startup::default())
    }

    /// Start the way the command line asked.
    #[must_use]
    pub fn start(startup: Startup) -> Self {
        let (config, config_error) = if startup.clean {
            (Config::default(), None)
        } else {
            match &startup.config_path {
                Some(path) => Config::load_from(path),
                None => Config::load(),
            }
        };
        let (theme, theme_error) = load_theme(&config);

        let mut app = Self::assemble(config, theme);
        app.startup = startup;
        app.history = super::complete::History::load();
        let mapping_error = app.compile_mappings();
        let script_error = crate::syntax::load_errors().first().cloned();
        if let Some(message) = config_error
            .or(theme_error)
            .or(mapping_error)
            .or(script_error)
        {
            app.error(message);
        }
        app
    }

    /// Compile the key mappings from the config, returning the first problem.
    fn compile_mappings(&mut self) -> Option<String> {
        let (mappings, errors) = crate::input::mappings::Mappings::compile(&self.config.keys);
        self.mappings = mappings;
        self.mappings_changed = true;
        errors.into_iter().next()
    }

    /// Read the config file again and apply it: settings, theme and key
    /// mappings. Open buffers keep their own settings.
    pub fn reload_config(&mut self) {
        let (config, error) = match &self.startup.config_path {
            Some(path) => Config::load_from(path),
            None => Config::load(),
        };
        if let Some(error) = error {
            self.error(error);
            return;
        }
        self.config = config;
        let (theme, theme_error) = load_theme(&self.config);
        self.theme = theme;
        if let Some(error) = theme_error.or_else(|| self.compile_mappings()) {
            self.error(error);
            return;
        }
        self.info("config reloaded");
    }

    /// An [`ephemeral`](App::ephemeral) editor on `config`, reading nothing
    /// from disk: one built by [`App::new`] would behave differently depending
    /// on whose machine it ran on, and would leave files behind.
    #[must_use]
    pub fn headless(config: Config) -> Self {
        let mut theme = Theme::builtin(&config.theme);
        if config.transparent_background {
            theme.make_transparent();
        }
        let mut app = Self::assemble(config, theme);
        app.ephemeral = true;
        app.compile_mappings();
        app
    }

    /// [`App::headless`], under the name the unit tests know it by.
    #[cfg(test)]
    #[must_use]
    pub fn with_config(config: Config) -> Self {
        Self::headless(config)
    }

    fn assemble(config: Config, theme: Theme) -> Self {
        let registers = Registers::new(config.system_clipboard);
        Self {
            mode: Mode::default(),
            config,
            theme,
            buffers: vec![Buffer::empty()],
            windows: Windows::new(Window::new(0)),
            terminals: HashMap::new(),
            command_line: String::new(),
            status: Status::default(),
            registers,
            global_marks: HashMap::new(),
            macro_recording: None,
            last_macro: None,
            pending_keys: VecDeque::new(),
            last_change: Vec::new(),
            change_recording: None,
            replaying: false,
            insert_group_open: false,
            showcmd: String::new(),
            pending_sudo: None,
            mappings: crate::input::mappings::Mappings::default(),
            mappings_changed: false,
            startup: Startup::default(),
            replace_session: None,
            completion: None,
            history: super::complete::History::default(),
            history_cursor: None,
            history_draft: String::new(),
            lsp: super::lsp::LspState::default(),
            ephemeral: false,
            search: Search::default(),
            tree: None,
            tree_selected: 0,
            tree_status: HashMap::new(),
            tree_visible: false,
            tree_source: None,
            pending_delete: None,
            popup: None,
            picker: None,
            recent_files: Vec::new(),
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

    /// The settings in force for one buffer: the global config with what its
    /// `.editorconfig`, its detected indentation and `:set` changed on top.
    #[must_use]
    pub fn buffer_config(&self, buffer: BufferId) -> Config {
        self.buffers[buffer].config(&self.config)
    }

    /// The focused buffer and window, borrowed together for one edit.
    pub fn edit(&mut self) -> Edit<'_> {
        let index = self.windows.focused().buffer;
        let Self {
            buffers, windows, ..
        } = self;
        Edit::new(&mut buffers[index], windows.focused_mut())
    }

    /// An edit alongside the settings in force for its buffer.
    pub fn edit_and_config(&mut self) -> (Edit<'_>, Config) {
        let index = self.windows.focused().buffer;
        let config = self.buffer_config(index);
        let Self {
            buffers, windows, ..
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
        let index = self.load(path.clone())?;
        self.windows.focused_mut().show(index);
        self.record_recent(path);
        Ok(())
    }

    /// The buffer for `path`, opening it without showing it if needed — for
    /// work that touches files the user is not looking at, such as a
    /// project-wide replace.
    ///
    /// # Errors
    /// Returns an error if the file exists but cannot be read.
    pub fn load(&mut self, path: PathBuf) -> Result<BufferId> {
        if let Some(index) = self.index_of(&path) {
            return Ok(index);
        }
        let existed = path.exists();
        let mut buffer = Buffer::new(Document::open(path.clone())?);
        if self.startup.readonly {
            buffer.document.set_readonly(true);
        }
        let limit = self.config.large_file_mb.saturating_mul(1 << 20);
        buffer.large = std::fs::metadata(&path).is_ok_and(|meta| meta.len() > limit);
        self.configure_buffer(&mut buffer, existed);
        if self.config.git && !buffer.large && !self.ephemeral {
            buffer.refresh_git();
        }
        let undo_dir = self.undo_dir().filter(|_| !buffer.large);
        match undo_dir {
            Some(directory) => buffer.restore_history(&directory, self.config.undo_limit),
            None => buffer.history.set_limit(self.config.undo_limit),
        }
        if buffer.large {
            buffer.syntax.set_language(None);
            self.info(format!(
                "{} is large — highlighting is off for it",
                buffer.document.display_name()
            ));
        }
        self.check_swap(&buffer);

        // The initial scratch buffer is a placeholder, not a document the user
        // asked for; replace it rather than accumulating an empty tab.
        let index = if self.buffers.len() == 1 && self.is_scratch(0) {
            self.buffers[0] = buffer;
            for (_, window) in self.windows.iter_mut() {
                window.reset(0);
            }
            0
        } else {
            self.buffers.push(buffer);
            self.buffers.len() - 1
        };
        super::lsp::attach(self, index);
        Ok(index)
    }

    /// Settle a freshly opened buffer's own settings: what `.editorconfig`
    /// says, and failing that how the file is already indented.
    fn configure_buffer(&self, buffer: &mut Buffer, existed: bool) {
        if self.config.detect_indent
            && let Some((spaces, width)) = crate::editor::document::indent::detect(&buffer.document)
        {
            buffer.options.expand_tabs = Some(spaces);
            if spaces {
                buffer.options.tab_width = Some(width);
            }
        }
        if !self.config.editorconfig {
            return;
        }
        let Some(path) = buffer.document.path() else {
            return;
        };
        let properties = crate::filesystem::editorconfig::properties_for(path);
        buffer.options.absorb(&properties);
        // An existing file keeps the line endings and encoding it has; only a
        // new one is created the way the project asks.
        if !existed {
            if let Some(ending) = properties.end_of_line {
                buffer.document.set_line_ending(ending);
            }
            if let Some(encoding) = properties.charset {
                buffer.document.set_encoding(encoding);
            }
            buffer.document.set_dirty(false);
        }
    }

    /// Where swap files go, when they are wanted at all.
    #[must_use]
    pub fn swap_dir(&self) -> Option<PathBuf> {
        if self.ephemeral || !self.config.swap_files {
            return None;
        }
        Some(config::data_dir().join("swap"))
    }

    /// Tell the user about unsaved work a swap file holds for `buffer`.
    fn check_swap(&mut self, buffer: &Buffer) {
        let (Some(directory), Some(path)) = (self.swap_dir(), buffer.document.path()) else {
            return;
        };
        let Some((swap, written)) = crate::filesystem::swap::read(&directory, path) else {
            return;
        };
        if swap.pid == std::process::id() {
            return;
        }
        let name = buffer.document.display_name();
        if swap.writer_alive() {
            self.show_popup(
                "file already open",
                format!(
                    "{name} is being edited in another termi (process {}).\n\nChanges made in both will overwrite each other.",
                    swap.pid
                ),
            );
            return;
        }
        if buffer
            .document
            .to_disk_bytes()
            .is_ok_and(|bytes| bytes == swap.text.as_bytes())
        {
            crate::filesystem::swap::remove(&directory, path);
            return;
        }
        let age = written.elapsed().map_or_else(
            |_| "some time".to_string(),
            |elapsed| describe_age(elapsed.as_secs()),
        );
        self.show_popup(
            "unsaved changes found",
            format!(
                "termi did not close cleanly while {name} had unsaved changes, {age} ago.\n\n:recover brings them back; :dropswap throws them away."
            ),
        );
    }

    /// Bring the git signs of every buffer on screen up to date with its text.
    pub fn update_git(&mut self) {
        const INTERVAL: std::time::Duration = std::time::Duration::from_millis(150);
        if !self.config.git {
            return;
        }
        let shown: Vec<BufferId> = self
            .windows
            .iter()
            .map(|(_, window)| window.buffer)
            .collect();
        for index in shown {
            if let Some(buffer) = self.buffers.get_mut(index) {
                buffer.update_git_signs(INTERVAL);
            }
        }
    }

    /// Write swap files for buffers changed since their last one, and drop
    /// the swap files of buffers that are clean again.
    pub fn write_swaps(&mut self) {
        let Some(directory) = self.swap_dir() else {
            return;
        };
        let now = std::time::Instant::now();
        for buffer in &mut self.buffers {
            let Some(path) = buffer.document.path().map(Path::to_path_buf) else {
                continue;
            };
            if buffer.large {
                continue;
            }
            // A swap file written under the buffer's old name is stale
            // either way once the buffer has a new one.
            if let Some((_, written_for)) = &buffer.swap
                && (*written_for != path || !buffer.document.is_dirty())
            {
                crate::filesystem::swap::remove(&directory, written_for);
                buffer.swap = None;
            }
            if !buffer.document.is_dirty() {
                continue;
            }
            let revision = buffer.document.revision();
            if buffer
                .swap
                .as_ref()
                .is_some_and(|(written, _)| *written == revision)
                || buffer
                    .swap_written
                    .is_some_and(|written| now.duration_since(written) < SWAP_INTERVAL)
            {
                continue;
            }
            let text = buffer.document.text().to_string();
            if crate::filesystem::swap::write(&directory, &path, &text).is_ok() {
                buffer.swap = Some((revision, path));
                buffer.swap_written = Some(now);
            }
        }
    }

    /// Remove every swap file this editor wrote, on the way out.
    pub fn remove_swaps(&mut self) {
        let Some(directory) = self.swap_dir() else {
            return;
        };
        for buffer in &mut self.buffers {
            if let Some((_, path)) = buffer.swap.take() {
                crate::filesystem::swap::remove(&directory, &path);
            }
        }
    }

    pub fn open_picker(&mut self, kind: PickerKind) {
        let picker = match kind {
            PickerKind::Files => self.file_picker(),
            PickerKind::Buffers => self.buffer_picker(),
            PickerKind::Commands => command_picker(),
            PickerKind::Themes => theme_picker(),
            PickerKind::Grep => Picker::live("grep"),
        };
        self.picker = Some(picker);
        self.mode = Mode::Picker;
    }

    /// Open the project search, with a query already typed if one is given.
    pub fn open_grep(&mut self, query: Option<&str>) {
        self.open_picker(PickerKind::Grep);
        if let (Some(picker), Some(query)) = (self.picker.as_mut(), query) {
            picker.query = query.to_string();
        }
        self.refresh_live_picker();
    }

    /// Search the project again for a live picker's query.
    ///
    /// Queries shorter than two characters match too much to be useful and
    /// would walk the whole project on every keystroke, so they show nothing.
    pub fn refresh_live_picker(&mut self) {
        const HIT_LIMIT: usize = 500;
        let Some(picker) = self.picker.as_mut().filter(|picker| picker.live) else {
            return;
        };
        if picker.query.chars().count() < 2 {
            picker.set_items(Vec::new());
            return;
        }
        let Ok(matcher) = crate::search::Matcher::new(
            &picker.query,
            self.search.regex,
            self.search.case_sensitive,
        ) else {
            picker.set_items(Vec::new());
            return;
        };
        let root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let items = crate::search::project::search(&root, &matcher, HIT_LIMIT)
            .into_iter()
            .map(|hit| {
                let name = hit
                    .path
                    .strip_prefix(&root)
                    .unwrap_or(&hit.path)
                    .display()
                    .to_string();
                PickerItem::new(
                    format!("{name}:{}: {}", hit.line + 1, hit.text),
                    PickerTarget::Location(hit.path, Position::new(hit.line, hit.col)),
                )
            })
            .collect();
        picker.set_items(items);
    }

    fn file_picker(&self) -> Picker {
        let root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let items = project_files(&root)
            .into_iter()
            .map(|path| {
                let label = path
                    .strip_prefix(&root)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .into_owned();
                let priority = self
                    .recent_files
                    .iter()
                    .position(|recent| recent == &path)
                    .unwrap_or(usize::MAX);
                PickerItem::new(label, PickerTarget::File(path)).prioritised(priority)
            })
            .collect();
        Picker::new("files", items)
    }

    fn buffer_picker(&self) -> Picker {
        let items = self
            .buffers
            .iter()
            .enumerate()
            .map(|(index, buffer)| {
                let label = buffer.document.path().map_or_else(
                    || buffer.document.display_name().to_string(),
                    |path| path.display().to_string(),
                );
                PickerItem::new(label, PickerTarget::Buffer(index))
            })
            .collect();
        Picker::new("buffers", items)
    }

    fn record_recent(&mut self, path: PathBuf) {
        self.recent_files.retain(|recent| recent != &path);
        self.recent_files.insert(0, path);
        self.recent_files.truncate(100);
    }

    /// Close the focused buffer, keeping at least one open.
    ///
    /// Every window is repaired, not just the focused one: buffer ids are
    /// positions in the buffer list, so closing one renumbers the rest.
    pub fn close_active(&mut self) {
        let index = self.windows.focused().buffer;
        self.close_buffer(index);
    }

    fn close_buffer(&mut self, index: BufferId) {
        super::lsp::detach(self, index);
        if let (Some(directory), Some((_, path))) = (self.swap_dir(), &self.buffers[index].swap) {
            crate::filesystem::swap::remove(&directory, path);
        }
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

    pub fn relocate_buffers(&mut self, source: &Path, destination: &Path) {
        for buffer in &mut self.buffers {
            let Some(path) = buffer.document.path() else {
                continue;
            };
            let Ok(relative) = path.strip_prefix(source) else {
                continue;
            };
            let path = if relative.as_os_str().is_empty() {
                destination.to_path_buf()
            } else {
                destination.join(relative)
            };
            buffer.document.relocate(path);
            buffer.detect_language();
        }
    }

    #[must_use]
    pub fn has_dirty_under(&self, path: &Path) -> bool {
        self.buffers.iter().any(|buffer| {
            buffer.document.is_dirty()
                && buffer
                    .document
                    .path()
                    .is_some_and(|buffer_path| buffer_path.starts_with(path))
        })
    }

    pub fn close_buffers_under(&mut self, path: &Path) {
        let indexes: Vec<_> = self
            .buffers
            .iter()
            .enumerate()
            .filter_map(|(index, buffer)| {
                buffer
                    .document
                    .path()
                    .is_some_and(|buffer_path| buffer_path.starts_with(path))
                    .then_some(index)
            })
            .collect();
        for index in indexes.into_iter().rev() {
            self.close_buffer(index);
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

    /// Where persistent undo histories go, when they are wanted at all.
    ///
    /// An ephemeral editor never writes there: its files are throwaway, and
    /// the directory is the real user's.
    #[must_use]
    pub fn undo_dir(&self) -> Option<PathBuf> {
        if self.ephemeral || !self.config.persistent_undo {
            return None;
        }
        Some(config::data_dir().join("undo"))
    }

    /// What to tell the user when unsaved work blocks quitting, naming the
    /// files so they know which ones to look at; `None` when nothing is dirty.
    #[must_use]
    pub fn unsaved_message(&self) -> Option<String> {
        let names: Vec<&str> = self
            .buffers
            .iter()
            .filter(|buffer| buffer.document.is_dirty())
            .map(|buffer| buffer.document.display_name())
            .collect();
        if names.is_empty() {
            return None;
        }
        const SHOWN: usize = 3;
        let mut list = names
            .iter()
            .take(SHOWN)
            .copied()
            .collect::<Vec<_>>()
            .join(", ");
        if names.len() > SHOWN {
            list.push_str(&format!(" and {} more", names.len() - SHOWN));
        }
        Some(format!(
            "unsaved changes in {list} — :wa to write, :qa! to discard"
        ))
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
        if self.config.git && !self.ephemeral {
            self.tree_status = crate::git::statuses(tree.root());
        }
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

    /// The open buffer showing `path`, if there is one.
    #[must_use]
    pub fn index_of(&self, path: &Path) -> Option<BufferId> {
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

/// Load the configured theme, without its background if the config asks.
pub fn load_theme(config: &Config) -> (Theme, Option<String>) {
    let (mut theme, error) = Theme::load(&config.theme, &config::themes_dir());
    if config.transparent_background {
        theme.make_transparent();
    }
    (theme, error)
}

/// "12 minutes", "3 hours": how long ago something happened, roughly.
fn describe_age(seconds: u64) -> String {
    let (amount, unit) = match seconds {
        0..60 => (seconds, "second"),
        60..3600 => (seconds / 60, "minute"),
        3600..86_400 => (seconds / 3600, "hour"),
        _ => (seconds / 86_400, "day"),
    };
    format!("{amount} {unit}{}", if amount == 1 { "" } else { "s" })
}

fn command_picker() -> Picker {
    let commands = [
        "find",
        "buffers",
        "themes",
        "grep",
        "help",
        "write",
        "wall",
        "quit",
        "qall",
        "split",
        "vsplit",
        "close",
        "only",
        "terminal",
        "config",
        "config reload",
        "recover",
        "earlier",
        "later",
    ];
    let items = commands
        .into_iter()
        .map(|command| PickerItem::new(command, PickerTarget::Command(command.to_string())))
        .collect();
    Picker::new("commands", items)
}

fn theme_picker() -> Picker {
    let mut names = vec!["dark".to_string(), "light".to_string()];
    if let Ok(files) = std::fs::read_dir(config::themes_dir()) {
        names.extend(files.flatten().filter_map(|file| {
            let path = file.path();
            (path.extension()? == "toml")
                .then(|| path.file_stem()?.to_str().map(str::to_string))
                .flatten()
        }));
    }
    names.sort();
    names.dedup();
    let items = names
        .into_iter()
        .map(|name| PickerItem::new(name.clone(), PickerTarget::Theme(name)))
        .collect();
    Picker::new("themes", items)
}
