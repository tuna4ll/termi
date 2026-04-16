use crossterm::{
    cursor,
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind,
        KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
    },
    execute,
    style::{Attribute, Color, SetAttribute, SetForegroundColor},
    terminal,
};
use discord_rich_presence::{
    activity::{Activity, Timestamps},
    DiscordIpc, DiscordIpcClient,
};
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use std::{
    collections::{HashMap, HashSet},
    env, fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, TryRecvError},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use vt100::Parser as TerminalParser;

const STATUS_HEIGHT: u16 = 1;
const TREE_WIDTH: u16 = 32;
const LINE_NUM_WIDTH: u16 = 6;
const TERMINAL_SCROLL_STEP: usize = 3;
const TERMINAL_SCROLLBACK_LEN: usize = 5000;
const RECENT_ITEM_LIMIT: usize = 6;
const DASHBOARD_LOGO: &[&str] = &[
    " _______                  _ ",
    "|__   __|                (_)",
    "   | | ___ _ __ _ __ ___  _ ",
    "   | |/ _ \\ '__| '_ ` _ \\| |",
    "   | |  __/ |  | | | | | | |",
    "   |_|\\___|_|  |_| |_| |_|_|",
];

#[derive(Clone)]
struct FileNode {
    name: String,
    path: PathBuf,
    is_dir: bool,
    expanded: bool,
    depth: usize,
}

enum EditorMode {
    Dashboard,
    Normal,
    Search,
    CreateFile,
    CreateDir,
    DeleteConfirm,
    Rename,
    Terminal,
    GoToLine,
    Autocomplete,
}

#[derive(Clone, Copy, PartialEq)]
enum TokenType {
    Keyword,
    String,
    Comment,
    Number,
    Normal,
}

#[derive(Clone, PartialEq)]
enum Language {
    Rust,
    JavaScript,
    Python,
    C,
    Cpp,
    Java,
    None,
}

struct TerminalSession {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn Child + Send>,
}

#[derive(Clone)]
enum DashboardAction {
    OpenWorkspace(PathBuf),
    NewBuffer,
    RecentProject(PathBuf),
    RecentFile(PathBuf),
}

fn detect_language(path: &PathBuf) -> Language {
    if let Some(ext) = path.extension() {
        match ext.to_string_lossy().to_lowercase().as_str() {
            "rs" => Language::Rust,
            "js" | "jsx" | "mjs" => Language::JavaScript,
            "py" | "pyw" => Language::Python,
            "c" => Language::C,
            "cpp" | "cc" | "cxx" | "hpp" | "hxx" => Language::Cpp,
            "java" => Language::Java,
            _ => Language::None,
        }
    } else {
        Language::None
    }
}

fn get_keywords(lang: &Language) -> Vec<&'static str> {
    match lang {
        Language::Rust => vec![
            "fn", "let", "mut", "const", "struct", "enum", "impl", "trait", "use", "mod", "pub",
            "if", "else", "match", "for", "while", "loop", "return", "break", "continue", "true",
            "false", "self", "Self", "super", "as", "impl", "dyn", "unsafe",
        ],
        Language::JavaScript => vec![
            "function",
            "const",
            "let",
            "var",
            "if",
            "else",
            "for",
            "while",
            "return",
            "class",
            "extends",
            "import",
            "export",
            "default",
            "async",
            "await",
            "true",
            "false",
            "null",
            "undefined",
            "this",
            "new",
            "typeof",
            "instanceof",
        ],
        Language::Python => vec![
            "def", "class", "if", "else", "elif", "for", "while", "return", "import", "from", "as",
            "try", "except", "finally", "with", "lambda", "True", "False", "None", "and", "or",
            "not", "in", "is",
        ],
        Language::C | Language::Cpp => vec![
            "int", "char", "float", "double", "void", "struct", "enum", "if", "else", "for",
            "while", "return", "break", "continue", "switch", "case", "default", "typedef",
            "static", "const", "extern", "volatile", "goto",
        ],
        Language::Java => vec![
            "class",
            "interface",
            "public",
            "private",
            "protected",
            "static",
            "final",
            "void",
            "int",
            "String",
            "if",
            "else",
            "for",
            "while",
            "return",
            "new",
            "this",
            "super",
            "extends",
            "implements",
            "import",
            "package",
        ],
        Language::None => vec![],
    }
}

fn get_token_color(token_type: TokenType) -> Color {
    match token_type {
        TokenType::Keyword => Color::Cyan,
        TokenType::String => Color::Green,
        TokenType::Comment => Color::DarkGrey,
        TokenType::Number => Color::Yellow,
        TokenType::Normal => Color::White,
    }
}

fn tokenize_line(line: &str, lang: &Language, keywords: &[&str]) -> Vec<(usize, usize, TokenType)> {
    let mut tokens = Vec::new();
    let mut i = 0;
    let chars: Vec<char> = line.chars().collect();
    let len = chars.len();

    while i < len {
        if chars[i] == '"' || chars[i] == '\'' {
            let quote = chars[i];
            let start = i;
            i += 1;
            while i < len && chars[i] != quote {
                if chars[i] == '\\' && i + 1 < len {
                    i += 2;
                } else {
                    i += 1;
                }
            }
            if i < len {
                i += 1;
            }
            tokens.push((start, i, TokenType::String));
            continue;
        }

        if (lang == &Language::Rust
            || lang == &Language::C
            || lang == &Language::Cpp
            || lang == &Language::Java
            || lang == &Language::JavaScript)
            && i + 1 < len
            && chars[i] == '/'
            && chars[i + 1] == '/'
        {
            tokens.push((i, len, TokenType::Comment));
            break;
        }
        if lang == &Language::Python && i < len && chars[i] == '#' {
            tokens.push((i, len, TokenType::Comment));
            break;
        }

        if chars[i].is_ascii_digit() {
            let start = i;
            while i < len
                && (chars[i].is_ascii_digit()
                    || chars[i] == '.'
                    || chars[i] == 'e'
                    || chars[i] == 'E'
                    || chars[i] == '+'
                    || chars[i] == '-')
            {
                i += 1;
            }
            tokens.push((start, i, TokenType::Number));
            continue;
        }

        if chars[i].is_ascii_alphabetic() || chars[i] == '_' {
            let start = i;
            while i < len && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();
            let token_type = if keywords.iter().any(|&kw| kw == word) {
                TokenType::Keyword
            } else {
                TokenType::Normal
            };
            tokens.push((start, i, token_type));
            continue;
        }

        i += 1;
    }

    tokens
}

fn to_io_error<E: std::fmt::Display>(err: E) -> io::Error {
    io::Error::new(io::ErrorKind::Other, err.to_string())
}

fn terminal_shell_program() -> String {
    #[cfg(windows)]
    {
        env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".to_string())
    }

    #[cfg(not(windows))]
    {
        env::var("SHELL").unwrap_or_else(|_| "sh".to_string())
    }
}

fn terminal_pty_size(rows: u16, cols: u16) -> PtySize {
    PtySize {
        rows: rows.max(1),
        cols: cols.max(1),
        pixel_width: 0,
        pixel_height: 0,
    }
}

fn normalize_recent_path(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn dashboard_state_path() -> PathBuf {
    #[cfg(windows)]
    {
        if let Ok(appdata) = env::var("APPDATA") {
            return PathBuf::from(appdata)
                .join("Termi")
                .join("dashboard_state.txt");
        }
    }

    if let Ok(xdg_config) = env::var("XDG_CONFIG_HOME") {
        return PathBuf::from(xdg_config)
            .join("termi")
            .join("dashboard_state.txt");
    }

    if let Ok(home) = env::var("HOME") {
        return PathBuf::from(home)
            .join(".config")
            .join("termi")
            .join("dashboard_state.txt");
    }

    PathBuf::from("dashboard_state.txt")
}

fn push_recent_path(list: &mut Vec<PathBuf>, path: &Path) {
    let normalized = normalize_recent_path(path);
    list.retain(|entry| entry != &normalized);
    list.insert(0, normalized);
    list.truncate(RECENT_ITEM_LIMIT);
}

fn shorten_text_middle(text: &str, max_chars: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= max_chars {
        return text.to_string();
    }

    if max_chars <= 3 {
        return "...".chars().take(max_chars).collect();
    }

    let head_len = (max_chars - 3) / 2;
    let tail_len = max_chars - 3 - head_len;
    let head: String = chars[..head_len].iter().collect();
    let tail: String = chars[chars.len() - tail_len..].iter().collect();
    format!("{}...{}", head, tail)
}

struct Editor {
    buffer: Vec<Vec<char>>,
    cursor_x: usize,
    cursor_y: usize,

    scroll_y: usize,
    scroll_x: usize,

    file_name: Option<String>,
    file_path: Option<PathBuf>,

    status: String,
    dirty: bool,

    tree: Vec<FileNode>,
    tree_cursor: usize,
    tree_scroll: usize,
    show_tree: bool,

    show_line_numbers: bool,

    mode: EditorMode,
    dashboard_selection: usize,
    dashboard_workspace: PathBuf,
    recent_projects: Vec<PathBuf>,
    recent_files: Vec<PathBuf>,
    search_query: Vec<char>,
    search_results: Vec<(usize, usize)>,
    current_search_index: usize,

    create_name: Vec<char>,
    create_parent_path: Option<PathBuf>,

    history: Vec<Vec<Vec<char>>>,
    history_index: usize,
    history_limit: usize,

    language: Language,

    cursor_locked: bool,

    delete_target: Option<PathBuf>,

    rename_target: Option<PathBuf>,
    rename_name: Vec<char>,

    selection_start: Option<(usize, usize)>,
    selection_end: Option<(usize, usize)>,
    clipboard: Option<String>,
    is_selecting: bool,
    mouse_dragging: bool,
    mouse_drag_start_pos: Option<(usize, usize)>,
    last_mouse_click_time: Option<Instant>,
    last_mouse_click_pos: Option<(usize, usize)>,

    terminal_show: bool,
    terminal_scroll: usize,
    terminal_output_rx: Option<Receiver<String>>,
    terminal_session: Option<TerminalSession>,
    terminal_parser: TerminalParser,

    goto_line_input: Vec<char>,

    matched_bracket: Option<(usize, usize)>,

    last_scroll_y: usize,
    last_scroll_x: usize,
    last_tree_scroll: usize,
    needs_full_redraw: bool,

    quit_confirm: bool,

    dirty_files: HashSet<PathBuf>,

    file_buffers: HashMap<PathBuf, Vec<Vec<char>>>,

    autocomplete_suggestions: Vec<String>,
    autocomplete_index: usize,
    autocomplete_prefix: String,

    discord_client: Option<DiscordIpcClient>,
    discord_start_time: i64,
    discord_enabled: bool,
}

impl Editor {
    fn new() -> Self {
        Self::new_with_options(".", true)
    }

    fn new_with_path(initial_path: &str) -> Self {
        Self::new_with_options(initial_path, false)
    }

    fn new_with_options(initial_path: &str, show_dashboard: bool) -> Self {
        let (cols, rows) = terminal::size().unwrap_or((80, 24));
        let dashboard_workspace = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let (recent_projects, recent_files) = Self::load_dashboard_state();
        let mut e = Self {
            buffer: vec![vec![]],
            cursor_x: 0,
            cursor_y: 0,
            scroll_y: 0,
            scroll_x: 0,
            file_name: None,
            file_path: None,
            status: Self::default_status(),
            dirty: true,
            tree: vec![],
            tree_cursor: 0,
            tree_scroll: 0,
            show_tree: false,
            show_line_numbers: true,
            mode: if show_dashboard {
                EditorMode::Dashboard
            } else {
                EditorMode::Normal
            },
            dashboard_selection: 0,
            dashboard_workspace,
            recent_projects,
            recent_files,
            search_query: vec![],
            search_results: vec![],
            current_search_index: 0,
            create_name: vec![],
            create_parent_path: None,
            history: vec![vec![vec![]]],
            history_index: 0,
            history_limit: 100,
            language: Language::None,
            cursor_locked: false,
            delete_target: None,
            rename_target: None,
            rename_name: vec![],
            selection_start: None,
            selection_end: None,
            clipboard: None,
            is_selecting: false,
            mouse_dragging: false,
            mouse_drag_start_pos: None,
            last_mouse_click_time: None,
            last_mouse_click_pos: None,
            terminal_show: false,
            terminal_scroll: 0,
            terminal_output_rx: None,
            terminal_session: None,
            terminal_parser: TerminalParser::new(rows.max(1), cols.max(1), TERMINAL_SCROLLBACK_LEN),
            goto_line_input: vec![],
            matched_bracket: None,
            last_scroll_y: 0,
            last_scroll_x: 0,
            last_tree_scroll: 0,
            needs_full_redraw: true,
            quit_confirm: false,
            dirty_files: HashSet::new(),
            file_buffers: HashMap::new(),
            autocomplete_suggestions: vec![],
            autocomplete_index: 0,
            autocomplete_prefix: String::new(),
            discord_client: None,
            discord_start_time: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as i64,
            discord_enabled: true,
        };

        e.init_discord();

        let path = PathBuf::from(initial_path);
        if show_dashboard {
            e.load_root(".");
        } else if path.exists() && path.is_file() {
            let _ = e.open_file(&path);
            if let Some(parent) = path.parent() {
                e.load_root(parent.to_str().unwrap_or("."));
                e.show_tree = true;
                for (i, node) in e.tree.iter().enumerate() {
                    if node.path == path {
                        e.tree_cursor = i;
                        e.tree_scroll = 0;
                        break;
                    }
                }
            }
        } else if path.exists() && path.is_dir() {
            e.load_root(initial_path);
            e.show_tree = true;
            e.record_recent_project(&path);
        } else {
            e.load_root(".");
        }

        e.clamp_dashboard_selection();

        e
    }

    fn default_status() -> String {
        "Ctrl+O Tree | Ctrl+N File | Ctrl+M Folder | F2 Rename | Del Delete | Ctrl+S Save | Ctrl+F Find | Ctrl+G Go to Line | Shift+Arrow Select | Ctrl+C Copy | Ctrl+V Paste | Ctrl+Arrow Word | Ctrl+1 Terminal | Ctrl+Q Quit".into()
    }

    fn load_dashboard_state() -> (Vec<PathBuf>, Vec<PathBuf>) {
        let state_path = dashboard_state_path();
        let Ok(contents) = fs::read_to_string(state_path) else {
            return (Vec::new(), Vec::new());
        };

        let mut recent_projects = Vec::new();
        let mut recent_files = Vec::new();

        for line in contents.lines() {
            let Some((kind, raw_path)) = line.split_once('\t') else {
                continue;
            };
            let path = PathBuf::from(raw_path);
            if !path.exists() {
                continue;
            }

            match kind {
                "project" => push_recent_path(&mut recent_projects, &path),
                "file" => push_recent_path(&mut recent_files, &path),
                _ => {}
            }
        }

        (recent_projects, recent_files)
    }

    fn save_dashboard_state(&self) {
        let state_path = dashboard_state_path();
        if let Some(parent) = state_path.parent() {
            if fs::create_dir_all(parent).is_err() {
                return;
            }
        }

        let mut contents = String::new();
        for path in &self.recent_projects {
            contents.push_str("project\t");
            contents.push_str(&path.to_string_lossy());
            contents.push('\n');
        }
        for path in &self.recent_files {
            contents.push_str("file\t");
            contents.push_str(&path.to_string_lossy());
            contents.push('\n');
        }

        let _ = fs::write(state_path, contents);
    }

    fn record_recent_project(&mut self, path: &Path) {
        if !path.exists() {
            return;
        }
        push_recent_path(&mut self.recent_projects, path);
        self.save_dashboard_state();
        self.clamp_dashboard_selection();
    }

    fn record_recent_file(&mut self, path: &Path) {
        if !path.exists() {
            return;
        }
        push_recent_path(&mut self.recent_files, path);
        if let Some(parent) = path.parent() {
            self.record_recent_project(parent);
        } else {
            self.save_dashboard_state();
        }
        self.clamp_dashboard_selection();
    }

    fn refresh_dashboard_recents(&mut self) {
        self.recent_projects
            .retain(|path| path.exists() && path.is_dir());
        self.recent_files
            .retain(|path| path.exists() && path.is_file());
        self.save_dashboard_state();
        self.clamp_dashboard_selection();
        self.needs_full_redraw = true;
        self.dirty = true;
    }

    fn dashboard_actions(&self) -> Vec<DashboardAction> {
        let mut actions = vec![
            DashboardAction::OpenWorkspace(self.dashboard_workspace.clone()),
            DashboardAction::NewBuffer,
        ];
        actions.extend(
            self.recent_projects
                .iter()
                .cloned()
                .map(DashboardAction::RecentProject),
        );
        actions.extend(
            self.recent_files
                .iter()
                .cloned()
                .map(DashboardAction::RecentFile),
        );
        actions
    }

    fn clamp_dashboard_selection(&mut self) {
        let count = self.dashboard_actions().len();
        self.dashboard_selection = self.dashboard_selection.min(count.saturating_sub(1));
    }

    fn restore_default_status(&mut self) {
        self.status = Self::default_status();
    }

    fn enter_dashboard(&mut self) {
        self.mode = EditorMode::Dashboard;
        self.clamp_dashboard_selection();
        self.needs_full_redraw = true;
        self.dirty = true;
    }

    fn leave_dashboard(&mut self) {
        self.mode = EditorMode::Normal;
        self.restore_default_status();
        self.needs_full_redraw = true;
        self.dirty = true;
    }

    fn open_workspace(&mut self, path: &Path) {
        if !path.exists() || !path.is_dir() {
            self.status = format!("Workspace not found: {}", path.to_string_lossy());
            self.needs_full_redraw = true;
            self.dirty = true;
            return;
        }

        let normalized = normalize_recent_path(path);
        self.dashboard_workspace = normalized.clone();
        self.load_root(normalized.to_str().unwrap_or("."));
        self.show_tree = true;
        self.tree_cursor = 0;
        self.tree_scroll = 0;
        self.record_recent_project(&normalized);
        self.leave_dashboard();
    }

    fn new_empty_buffer(&mut self) {
        self.buffer = vec![vec![]];
        self.cursor_x = 0;
        self.cursor_y = 0;
        self.scroll_x = 0;
        self.scroll_y = 0;
        self.file_name = None;
        self.file_path = None;
        self.language = Language::None;
        self.history = vec![vec![vec![]]];
        self.history_index = 0;
        self.dirty = true;
        self.quit_confirm = false;
        self.update_discord_presence();
        self.leave_dashboard();
    }

    fn activate_dashboard_selection(&mut self) {
        let actions = self.dashboard_actions();
        let Some(action) = actions.get(self.dashboard_selection).cloned() else {
            return;
        };

        match action {
            DashboardAction::OpenWorkspace(path) | DashboardAction::RecentProject(path) => {
                self.open_workspace(&path);
            }
            DashboardAction::NewBuffer => {
                self.new_empty_buffer();
            }
            DashboardAction::RecentFile(path) => {
                if !path.exists() || !path.is_file() {
                    self.refresh_dashboard_recents();
                    return;
                }
                if let Some(parent) = path.parent() {
                    self.open_workspace(parent);
                }
                let _ = self.open_file(&path);
                self.leave_dashboard();
            }
        }
    }

    fn init_discord(&mut self) {
        const DISCORD_APP_ID: &str = "1457025246568906804";

        match DiscordIpcClient::new(DISCORD_APP_ID) {
            Ok(mut client) => match client.connect() {
                Ok(_) => {
                    self.discord_client = Some(client);
                    self.update_discord_presence();
                }
                Err(_) => {
                    self.discord_enabled = false;
                }
            },
            Err(_) => {
                self.discord_enabled = false;
            }
        }
    }

    fn update_discord_presence(&mut self) {
        if !self.discord_enabled {
            return;
        }

        let client = match &mut self.discord_client {
            Some(c) => c,
            None => return,
        };

        let (details, state) = if let Some(ref file_name) = self.file_name {
            let lang_name = match self.language {
                Language::Rust => "Rust",
                Language::JavaScript => "JavaScript",
                Language::Python => "Python",
                Language::C => "C",
                Language::Cpp => "C++",
                Language::Java => "Java",
                Language::None => "Text",
            };

            let line_count = self.buffer.len();
            (
                format!("Editing {}", file_name),
                format!("{} | {} lines", lang_name, line_count),
            )
        } else {
            ("Idle".to_string(), "No file open".to_string())
        };

        let activity = Activity::new()
            .details(&details)
            .state(&state)
            .timestamps(Timestamps::new().start(self.discord_start_time));

        let _ = client.set_activity(activity);
    }

    fn close_discord(&mut self) {
        if let Some(ref mut client) = self.discord_client {
            let _ = client.close();
        }
        self.discord_client = None;
    }

    fn load_root(&mut self, dir: &str) {
        self.tree.clear();
        self.load_dir(PathBuf::from(dir), 0);
        self.tree_scroll = 0;
        self.tree_cursor = 0;
        self.needs_full_redraw = true;
    }

    fn load_dir(&mut self, path: PathBuf, depth: usize) {
        if let Ok(entries) = fs::read_dir(path) {
            for e in entries.flatten() {
                let meta = e.metadata().unwrap();
                self.tree.push(FileNode {
                    name: e.file_name().to_string_lossy().into(),
                    path: e.path(),
                    is_dir: meta.is_dir(),
                    expanded: false,
                    depth,
                });
            }
        }
    }

    fn toggle_dir(&mut self, idx: usize) {
        if !self.tree[idx].is_dir {
            return;
        }

        if self.tree[idx].expanded {
            let d = self.tree[idx].depth;
            self.tree[idx].expanded = false;
            let start_idx = idx + 1;
            while start_idx < self.tree.len() && self.tree[start_idx].depth > d {
                self.tree.remove(start_idx);
            }
            self.needs_full_redraw = true;
        } else {
            self.tree[idx].expanded = true;
            let path = self.tree[idx].path.clone();
            let depth = self.tree[idx].depth + 1;
            let mut insert = idx + 1;

            if let Ok(entries) = fs::read_dir(path) {
                for e in entries.flatten() {
                    let meta = e.metadata().unwrap();
                    self.tree.insert(
                        insert,
                        FileNode {
                            name: e.file_name().to_string_lossy().into(),
                            path: e.path(),
                            is_dir: meta.is_dir(),
                            expanded: false,
                            depth,
                        },
                    );
                    insert += 1;
                }
            }
            self.needs_full_redraw = true;
        }
    }

    fn open_file(&mut self, path: &PathBuf) -> io::Result<()> {
        if let Some(old_path) = &self.file_path {
            self.file_buffers
                .insert(old_path.clone(), self.buffer.clone());
        }

        if let Some(cached_buffer) = self.file_buffers.get(path) {
            self.buffer = cached_buffer.clone();
        } else {
            let mut s = String::new();
            fs::File::open(path)?.read_to_string(&mut s)?;
            self.buffer = s.lines().map(|l| l.chars().collect()).collect();
            if self.buffer.is_empty() {
                self.buffer.push(vec![]);
            }
            self.file_buffers.insert(path.clone(), self.buffer.clone());
        }

        self.file_path = Some(path.clone());
        self.file_name = Some(path.file_name().unwrap().to_string_lossy().into());
        self.language = detect_language(path);
        self.cursor_x = 0;
        self.cursor_y = 0;
        self.scroll_y = 0;
        self.scroll_x = 0;
        self.needs_full_redraw = true;
        self.dirty = false;
        self.dirty_files.remove(path);
        self.update_bracket_matching();
        self.save_history_state();
        self.record_recent_file(path);
        self.update_discord_presence();
        Ok(())
    }

    fn mark_file_dirty(&mut self) {
        self.dirty = true;
        self.needs_full_redraw = true;
        if let Some(path) = &self.file_path {
            self.dirty_files.insert(path.clone());
        }
    }

    fn get_word_boundaries(&self, y: usize, x: usize) -> Option<(usize, usize)> {
        if y >= self.buffer.len() {
            return None;
        }
        let line = &self.buffer[y];
        if x > line.len() {
            return None;
        }

        let is_special_char = |c: char| -> bool {
            matches!(
                c,
                '.' | ','
                    | '['
                    | ']'
                    | '{'
                    | '}'
                    | '$'
                    | '('
                    | ')'
                    | ';'
                    | ':'
                    | '!'
                    | '?'
                    | '@'
                    | '#'
                    | '%'
                    | '^'
                    | '&'
                    | '*'
                    | '+'
                    | '-'
                    | '='
                    | '/'
                    | '\\'
                    | '|'
                    | '<'
                    | '>'
                    | '`'
                    | '\''
                    | '"'
            )
        };

        if line.is_empty() {
            return Some((0, 0));
        }

        let mut start = x.min(line.len());
        let mut end = x.min(line.len());

        if start >= line.len() || line[start] == ' ' || line[start] == '\t' {
            while start > 0 && (line[start - 1] == ' ' || line[start - 1] == '\t') {
                start -= 1;
            }
            if start > 0 {
                start -= 1;
                while start > 0 && !(line[start - 1] == ' ' || line[start - 1] == '\t') {
                    if is_special_char(line[start - 1]) {
                        break;
                    }
                    start -= 1;
                }
            }
            end = start;
            while end < line.len() && !(line[end] == ' ' || line[end] == '\t') {
                if is_special_char(line[end]) {
                    end += 1;
                    break;
                }
                end += 1;
            }
        } else {
            let c = line[start];
            if is_special_char(c) {
                end = start + 1;
            } else {
                while start > 0 && !(line[start - 1] == ' ' || line[start - 1] == '\t') {
                    let prev = line[start - 1];
                    if is_special_char(prev) {
                        break;
                    }
                    start -= 1;
                }
                while end < line.len() && !(line[end] == ' ' || line[end] == '\t') {
                    if is_special_char(line[end]) {
                        end += 1;
                        break;
                    }
                    end += 1;
                }
            }
        }

        Some((start, end))
    }

    fn select_word_at(&mut self, y: usize, x: usize) {
        if let Some((start, end)) = self.get_word_boundaries(y, x) {
            self.cursor_y = y;
            self.cursor_x = start;
            self.start_selection();
            self.cursor_x = end;
            self.update_selection_end();
            self.needs_full_redraw = true;
        }
    }

    fn select_line_at(&mut self, y: usize) {
        if y < self.buffer.len() {
            self.cursor_y = y;
            self.cursor_x = 0;
            self.start_selection();
            self.cursor_x = self.buffer[y].len();
            self.update_selection_end();
            self.needs_full_redraw = true;
        }
    }

    fn handle_mouse_click(&mut self, col: u16, row: u16, rows: u16, _cols: u16, shift: bool) {
        let tree_offset = if self.show_tree { TREE_WIDTH } else { 0 };
        let line_num_offset = if self.show_line_numbers {
            LINE_NUM_WIDTH
        } else {
            0
        };
        let text_offset = tree_offset + line_num_offset;

        if col < text_offset {
            return;
        }

        let max_lines = rows - STATUS_HEIGHT;
        if row >= max_lines {
            return;
        }

        let clicked_y = self.scroll_y + row as usize;
        if clicked_y < self.buffer.len() {
            let clicked_x_screen = (col - text_offset) as usize;
            let clicked_x = self.scroll_x + clicked_x_screen;
            let clicked_pos = (clicked_y, clicked_x.min(self.buffer[clicked_y].len()));

            let now = Instant::now();
            let is_double_click = if let (Some(last_time), Some(last_pos)) =
                (self.last_mouse_click_time, self.last_mouse_click_pos)
            {
                last_pos == clicked_pos
                    && now.duration_since(last_time) < Duration::from_millis(500)
            } else {
                false
            };

            self.cursor_y = clicked_y;
            if let Some(line) = self.buffer.get(clicked_y) {
                self.cursor_x = clicked_x.min(line.len());
            } else {
                self.cursor_x = 0;
            }

            if is_double_click {
                self.select_word_at(clicked_y, self.cursor_x);
            } else if shift {
                if !self.is_selecting {
                    self.start_selection();
                }
                self.update_selection_end();
            } else {
                self.is_selecting = false;
                self.selection_start = None;
                self.selection_end = None;
                self.mouse_dragging = true;
                self.mouse_drag_start_pos = Some((self.cursor_y, self.cursor_x));
            }

            self.last_mouse_click_time = Some(now);
            self.last_mouse_click_pos = Some(clicked_pos);
            self.needs_full_redraw = true;
        }
    }

    fn handle_mouse_drag(&mut self, col: u16, row: u16, rows: u16, _cols: u16) {
        let tree_offset = if self.show_tree { TREE_WIDTH } else { 0 };
        let line_num_offset = if self.show_line_numbers {
            LINE_NUM_WIDTH
        } else {
            0
        };
        let text_offset = tree_offset + line_num_offset;

        if col < text_offset {
            return;
        }

        let max_lines = rows - STATUS_HEIGHT;
        if row >= max_lines {
            return;
        }

        if self.mouse_dragging {
            let clicked_y = self.scroll_y + row as usize;
            if clicked_y < self.buffer.len() {
                let clicked_x_screen = (col - text_offset) as usize;
                let clicked_x = self.scroll_x + clicked_x_screen;

                if !self.is_selecting {
                    if let Some(start_pos) = self.mouse_drag_start_pos {
                        self.selection_start = Some(start_pos);
                        self.selection_end = Some(start_pos);
                        self.is_selecting = true;
                    } else {
                        self.cursor_y = clicked_y;
                        if let Some(line) = self.buffer.get(clicked_y) {
                            self.cursor_x = clicked_x.min(line.len());
                        } else {
                            self.cursor_x = 0;
                        }
                        self.start_selection();
                    }
                }

                self.cursor_y = clicked_y;
                if let Some(line) = self.buffer.get(clicked_y) {
                    self.cursor_x = clicked_x.min(line.len());
                } else {
                    self.cursor_x = 0;
                }
                self.update_selection_end();
                self.needs_full_redraw = true;
            }
        }
    }

    fn handle_mouse_release(&mut self) {
        self.mouse_dragging = false;
    }

    fn handle_mouse_scroll(&mut self, rows: u16, up: bool) {
        let max_lines = rows as usize - STATUS_HEIGHT as usize;
        let max_scroll_y = self.buffer.len().saturating_sub(max_lines);

        self.cursor_locked = true;

        const SCROLL_STEP: usize = 3;

        if up {
            if self.scroll_y > 0 {
                self.scroll_y = self.scroll_y.saturating_sub(SCROLL_STEP);
                self.dirty = true;
            }
        } else {
            if self.scroll_y < max_scroll_y {
                self.scroll_y = (self.scroll_y + SCROLL_STEP).min(max_scroll_y);
                self.dirty = true;
            }
        }
    }

    fn save(&mut self) -> io::Result<()> {
        if let Some(path) = &self.file_path {
            let txt = self
                .buffer
                .iter()
                .map(|l| l.iter().collect::<String>())
                .collect::<Vec<_>>()
                .join("\n");
            fs::write(path, txt)?;
            self.status = "Saved".into();
            self.needs_full_redraw = true;
            self.dirty = false;
            self.dirty_files.remove(path);
            self.file_buffers.insert(path.clone(), self.buffer.clone());
        }
        Ok(())
    }

    fn ensure_cursor_visible(&mut self, rows: u16, cols: u16) {
        let max_lines = rows as usize - STATUS_HEIGHT as usize;
        let tree_offset = if self.show_tree { TREE_WIDTH } else { 0 };
        let line_num_offset = if self.show_line_numbers {
            LINE_NUM_WIDTH
        } else {
            0
        };
        let text_offset = tree_offset + line_num_offset;
        let available_width = (cols - text_offset) as usize;

        if self.cursor_y < self.scroll_y {
            self.scroll_y = self.cursor_y;
        } else if max_lines > 0 && self.cursor_y >= self.scroll_y + max_lines {
            self.scroll_y = self.cursor_y - max_lines + 1;
        }

        if available_width > 0 {
            if self.cursor_x < self.scroll_x {
                self.scroll_x = self.cursor_x;
            } else if self.cursor_x >= self.scroll_x + available_width {
                self.scroll_x = self.cursor_x - available_width + 1;
            }
        }
    }

    fn left(&mut self) {
        if self.cursor_x > 0 {
            if self.is_selecting {
                self.update_selection_end();
            } else {
                self.clear_selection();
            }
            self.cursor_x -= 1;
            self.cursor_locked = false;
            self.update_bracket_matching();
            self.dirty = true;
        }
    }
    fn right(&mut self) {
        if self.cursor_x < self.buffer[self.cursor_y].len() {
            if !self.is_selecting {
                self.clear_selection();
            }
            self.cursor_x += 1;
            if self.is_selecting {
                self.update_selection_end();
            }
            self.cursor_locked = false;
            self.update_bracket_matching();
            self.dirty = true;
        }
    }
    fn up(&mut self) {
        if self.cursor_y > 0 {
            if self.is_selecting {
                self.update_selection_end();
            } else {
                self.clear_selection();
            }
            self.cursor_y -= 1;
            self.cursor_x = self.cursor_x.min(self.buffer[self.cursor_y].len());
            self.cursor_locked = false;
            self.update_bracket_matching();
            self.dirty = true;
        }
    }
    fn down(&mut self) {
        if self.cursor_y + 1 < self.buffer.len() {
            if self.is_selecting {
                self.update_selection_end();
            } else {
                self.clear_selection();
            }
            self.cursor_y += 1;
            self.cursor_x = self.cursor_x.min(self.buffer[self.cursor_y].len());
            self.cursor_locked = false;
            self.update_bracket_matching();
            self.dirty = true;
        }
    }

    fn start_selection(&mut self) {
        self.is_selecting = true;
        self.selection_start = Some((self.cursor_y, self.cursor_x));
        self.selection_end = Some((self.cursor_y, self.cursor_x));
        self.dirty = true;
    }

    fn update_selection_end(&mut self) {
        if self.is_selecting {
            self.selection_end = Some((self.cursor_y, self.cursor_x));
            self.dirty = true;
        }
    }

    fn clear_selection(&mut self) {
        if !self.is_selecting {
            self.selection_start = None;
            self.selection_end = None;
        }
    }

    fn get_selected_text(&self) -> Option<String> {
        if let (Some(start), Some(end)) = (self.selection_start, self.selection_end) {
            let (start_y, start_x) = start;
            let (end_y, end_x) = end;

            if start_y == end_y && start_x == end_x {
                return None;
            }

            let (actual_start_y, actual_start_x, actual_end_y, actual_end_x) =
                if (start_y, start_x) < (end_y, end_x) {
                    (start_y, start_x, end_y, end_x)
                } else {
                    (end_y, end_x, start_y, start_x)
                };

            let mut result = String::new();

            if actual_start_y == actual_end_y {
                if let Some(line) = self.buffer.get(actual_start_y) {
                    let selected: String = line
                        .iter()
                        .skip(actual_start_x)
                        .take(actual_end_x - actual_start_x)
                        .collect();
                    result.push_str(&selected);
                }
            } else {
                for y in actual_start_y..=actual_end_y {
                    if let Some(line) = self.buffer.get(y) {
                        if y == actual_start_y {
                            let selected: String = line.iter().skip(actual_start_x).collect();
                            result.push_str(&selected);
                        } else if y == actual_end_y {
                            let selected: String = line.iter().take(actual_end_x).collect();
                            result.push_str(&selected);
                        } else {
                            let selected: String = line.iter().collect();
                            result.push_str(&selected);
                        }
                        if y < actual_end_y {
                            result.push('\n');
                        }
                    }
                }
            }

            if result.is_empty() {
                None
            } else {
                Some(result)
            }
        } else {
            None
        }
    }

    fn select_all(&mut self) {
        if self.buffer.is_empty() {
            return;
        }

        self.selection_start = Some((0, 0));

        let last_line = self.buffer.len() - 1;
        let last_col = self.buffer[last_line].len();
        self.selection_end = Some((last_line, last_col));

        self.cursor_y = last_line;
        self.cursor_x = last_col;
        self.is_selecting = true;
        self.needs_full_redraw = true;
        self.dirty = true;
    }

    fn copy_selection(&mut self) {
        if let Some(text) = self.get_selected_text() {
            self.clipboard = Some(text.clone());
            if let Ok(mut clipboard) = arboard::Clipboard::new() {
                let _ = clipboard.set_text(&text);
            }
            self.status = "Copied".into();
            self.dirty = true;
        }
    }

    fn paste(&mut self) {
        let clipboard_text = if let Some(ref internal_text) = self.clipboard {
            Some(internal_text.clone())
        } else {
            if let Ok(mut clipboard) = arboard::Clipboard::new() {
                clipboard.get_text().ok()
            } else {
                None
            }
        };

        if let Some(clipboard_text) = clipboard_text {
            self.save_history_state();
            self.clear_selection();

            let normalized_text = clipboard_text.replace("\r\n", "\n").replace('\r', "\n");

            let lines: Vec<&str> = normalized_text.split('\n').collect();

            if lines.len() == 1 {
                let chars: Vec<char> = lines[0].chars().collect();
                for &c in &chars {
                    self.buffer[self.cursor_y].insert(self.cursor_x, c);
                    self.cursor_x += 1;
                }
            } else {
                let rest = self.buffer[self.cursor_y].split_off(self.cursor_x);

                let first_chars: Vec<char> = lines[0].chars().collect();
                for &c in &first_chars {
                    self.buffer[self.cursor_y].push(c);
                    self.cursor_x += 1;
                }

                for line in lines.iter().skip(1).take(lines.len() - 1) {
                    let line_chars: Vec<char> = line.chars().collect();
                    self.buffer.insert(self.cursor_y + 1, line_chars);
                    self.cursor_y += 1;
                    self.cursor_x = self.buffer[self.cursor_y].len();
                }

                if let Some(last_line) = lines.last() {
                    let mut new_last_line: Vec<char> = last_line.chars().collect();
                    new_last_line.extend(rest);
                    self.buffer.insert(self.cursor_y + 1, new_last_line);
                    self.cursor_y += 1;
                    self.cursor_x = lines.last().unwrap().chars().count();
                }
            }

            self.mark_file_dirty();
        }
    }

    fn save_history_state(&mut self) {
        self.history.truncate(self.history_index + 1);

        let snapshot = self.buffer.iter().map(|line| line.clone()).collect();
        self.history.push(snapshot);
        self.history_index += 1;

        if self.history.len() > self.history_limit {
            self.history.remove(0);
            self.history_index -= 1;
        }
    }

    fn undo(&mut self) {
        if self.history_index > 0 {
            self.history_index -= 1;
            if let Some(old_state) = self.history.get(self.history_index) {
                self.buffer = old_state.iter().map(|line| line.clone()).collect();
                if self.cursor_y >= self.buffer.len() {
                    self.cursor_y = self.buffer.len().saturating_sub(1);
                }
                if let Some(line) = self.buffer.get(self.cursor_y) {
                    self.cursor_x = self.cursor_x.min(line.len());
                }
                self.needs_full_redraw = true;
                self.dirty = true;
            }
        }
    }

    fn redo(&mut self) {
        if self.history_index + 1 < self.history.len() {
            self.history_index += 1;
            if let Some(new_state) = self.history.get(self.history_index) {
                self.buffer = new_state.iter().map(|line| line.clone()).collect();
                if self.cursor_y >= self.buffer.len() {
                    self.cursor_y = self.buffer.len().saturating_sub(1);
                }
                if let Some(line) = self.buffer.get(self.cursor_y) {
                    self.cursor_x = self.cursor_x.min(line.len());
                }
                self.needs_full_redraw = true;
                self.dirty = true;
            }
        }
    }

    fn start_search(&mut self) {
        self.mode = EditorMode::Search;
        self.search_query.clear();
        self.search_results.clear();
        self.current_search_index = 0;
        self.status = "Search: ".into();
        self.needs_full_redraw = true;
        self.dirty = true;
    }

    fn cancel_search(&mut self) {
        self.mode = EditorMode::Normal;
        self.search_query.clear();
        self.search_results.clear();
        self.status =
            "Ctrl+O Tree | Ctrl+S Save | Ctrl+F Find | Ctrl+Z Undo | Ctrl+Y Redo | Ctrl+Q Quit"
                .into();
        self.needs_full_redraw = true;
        self.dirty = true;
    }

    fn start_create_file(&mut self) {
        if !self.show_tree || self.tree.is_empty() {
            return;
        }

        let selected_node = &self.tree[self.tree_cursor];
        let parent_path = if selected_node.is_dir {
            selected_node.path.clone()
        } else {
            selected_node
                .path
                .parent()
                .unwrap_or(&PathBuf::from("."))
                .to_path_buf()
        };

        self.mode = EditorMode::CreateFile;
        self.create_name.clear();
        self.create_parent_path = Some(parent_path);
        self.status = "New file name: ".into();
        self.needs_full_redraw = true;
        self.dirty = true;
    }

    fn start_create_dir(&mut self) {
        if !self.show_tree || self.tree.is_empty() {
            return;
        }

        let selected_node = &self.tree[self.tree_cursor];
        let parent_path = if selected_node.is_dir {
            selected_node.path.clone()
        } else {
            selected_node
                .path
                .parent()
                .unwrap_or(&PathBuf::from("."))
                .to_path_buf()
        };

        self.mode = EditorMode::CreateDir;
        self.create_name.clear();
        self.create_parent_path = Some(parent_path);
        self.status = "New folder name: ".into();
        self.needs_full_redraw = true;
        self.dirty = true;
    }

    fn cancel_create(&mut self) {
        self.mode = EditorMode::Normal;
        self.create_name.clear();
        self.create_parent_path = None;
        self.status =
            "Ctrl+O Tree | Ctrl+S Save | Ctrl+F Find | Ctrl+Z Undo | Ctrl+Y Redo | Ctrl+Q Quit"
                .into();
        self.needs_full_redraw = true;
        self.dirty = true;
    }

    fn create_file_or_dir(&mut self) -> io::Result<()> {
        if self.create_name.is_empty() {
            return Ok(());
        }

        let name: String = self.create_name.iter().collect();
        let parent_path = self.create_parent_path.clone();
        if let Some(parent) = parent_path {
            let new_path = parent.join(&name);

            match self.mode {
                EditorMode::CreateFile => {
                    fs::File::create(&new_path)?;
                    let _ = self.open_file(&new_path);
                }
                EditorMode::CreateDir => {
                    fs::create_dir(&new_path)?;
                }
                _ => {}
            }

            if parent.to_string_lossy() == "." {
                self.load_root(".");
            } else {
                self.reload_tree_at_parent(&parent);
            }
            self.needs_full_redraw = true;
        }

        self.cancel_create();
        Ok(())
    }

    fn reload_tree_at_parent(&mut self, parent: &std::path::Path) {
        for (i, node) in self.tree.iter().enumerate() {
            if node.path == *parent && node.is_dir {
                if node.expanded {
                    let depth = node.depth;
                    let remove_start = i + 1;
                    while remove_start < self.tree.len() && self.tree[remove_start].depth > depth {
                        self.tree.remove(remove_start);
                    }

                    if let Ok(entries) = fs::read_dir(parent) {
                        let mut insert_pos = i + 1;
                        for e in entries.flatten() {
                            let meta = e.metadata().unwrap();
                            self.tree.insert(
                                insert_pos,
                                FileNode {
                                    name: e.file_name().to_string_lossy().into(),
                                    path: e.path(),
                                    is_dir: meta.is_dir(),
                                    expanded: false,
                                    depth: depth + 1,
                                },
                            );
                            insert_pos += 1;
                        }
                    }
                } else {
                    self.toggle_dir(i);
                }
                break;
            }
        }
        self.dirty = true;
    }

    fn start_delete(&mut self) {
        if !self.show_tree || self.tree.is_empty() {
            return;
        }

        let selected_node = &self.tree[self.tree_cursor];
        self.delete_target = Some(selected_node.path.clone());
        self.mode = EditorMode::DeleteConfirm;
        let item_type = if selected_node.is_dir {
            "folder"
        } else {
            "file"
        };
        self.status = format!("Delete {}? (Y/N)", item_type);
        self.needs_full_redraw = true;
        self.dirty = true;
    }

    fn confirm_delete(&mut self) -> io::Result<()> {
        let target = if let Some(t) = &self.delete_target {
            Some(t.clone())
        } else {
            None
        };

        if let Some(target) = target {
            let is_dir = target.is_dir();
            let parent = target
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| PathBuf::from("."));

            if is_dir {
                fs::remove_dir_all(&target)?;
            } else {
                fs::remove_file(&target)?;
            }

            if let Some(current_path) = &self.file_path {
                if current_path == &target {
                    self.buffer = vec![vec![]];
                    self.file_path = None;
                    self.file_name = None;
                    self.language = Language::None;
                }
            }

            if parent.to_string_lossy() == "." {
                self.load_root(".");
            } else {
                self.reload_tree_at_parent(&parent);
                for (i, node) in self.tree.iter().enumerate() {
                    if node.path == parent {
                        self.tree_cursor = i;
                        break;
                    }
                }
            }
        }

        self.cancel_delete();
        Ok(())
    }

    fn cancel_delete(&mut self) {
        self.mode = EditorMode::Normal;
        self.delete_target = None;
        self.status =
            "Ctrl+O Tree | Ctrl+S Save | Ctrl+F Find | Ctrl+Z Undo | Ctrl+Y Redo | Ctrl+Q Quit"
                .into();
        self.needs_full_redraw = true;
        self.dirty = true;
    }

    fn start_rename(&mut self) {
        if !self.show_tree || self.tree.is_empty() {
            return;
        }

        let selected_node = &self.tree[self.tree_cursor];
        self.rename_target = Some(selected_node.path.clone());
        self.rename_name = selected_node.name.chars().collect();
        self.mode = EditorMode::Rename;
        self.status = "Rename: ".into();
        self.needs_full_redraw = true;
        self.dirty = true;
    }

    fn cancel_rename(&mut self) {
        self.mode = EditorMode::Normal;
        self.rename_target = None;
        self.rename_name.clear();
        self.status =
            "Ctrl+O Tree | Ctrl+S Save | Ctrl+F Find | Ctrl+Z Undo | Ctrl+Y Redo | Ctrl+Q Quit"
                .into();
        self.needs_full_redraw = true;
        self.dirty = true;
    }

    fn confirm_rename(&mut self) -> io::Result<()> {
        if self.rename_name.is_empty() {
            return Ok(());
        }

        let target = if let Some(t) = &self.rename_target {
            Some(t.clone())
        } else {
            None
        };

        let new_name: String = self.rename_name.iter().collect();
        if let Some(target) = target {
            let parent = target
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| PathBuf::from("."));
            let new_path = parent.join(&new_name);

            fs::rename(&target, &new_path)?;

            if let Some(current_path) = &self.file_path {
                if current_path == &target {
                    self.file_path = Some(new_path.clone());
                    self.file_name = Some(new_name.clone());
                }
            }

            if parent.to_string_lossy() == "." {
                self.load_root(".");
                for (i, node) in self.tree.iter().enumerate() {
                    if node.path == new_path {
                        self.tree_cursor = i;
                        break;
                    }
                }
            } else {
                self.reload_tree_at_parent(&parent);
                for (i, node) in self.tree.iter().enumerate() {
                    if node.path == new_path {
                        self.tree_cursor = i;
                        break;
                    }
                }
            }
            self.needs_full_redraw = true;
        }

        self.cancel_rename();
        Ok(())
    }

    fn update_search(&mut self) {
        if self.search_query.is_empty() {
            self.search_results.clear();
            return;
        }

        let query: String = self.search_query.iter().collect();
        self.search_results.clear();

        for (y, line) in self.buffer.iter().enumerate() {
            let line_str: String = line.iter().collect();
            let mut start = 0;
            while let Some(pos) = line_str[start..].find(&query) {
                let absolute_pos = start + pos;
                self.search_results.push((y, absolute_pos));
                start = absolute_pos + 1;
            }
        }

        if !self.search_results.is_empty() {
            self.current_search_index = 0;
            self.jump_to_search_result(0);
        }
    }

    fn jump_to_search_result(&mut self, index: usize) {
        if let Some(&(y, x)) = self.search_results.get(index) {
            self.cursor_y = y;
            self.cursor_x = x;
            self.current_search_index = index;
            self.dirty = true;
        }
    }

    fn next_search_result(&mut self) {
        if !self.search_results.is_empty() {
            self.current_search_index = (self.current_search_index + 1) % self.search_results.len();
            self.jump_to_search_result(self.current_search_index);
        }
    }

    fn insert(&mut self, c: char) {
        self.save_history_state();

        let closing = match c {
            '(' => Some(')'),
            '[' => Some(']'),
            '{' => Some('}'),
            '"' => Some('"'),
            '\'' => Some('\''),
            _ => None,
        };

        self.buffer[self.cursor_y].insert(self.cursor_x, c);
        self.cursor_x += 1;

        if let Some(close) = closing {
            self.buffer[self.cursor_y].insert(self.cursor_x, close);
        }

        self.cursor_locked = false;
        self.mark_file_dirty();
    }

    fn backspace(&mut self) {
        if self.cursor_x > 0 {
            self.save_history_state();
            self.cursor_x -= 1;
            self.buffer[self.cursor_y].remove(self.cursor_x);
            self.cursor_locked = false;
            self.mark_file_dirty();
        } else if self.cursor_y > 0 {
            self.save_history_state();
            let current_line = self.buffer.remove(self.cursor_y);
            self.cursor_y -= 1;
            self.cursor_x = self.buffer[self.cursor_y].len();
            self.buffer[self.cursor_y].extend(current_line);
            self.cursor_locked = false;
            self.update_bracket_matching();
            self.mark_file_dirty();
        }
    }

    fn delete(&mut self) {
        if self.cursor_x < self.buffer[self.cursor_y].len() {
            self.save_history_state();
            self.buffer[self.cursor_y].remove(self.cursor_x);
            self.cursor_locked = false;
            self.mark_file_dirty();
        } else if self.cursor_x == self.buffer[self.cursor_y].len()
            && self.cursor_y + 1 < self.buffer.len()
        {
            self.save_history_state();
            let next_line = self.buffer.remove(self.cursor_y + 1);
            self.buffer[self.cursor_y].extend(next_line);
            self.cursor_locked = false;
            self.update_bracket_matching();
            self.mark_file_dirty();
        }
    }

    fn newline(&mut self) {
        self.save_history_state();
        let rest = self.buffer[self.cursor_y].split_off(self.cursor_x);

        let indent_level = self.calculate_indent_level();

        self.buffer.insert(self.cursor_y + 1, rest);
        self.cursor_y += 1;
        self.cursor_x = 0;

        if indent_level > 0 {
            let indent = self.get_indent_string(indent_level);
            for c in indent.chars() {
                self.buffer[self.cursor_y].insert(0, c);
                self.cursor_x += 1;
            }
        }

        self.cursor_locked = false;
        self.update_bracket_matching();
        self.mark_file_dirty();
    }

    fn calculate_indent_level(&self) -> usize {
        if self.cursor_y == 0 {
            return 0;
        }

        let prev_line = &self.buffer[self.cursor_y - 1];

        let mut prev_indent = 0;
        for c in prev_line.iter() {
            if *c == ' ' {
                prev_indent += 1;
            } else if *c == '\t' {
                prev_indent += 4;
            } else {
                break;
            }
        }

        let prev_line_str: String = prev_line.iter().collect();
        let trimmed_prev: String = prev_line_str.trim_start().to_string();

        let increase_indent = match self.language {
            Language::Python => {
                trimmed_prev.ends_with(':')
                    || trimmed_prev.starts_with("if ")
                    || trimmed_prev.starts_with("elif ")
                    || trimmed_prev.starts_with("else ")
                    || trimmed_prev.starts_with("for ")
                    || trimmed_prev.starts_with("while ")
                    || trimmed_prev.starts_with("def ")
                    || trimmed_prev.starts_with("class ")
                    || trimmed_prev.starts_with("try:")
                    || trimmed_prev.starts_with("except ")
                    || trimmed_prev.starts_with("finally:")
                    || trimmed_prev.starts_with("with ")
                    || trimmed_prev.starts_with("async ")
                    || trimmed_prev == "else:"
                    || trimmed_prev == "elif:"
                    || trimmed_prev == "except:"
                    || trimmed_prev == "finally:"
                    || trimmed_prev == "try:"
            }
            Language::Rust
            | Language::JavaScript
            | Language::C
            | Language::Cpp
            | Language::Java => {
                trimmed_prev.ends_with(" {")
                    || trimmed_prev.ends_with("{")
                    || (trimmed_prev.starts_with("if ") && !trimmed_prev.contains(";"))
                    || (trimmed_prev.starts_with("for ") && !trimmed_prev.contains(";"))
                    || (trimmed_prev.starts_with("while ") && !trimmed_prev.contains(";"))
                    || trimmed_prev.starts_with("fn ")
                    || trimmed_prev.starts_with("impl ")
                    || trimmed_prev.starts_with("trait ")
                    || trimmed_prev.starts_with("struct ")
                    || trimmed_prev.starts_with("enum ")
                    || trimmed_prev.starts_with("match ")
                    || trimmed_prev.starts_with("unsafe ")
                    || trimmed_prev.starts_with("loop ")
                    || trimmed_prev.starts_with("function ")
                    || trimmed_prev.starts_with("class ")
                    || trimmed_prev.ends_with("=>")
                    || trimmed_prev.ends_with("else {")
                    || trimmed_prev.ends_with("try {")
                    || trimmed_prev.ends_with("catch {")
            }
            Language::None => false,
        };

        if increase_indent {
            prev_indent + 4
        } else {
            prev_indent
        }
    }

    fn get_indent_string(&self, level: usize) -> String {
        " ".repeat(level)
    }

    fn indent(&mut self) {
        self.save_history_state();
        let indent = self.get_indent_string(4);

        if self.cursor_x == 0
            || self.buffer[self.cursor_y]
                .iter()
                .take(self.cursor_x)
                .all(|&c| c == ' ' || c == '\t')
        {
            for c in indent.chars() {
                self.buffer[self.cursor_y].insert(self.cursor_x, c);
                self.cursor_x += 1;
            }
        } else {
            for c in indent.chars() {
                self.buffer[self.cursor_y].insert(self.cursor_x, c);
                self.cursor_x += 1;
            }
        }

        self.cursor_locked = false;
        self.needs_full_redraw = true;
        self.dirty = true;
    }

    fn unindent(&mut self) {
        self.save_history_state();
        let line = &mut self.buffer[self.cursor_y];

        if line.is_empty() {
            return;
        }

        let mut removed = 0;

        while removed < line.len() && removed < 4 {
            if line[0] == ' ' {
                line.remove(0);
                removed += 1;
                if self.cursor_x > 0 {
                    self.cursor_x -= 1;
                }
            } else if line[0] == '\t' {
                line.remove(0);
                removed += 4;
                if self.cursor_x > 0 {
                    self.cursor_x -= 1;
                }
            } else {
                break;
            }
        }

        self.cursor_locked = false;
        self.needs_full_redraw = true;
        self.dirty = true;
    }

    fn ensure_terminal_session(&mut self) -> io::Result<()> {
        if self.terminal_session.is_some() {
            return Ok(());
        }

        let (cols, rows) = terminal::size().unwrap_or((80, 24));
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(terminal_pty_size(rows, cols))
            .map_err(to_io_error)?;
        let shell = terminal_shell_program();
        let cmd = CommandBuilder::new(shell);
        let child = pair.slave.spawn_command(cmd).map_err(to_io_error)?;
        let mut reader = pair.master.try_clone_reader().map_err(to_io_error)?;
        let writer = pair.master.take_writer().map_err(to_io_error)?;
        let (tx, rx) = mpsc::channel();

        thread::spawn(move || {
            let mut buffer = [0_u8; 4096];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) => {
                        let _ = tx.send("\n[Shell exited]\n".to_string());
                        break;
                    }
                    Ok(n) => {
                        let chunk = String::from_utf8_lossy(&buffer[..n]).into_owned();
                        if tx.send(chunk).is_err() {
                            break;
                        }
                    }
                    Err(err) => {
                        let _ = tx.send(format!("\n[Terminal read error: {}]\n", err));
                        break;
                    }
                }
            }
        });

        self.terminal_output_rx = Some(rx);
        self.terminal_session = Some(TerminalSession {
            master: pair.master,
            writer,
            child,
        });
        self.terminal_scroll = 0;
        self.needs_full_redraw = true;
        self.dirty = true;
        Ok(())
    }

    fn resize_terminal_session(&mut self, rows: u16, cols: u16) {
        if let Some(session) = self.terminal_session.as_mut() {
            let _ = session.master.resize(terminal_pty_size(rows, cols));
        }
        self.terminal_parser
            .screen_mut()
            .set_size(rows.max(1), cols.max(1));
        self.sync_terminal_scrollback();
    }

    fn sync_terminal_scrollback(&mut self) {
        self.terminal_parser
            .screen_mut()
            .set_scrollback(self.terminal_scroll);
        self.terminal_scroll = self.terminal_parser.screen().scrollback();
    }

    fn process_terminal_bytes(&mut self, bytes: &[u8]) {
        self.terminal_parser.process(bytes);
        self.sync_terminal_scrollback();
    }

    fn append_terminal_message(&mut self, message: &str) {
        self.process_terminal_bytes(message.as_bytes());
    }

    fn scroll_terminal(&mut self, delta: isize) {
        if delta.is_negative() {
            self.terminal_scroll = self.terminal_scroll.saturating_sub(delta.unsigned_abs());
        } else {
            self.terminal_scroll = self.terminal_scroll.saturating_add(delta as usize);
        }
        self.sync_terminal_scrollback();
        self.needs_full_redraw = true;
        self.dirty = true;
    }

    fn reset_terminal_scrollback(&mut self) {
        if self.terminal_scroll != 0 {
            self.terminal_scroll = 0;
            self.sync_terminal_scrollback();
        }
    }

    fn write_terminal_bytes(&mut self, bytes: &[u8]) {
        if let Err(err) = self.ensure_terminal_session() {
            self.append_terminal_message(&format!("\r\n[Failed to start shell: {}]\r\n", err));
            self.needs_full_redraw = true;
            self.dirty = true;
            return;
        }

        self.reset_terminal_scrollback();

        if let Some(session) = self.terminal_session.as_mut() {
            let result = session
                .writer
                .write_all(bytes)
                .and_then(|_| session.writer.flush());

            if result.is_err() {
                self.append_terminal_message("\r\n[Failed to write to shell]\r\n");
                self.terminal_output_rx = None;
                self.terminal_session = None;
            }
        }

        self.needs_full_redraw = true;
        self.dirty = true;
    }

    fn terminal_key_bytes(&self, code: KeyCode, modifiers: KeyModifiers) -> Option<Vec<u8>> {
        let application_cursor = self.terminal_parser.screen().application_cursor();
        let alt = modifiers.contains(KeyModifiers::ALT);
        let control = modifiers.contains(KeyModifiers::CONTROL);

        let mut bytes = match code {
            KeyCode::Enter => vec![b'\r'],
            KeyCode::Backspace => vec![0x7f],
            KeyCode::Tab => vec![b'\t'],
            KeyCode::BackTab => b"\x1b[Z".to_vec(),
            KeyCode::Esc => vec![0x1b],
            KeyCode::Left => {
                if application_cursor {
                    b"\x1bOD".to_vec()
                } else {
                    b"\x1b[D".to_vec()
                }
            }
            KeyCode::Right => {
                if application_cursor {
                    b"\x1bOC".to_vec()
                } else {
                    b"\x1b[C".to_vec()
                }
            }
            KeyCode::Up => {
                if application_cursor {
                    b"\x1bOA".to_vec()
                } else {
                    b"\x1b[A".to_vec()
                }
            }
            KeyCode::Down => {
                if application_cursor {
                    b"\x1bOB".to_vec()
                } else {
                    b"\x1b[B".to_vec()
                }
            }
            KeyCode::Home => b"\x1b[H".to_vec(),
            KeyCode::End => b"\x1b[F".to_vec(),
            KeyCode::Delete => b"\x1b[3~".to_vec(),
            KeyCode::Insert => b"\x1b[2~".to_vec(),
            KeyCode::PageUp => b"\x1b[5~".to_vec(),
            KeyCode::PageDown => b"\x1b[6~".to_vec(),
            KeyCode::Char(c) => {
                if control && !alt {
                    let lower = c.to_ascii_lowercase();
                    let control_byte = match lower {
                        'a'..='z' => lower as u8 - b'a' + 1,
                        ' ' | '@' => 0,
                        '[' => 27,
                        '\\' => 28,
                        ']' => 29,
                        '^' => 30,
                        '_' => 31,
                        _ => return None,
                    };
                    vec![control_byte]
                } else {
                    c.to_string().into_bytes()
                }
            }
            _ => return None,
        };

        if alt && !control && matches!(code, KeyCode::Char(_)) {
            bytes.insert(0, 0x1b);
        }

        Some(bytes)
    }

    fn handle_terminal_key_event(&mut self, code: KeyCode, modifiers: KeyModifiers) {
        match (code, modifiers) {
            (KeyCode::Char('1'), KeyModifiers::CONTROL) => {
                self.toggle_terminal();
            }
            (KeyCode::PageUp, m) if m.contains(KeyModifiers::SHIFT) => {
                self.scroll_terminal(
                    terminal::size()
                        .map(|(_, rows)| rows as isize)
                        .unwrap_or(24),
                );
            }
            (KeyCode::PageDown, m) if m.contains(KeyModifiers::SHIFT) => {
                self.scroll_terminal(
                    -terminal::size()
                        .map(|(_, rows)| rows as isize)
                        .unwrap_or(24),
                );
            }
            _ => {
                if let Some(bytes) = self.terminal_key_bytes(code, modifiers) {
                    self.write_terminal_bytes(&bytes);
                }
            }
        }
    }

    fn drain_terminal_output(&mut self) {
        let mut disconnected = false;
        let mut chunks = Vec::new();

        if let Some(rx) = self.terminal_output_rx.as_ref() {
            loop {
                match rx.try_recv() {
                    Ok(chunk) => chunks.push(chunk),
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        disconnected = true;
                        break;
                    }
                }
            }
        }

        if chunks.is_empty() && !disconnected {
            return;
        }

        for chunk in chunks {
            self.process_terminal_bytes(chunk.as_bytes());
        }

        if disconnected {
            self.terminal_output_rx = None;
            self.terminal_session = None;
        }

        self.needs_full_redraw = true;
        self.dirty = true;
    }

    fn close_terminal_session(&mut self) {
        self.terminal_output_rx = None;
        if let Some(mut session) = self.terminal_session.take() {
            let _ = session.child.kill();
            let _ = session.child.wait();
        }
    }

    fn interrupt_terminal(&mut self) {
        self.write_terminal_bytes(&[3]);
    }

    fn toggle_terminal(&mut self) {
        self.terminal_show = !self.terminal_show;
        if self.terminal_show {
            self.mode = EditorMode::Terminal;
            if let Err(err) = self.ensure_terminal_session() {
                self.append_terminal_message(&format!("\r\n[Failed to start shell: {}]\r\n", err));
            } else {
                let (cols, rows) = terminal::size().unwrap_or((80, 24));
                self.resize_terminal_session(rows, cols);
            }
        } else {
            self.mode = EditorMode::Normal;
        }
        self.needs_full_redraw = true;
        self.dirty = true;
    }

    fn start_goto_line(&mut self) {
        self.mode = EditorMode::GoToLine;
        self.goto_line_input.clear();
        self.status = "Go to line: ".into();
        self.needs_full_redraw = true;
        self.dirty = true;
    }

    fn cancel_goto_line(&mut self) {
        self.mode = EditorMode::Normal;
        self.goto_line_input.clear();
        self.status = "Ctrl+O Tree | Ctrl+N File | Ctrl+M Folder | F2 Rename | Del Delete | Ctrl+S Save | Ctrl+F Find | Shift+Arrow Select | Ctrl+C Copy | Ctrl+V Paste | Ctrl+1 Terminal | Ctrl+Q Quit".into();
        self.needs_full_redraw = true;
        self.dirty = true;
    }

    fn confirm_goto_line(&mut self) {
        if self.goto_line_input.is_empty() {
            return;
        }

        let line_str: String = self.goto_line_input.iter().collect();
        if let Ok(line_num) = line_str.parse::<usize>() {
            if line_num > 0 && line_num <= self.buffer.len() {
                self.cursor_y = line_num - 1;
                if let Some(line) = self.buffer.get(self.cursor_y) {
                    self.cursor_x = self.cursor_x.min(line.len());
                } else {
                    self.cursor_x = 0;
                }
                self.cursor_locked = false;
                self.dirty = true;
            }
        }
        self.cancel_goto_line();
    }

    fn find_matching_bracket(&mut self, y: usize, x: usize) -> Option<(usize, usize)> {
        if y >= self.buffer.len() {
            return None;
        }

        let line = &self.buffer[y];
        if x >= line.len() {
            return None;
        }

        let char_at = line[x];

        let (open, close, forward) = match char_at {
            '(' => ('(', ')', true),
            ')' => (')', '(', false),
            '[' => ('[', ']', true),
            ']' => (']', '[', false),
            '{' => ('{', '}', true),
            '}' => ('}', '{', false),
            _ => return None,
        };

        let mut depth = 0;
        let mut current_y = y;
        let mut current_x = if forward { x + 1 } else { x.saturating_sub(1) };

        loop {
            if current_y >= self.buffer.len() {
                break;
            }

            let line = &self.buffer[current_y];

            while (forward && current_x < line.len()) || (!forward && current_x > 0) {
                let c = if forward {
                    line[current_x]
                } else {
                    line[current_x - 1]
                };

                if c == open {
                    depth += 1;
                } else if c == close {
                    if depth == 0 {
                        return Some((current_y, if forward { current_x } else { current_x - 1 }));
                    }
                    depth -= 1;
                }

                if forward {
                    current_x += 1;
                } else {
                    current_x = current_x.saturating_sub(1);
                }
            }

            if forward {
                current_y += 1;
                current_x = 0;
            } else {
                if current_y == 0 {
                    break;
                }
                current_y -= 1;
                current_x = self.buffer[current_y].len();
            }
        }

        None
    }

    fn update_bracket_matching(&mut self) {
        self.matched_bracket = None;

        if self.cursor_y >= self.buffer.len() {
            return;
        }

        let line = &self.buffer[self.cursor_y];
        if self.cursor_x >= line.len() {
            return;
        }

        if let Some(matched) = self.find_matching_bracket(self.cursor_y, self.cursor_x) {
            self.matched_bracket = Some(matched);
        } else {
            if self.cursor_x > 0 {
                if let Some(matched) = self.find_matching_bracket(self.cursor_y, self.cursor_x - 1)
                {
                    self.matched_bracket = Some(matched);
                }
            }
        }
    }

    fn word_left(&mut self) {
        if self.cursor_x == 0 {
            if self.cursor_y > 0 {
                self.cursor_y -= 1;
                self.cursor_x = self.buffer[self.cursor_y].len();
                self.cursor_locked = false;
                self.dirty = true;
            }
            return;
        }

        let line = &self.buffer[self.cursor_y];
        let mut x = self.cursor_x;

        while x > 0 && (line[x - 1] == ' ' || line[x - 1] == '\t') {
            x -= 1;
        }

        if x == 0 {
            self.cursor_x = x;
            self.cursor_locked = false;
            self.update_bracket_matching();
            self.dirty = true;
            return;
        }

        let is_special_char = |c: char| -> bool {
            matches!(
                c,
                '.' | ','
                    | '['
                    | ']'
                    | '{'
                    | '}'
                    | '$'
                    | '('
                    | ')'
                    | ';'
                    | ':'
                    | '!'
                    | '?'
                    | '@'
                    | '#'
                    | '%'
                    | '^'
                    | '&'
                    | '*'
                    | '+'
                    | '-'
                    | '='
                    | '/'
                    | '\\'
                    | '|'
                    | '<'
                    | '>'
                    | '`'
                    | '\''
                    | '"'
            )
        };

        let prev_char = line[x - 1];

        if is_special_char(prev_char) {
            x -= 1;
        } else if prev_char.is_alphanumeric() || prev_char == '_' {
            while x > 0 {
                let c = line[x - 1];
                if c.is_alphanumeric() || c == '_' {
                    x -= 1;
                } else {
                    break;
                }
            }
        }

        self.cursor_x = x;
        self.cursor_locked = false;
        self.update_bracket_matching();
        self.dirty = true;
    }

    fn word_right(&mut self) {
        let line = &self.buffer[self.cursor_y];

        if self.cursor_x >= line.len() {
            if self.cursor_y + 1 < self.buffer.len() {
                self.cursor_y += 1;
                self.cursor_x = 0;
                self.cursor_locked = false;
                self.update_bracket_matching();
                self.dirty = true;
            }
            return;
        }

        let mut x = self.cursor_x;

        while x < line.len() && (line[x] == ' ' || line[x] == '\t') {
            x += 1;
        }

        if x >= line.len() {
            self.cursor_x = x;
            self.cursor_locked = false;
            self.update_bracket_matching();
            self.dirty = true;
            return;
        }

        let is_special_char = |c: char| -> bool {
            matches!(
                c,
                '.' | ','
                    | '['
                    | ']'
                    | '{'
                    | '}'
                    | '$'
                    | '('
                    | ')'
                    | ';'
                    | ':'
                    | '!'
                    | '?'
                    | '@'
                    | '#'
                    | '%'
                    | '^'
                    | '&'
                    | '*'
                    | '+'
                    | '-'
                    | '='
                    | '/'
                    | '\\'
                    | '|'
                    | '<'
                    | '>'
                    | '`'
                    | '\''
                    | '"'
            )
        };

        let current_char = line[x];

        if is_special_char(current_char) {
            x += 1;
        } else if current_char.is_alphanumeric() || current_char == '_' {
            while x < line.len() {
                let c = line[x];
                if c.is_alphanumeric() || c == '_' {
                    x += 1;
                } else {
                    break;
                }
            }
        }

        self.cursor_x = x;
        self.cursor_locked = false;
        self.update_bracket_matching();
        self.dirty = true;
    }

    fn delete_word_backward(&mut self) {
        if self.cursor_x == 0 {
            if self.cursor_y > 0 {
                self.save_history_state();
                let current_line = self.buffer.remove(self.cursor_y);
                self.cursor_y -= 1;
                self.cursor_x = self.buffer[self.cursor_y].len();
                self.buffer[self.cursor_y].extend(current_line);
                self.mark_file_dirty();
            }
            return;
        }

        let line = &self.buffer[self.cursor_y];
        let mut start = self.cursor_x;

        let is_special_char = |c: char| -> bool {
            matches!(
                c,
                '.' | ','
                    | '['
                    | ']'
                    | '{'
                    | '}'
                    | '$'
                    | '('
                    | ')'
                    | ';'
                    | ':'
                    | '!'
                    | '?'
                    | '@'
                    | '#'
                    | '%'
                    | '^'
                    | '&'
                    | '*'
                    | '+'
                    | '-'
                    | '='
                    | '/'
                    | '\\'
                    | '|'
                    | '<'
                    | '>'
                    | '`'
                    | '\''
                    | '"'
            )
        };

        while start > 0 && (line[start - 1] == ' ' || line[start - 1] == '\t') {
            start -= 1;
        }

        if start < self.cursor_x {
            self.save_history_state();
            let line = &mut self.buffer[self.cursor_y];
            line.drain(start..self.cursor_x);
            self.cursor_x = start;
            self.mark_file_dirty();
            return;
        }

        if start > 0 {
            let prev_char = line[start - 1];

            if is_special_char(prev_char) {
                start -= 1;
            } else if prev_char.is_alphanumeric() || prev_char == '_' {
                while start > 0 {
                    let c = line[start - 1];
                    if c.is_alphanumeric() || c == '_' {
                        start -= 1;
                    } else {
                        break;
                    }
                }
            }
        }

        if start < self.cursor_x {
            self.save_history_state();
            let line = &mut self.buffer[self.cursor_y];
            line.drain(start..self.cursor_x);
            self.cursor_x = start;
            self.mark_file_dirty();
        }
    }

    fn delete_word_forward(&mut self) {
        let line = &self.buffer[self.cursor_y];

        if self.cursor_x >= line.len() {
            if self.cursor_y + 1 < self.buffer.len() {
                self.save_history_state();
                let next_line = self.buffer.remove(self.cursor_y + 1);
                self.buffer[self.cursor_y].extend(next_line);
                self.mark_file_dirty();
            }
            return;
        }

        let mut end = self.cursor_x;

        let is_special_char = |c: char| -> bool {
            matches!(
                c,
                '.' | ','
                    | '['
                    | ']'
                    | '{'
                    | '}'
                    | '$'
                    | '('
                    | ')'
                    | ';'
                    | ':'
                    | '!'
                    | '?'
                    | '@'
                    | '#'
                    | '%'
                    | '^'
                    | '&'
                    | '*'
                    | '+'
                    | '-'
                    | '='
                    | '/'
                    | '\\'
                    | '|'
                    | '<'
                    | '>'
                    | '`'
                    | '\''
                    | '"'
            )
        };

        while end < line.len() && (line[end] == ' ' || line[end] == '\t') {
            end += 1;
        }

        if end > self.cursor_x {
            self.save_history_state();
            let line = &mut self.buffer[self.cursor_y];
            line.drain(self.cursor_x..end);
            self.mark_file_dirty();
            return;
        }

        if end < line.len() {
            let current_char = line[end];

            if is_special_char(current_char) {
                end += 1;
            } else if current_char.is_alphanumeric() || current_char == '_' {
                while end < line.len() {
                    let c = line[end];
                    if c.is_alphanumeric() || c == '_' {
                        end += 1;
                    } else {
                        break;
                    }
                }
            }
        }

        let line = &self.buffer[self.cursor_y];
        while end < line.len() && (line[end] == ' ' || line[end] == '\t') {
            end += 1;
        }

        if end > self.cursor_x {
            self.save_history_state();
            let line = &mut self.buffer[self.cursor_y];
            line.drain(self.cursor_x..end);
            self.mark_file_dirty();
        }
    }

    fn get_word_at_cursor(&self) -> Option<(String, usize)> {
        if self.cursor_y >= self.buffer.len() {
            return None;
        }

        let line = &self.buffer[self.cursor_y];
        if self.cursor_x == 0 {
            return None;
        }

        let mut start = self.cursor_x;

        while start > 0 {
            let c = line[start - 1];
            if c.is_alphanumeric() || c == '_' {
                start -= 1;
            } else {
                break;
            }
        }

        if start == self.cursor_x {
            return None;
        }

        let word: String = line[start..self.cursor_x].iter().collect();
        Some((word, start))
    }

    fn collect_words_from_buffer(&self) -> Vec<String> {
        let mut words: HashSet<String> = HashSet::new();

        for line in &self.buffer {
            let line_str: String = line.iter().collect();
            let mut word = String::new();
            for c in line_str.chars() {
                if c.is_alphanumeric() || c == '_' {
                    word.push(c);
                } else {
                    if word.len() >= 2 {
                        words.insert(word.clone());
                    }
                    word.clear();
                }
            }
            if word.len() >= 2 {
                words.insert(word);
            }
        }

        words.into_iter().collect()
    }

    fn start_autocomplete(&mut self) {
        if let Some((prefix, _start)) = self.get_word_at_cursor() {
            if prefix.is_empty() {
                return;
            }

            let all_words = self.collect_words_from_buffer();
            let mut suggestions: Vec<String> = all_words
                .into_iter()
                .filter(|w| w.starts_with(&prefix) && w != &prefix)
                .collect();

            let keywords = get_keywords(&self.language);
            for kw in keywords {
                if kw.starts_with(&prefix) && kw != &prefix {
                    let kw_str = kw.to_string();
                    if !suggestions.contains(&kw_str) {
                        suggestions.push(kw_str);
                    }
                }
            }

            suggestions.sort();

            if !suggestions.is_empty() {
                self.autocomplete_prefix = prefix;
                self.autocomplete_suggestions = suggestions;
                self.autocomplete_index = 0;
                self.mode = EditorMode::Autocomplete;
                self.needs_full_redraw = true;
                self.dirty = true;
            }
        }
    }

    fn apply_autocomplete(&mut self) {
        if self.autocomplete_suggestions.is_empty() {
            self.cancel_autocomplete();
            return;
        }

        let selected = &self.autocomplete_suggestions[self.autocomplete_index].clone();

        if let Some((_prefix, start)) = self.get_word_at_cursor() {
            self.save_history_state();

            let line = &mut self.buffer[self.cursor_y];
            line.drain(start..self.cursor_x);
            self.cursor_x = start;

            for c in selected.chars() {
                self.buffer[self.cursor_y].insert(self.cursor_x, c);
                self.cursor_x += 1;
            }

            self.mark_file_dirty();
        }

        self.cancel_autocomplete();
    }

    fn cancel_autocomplete(&mut self) {
        self.mode = EditorMode::Normal;
        self.autocomplete_suggestions.clear();
        self.autocomplete_index = 0;
        self.autocomplete_prefix.clear();
        self.needs_full_redraw = true;
        self.dirty = true;
    }

    fn next_autocomplete(&mut self) {
        if !self.autocomplete_suggestions.is_empty() {
            self.autocomplete_index =
                (self.autocomplete_index + 1) % self.autocomplete_suggestions.len();
            self.dirty = true;
        }
    }

    fn prev_autocomplete(&mut self) {
        if !self.autocomplete_suggestions.is_empty() {
            if self.autocomplete_index == 0 {
                self.autocomplete_index = self.autocomplete_suggestions.len() - 1;
            } else {
                self.autocomplete_index -= 1;
            }
            self.dirty = true;
        }
    }

    fn execute_terminal_command(&mut self) {
        self.write_terminal_bytes(b"\r");
    }
}

fn dashboard_action_title(action: &DashboardAction) -> String {
    match action {
        DashboardAction::OpenWorkspace(_) => "Open current workspace".into(),
        DashboardAction::NewBuffer => "Start with an empty buffer".into(),
        DashboardAction::RecentProject(path) => {
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            format!("Project  {}", name)
        }
        DashboardAction::RecentFile(path) => {
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            format!("File     {}", name)
        }
    }
}

fn dashboard_action_meta(action: &DashboardAction, width: usize) -> String {
    let raw = match action {
        DashboardAction::OpenWorkspace(path) | DashboardAction::RecentProject(path) => {
            path.to_string_lossy().into_owned()
        }
        DashboardAction::NewBuffer => "No file selected yet".into(),
        DashboardAction::RecentFile(path) => path
            .parent()
            .map(|parent| parent.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string_lossy().into_owned()),
    };
    shorten_text_middle(&raw, width)
}

fn draw_centered_text(
    out: &mut io::Stdout,
    row: u16,
    cols: u16,
    text: &str,
    color: Color,
    bold: bool,
) -> io::Result<()> {
    let text_width = text.chars().count() as u16;
    let x = cols.saturating_sub(text_width) / 2;
    execute!(out, cursor::MoveTo(x, row), SetForegroundColor(color))?;
    if bold {
        execute!(out, SetAttribute(Attribute::Bold))?;
    }
    write!(out, "{}", text)?;
    execute!(
        out,
        SetAttribute(Attribute::Reset),
        SetForegroundColor(Color::Reset)
    )?;
    Ok(())
}

fn draw_dashboard(ed: &mut Editor, out: &mut io::Stdout, cols: u16, rows: u16) -> io::Result<()> {
    execute!(
        out,
        terminal::Clear(terminal::ClearType::All),
        cursor::Hide,
        SetAttribute(Attribute::Reset),
        SetForegroundColor(Color::Reset),
        crossterm::style::SetBackgroundColor(Color::Reset)
    )?;

    for (i, line) in DASHBOARD_LOGO.iter().enumerate() {
        let color = match i {
            0 | 1 => Color::Cyan,
            2 | 3 => Color::Blue,
            _ => Color::White,
        };
        draw_centered_text(out, 1 + i as u16, cols, line, color, true)?;
    }

    draw_centered_text(
        out,
        8,
        cols,
        "Launch pad for recent work, fast starts, and clean sessions",
        Color::DarkGrey,
        false,
    )?;

    let panel_width = cols.saturating_sub(6).min(86);
    let panel_x = cols.saturating_sub(panel_width) / 2;
    let mut row = 10;

    execute!(
        out,
        cursor::MoveTo(panel_x, row),
        SetForegroundColor(Color::DarkGrey),
        SetAttribute(Attribute::Bold)
    )?;
    write!(out, "Start")?;
    execute!(
        out,
        SetAttribute(Attribute::Reset),
        SetForegroundColor(Color::Reset)
    )?;
    row += 1;

    let actions = ed.dashboard_actions();
    let meta_width = panel_width.saturating_sub(8) as usize;
    let draw_item = |out: &mut io::Stdout,
                     row: &mut u16,
                     index: usize,
                     action: &DashboardAction|
     -> io::Result<()> {
        if *row >= rows.saturating_sub(6) {
            return Ok(());
        }

        execute!(out, cursor::MoveTo(panel_x, *row))?;
        let prefix = format!("{:>2}.", index + 1);
        let title = dashboard_action_title(action);
        let meta = dashboard_action_meta(action, meta_width);

        if index == ed.dashboard_selection {
            execute!(
                out,
                crossterm::style::SetBackgroundColor(Color::Blue),
                SetForegroundColor(Color::White),
                SetAttribute(Attribute::Bold)
            )?;
            write!(
                out,
                " {} {:<width$} ",
                prefix,
                shorten_text_middle(&title, panel_width.saturating_sub(6) as usize),
                width = panel_width.saturating_sub(6) as usize
            )?;
            execute!(
                out,
                crossterm::style::SetBackgroundColor(Color::Reset),
                SetForegroundColor(Color::Reset),
                SetAttribute(Attribute::Reset)
            )?;
            *row += 1;
            if *row < rows.saturating_sub(5) {
                execute!(
                    out,
                    cursor::MoveTo(panel_x + 4, *row),
                    SetForegroundColor(Color::Grey)
                )?;
                write!(
                    out,
                    "{:<width$}",
                    meta,
                    width = panel_width.saturating_sub(4) as usize
                )?;
                execute!(out, SetForegroundColor(Color::Reset))?;
            }
        } else {
            execute!(out, SetForegroundColor(Color::White))?;
            write!(
                out,
                " {} {}",
                prefix,
                shorten_text_middle(&title, panel_width.saturating_sub(6) as usize)
            )?;
            *row += 1;
            if *row < rows.saturating_sub(5) {
                execute!(
                    out,
                    cursor::MoveTo(panel_x + 4, *row),
                    SetForegroundColor(Color::DarkGrey)
                )?;
                write!(out, "{}", meta)?;
                execute!(out, SetForegroundColor(Color::Reset))?;
            }
        }

        *row += 1;
        Ok(())
    };

    draw_item(out, &mut row, 0, &actions[0])?;
    draw_item(out, &mut row, 1, &actions[1])?;

    row += 1;
    execute!(
        out,
        cursor::MoveTo(panel_x, row),
        SetForegroundColor(Color::DarkGrey),
        SetAttribute(Attribute::Bold)
    )?;
    write!(out, "Recent Projects")?;
    execute!(
        out,
        SetAttribute(Attribute::Reset),
        SetForegroundColor(Color::Reset)
    )?;
    row += 1;

    if ed.recent_projects.is_empty() {
        execute!(
            out,
            cursor::MoveTo(panel_x + 4, row),
            SetForegroundColor(Color::DarkGrey)
        )?;
        write!(out, "No recent projects yet")?;
        execute!(out, SetForegroundColor(Color::Reset))?;
        row += 2;
    } else {
        for (offset, action) in actions
            .iter()
            .skip(2)
            .take(ed.recent_projects.len())
            .enumerate()
        {
            draw_item(out, &mut row, 2 + offset, action)?;
        }
    }

    execute!(
        out,
        cursor::MoveTo(panel_x, row),
        SetForegroundColor(Color::DarkGrey),
        SetAttribute(Attribute::Bold)
    )?;
    write!(out, "Recent Files")?;
    execute!(
        out,
        SetAttribute(Attribute::Reset),
        SetForegroundColor(Color::Reset)
    )?;
    row += 1;

    if ed.recent_files.is_empty() {
        execute!(
            out,
            cursor::MoveTo(panel_x + 4, row),
            SetForegroundColor(Color::DarkGrey)
        )?;
        write!(out, "No recent files yet")?;
        execute!(out, SetForegroundColor(Color::Reset))?;
    } else {
        for (offset, action) in actions
            .iter()
            .skip(2 + ed.recent_projects.len())
            .take(ed.recent_files.len())
            .enumerate()
        {
            draw_item(out, &mut row, 2 + ed.recent_projects.len() + offset, action)?;
        }
    }

    let shortcuts_row = rows.saturating_sub(5);
    execute!(
        out,
        cursor::MoveTo(panel_x, shortcuts_row),
        SetForegroundColor(Color::DarkGrey),
        SetAttribute(Attribute::Bold)
    )?;
    write!(out, "Shortcuts")?;
    execute!(
        out,
        SetAttribute(Attribute::Reset),
        SetForegroundColor(Color::Reset)
    )?;

    execute!(
        out,
        cursor::MoveTo(panel_x, shortcuts_row + 1),
        SetForegroundColor(Color::Grey)
    )?;
    write!(out, "Ctrl+O Tree   Ctrl+1 Terminal   Ctrl+F Search")?;
    execute!(out, cursor::MoveTo(panel_x, shortcuts_row + 2))?;
    write!(out, "Ctrl+N File   Ctrl+M Folder     Ctrl+G Go to line")?;
    execute!(out, SetForegroundColor(Color::Reset))?;

    execute!(
        out,
        cursor::MoveTo(0, rows - 2),
        SetForegroundColor(Color::DarkGrey)
    )?;
    write!(
        out,
        "{:^width$}",
        "Enter open | Up/Down navigate | O workspace | N new buffer | R refresh | Ctrl+Q quit",
        width = cols as usize
    )?;
    execute!(out, SetForegroundColor(Color::Reset))?;

    execute!(out, cursor::MoveTo(0, rows - 1))?;
    write!(out, "{:^width$}", "Termi Dashboard", width = cols as usize)?;

    out.flush()?;
    ed.needs_full_redraw = false;
    Ok(())
}

fn draw(ed: &mut Editor, out: &mut io::Stdout) -> io::Result<()> {
    let (cols, rows) = terminal::size()?;

    if matches!(ed.mode, EditorMode::Terminal) {
        let formatted = ed.terminal_parser.screen().contents_formatted();
        out.write_all(&formatted)?;
        out.flush()?;
        return Ok(());
    }

    if matches!(ed.mode, EditorMode::Dashboard) {
        return draw_dashboard(ed, out, cols, rows);
    }

    execute!(
        out,
        cursor::Hide,
        SetAttribute(Attribute::Reset),
        SetForegroundColor(Color::Reset),
        crossterm::style::SetBackgroundColor(Color::Reset)
    )?;

    let max_lines = rows - STATUS_HEIGHT;
    let tree_offset = if ed.show_tree { TREE_WIDTH } else { 0 };
    let line_num_offset = if ed.show_line_numbers {
        LINE_NUM_WIDTH
    } else {
        0
    };
    let text_offset = tree_offset + line_num_offset;

    let scroll_changed = ed.scroll_y != ed.last_scroll_y || ed.scroll_x != ed.last_scroll_x;
    let tree_scroll_changed =
        ed.show_tree && (ed.tree_scroll != ed.last_tree_scroll || ed.needs_full_redraw);
    let should_clear = ed.needs_full_redraw || scroll_changed;

    if should_clear {
        if scroll_changed && !ed.needs_full_redraw {
            for y in 0..max_lines {
                execute!(out, cursor::MoveTo(0, y))?;
                write!(out, "\x1b[K")?;
            }
        } else {
            execute!(out, terminal::Clear(terminal::ClearType::All))?;
        }
    }

    if matches!(ed.mode, EditorMode::DeleteConfirm) {
        let dialog_y = (rows / 2) as u16;
        let dialog_x = (cols / 2).saturating_sub(20);
        execute!(out, cursor::MoveTo(dialog_x, dialog_y))?;
        execute!(out, SetForegroundColor(Color::Red))?;
        execute!(out, SetAttribute(Attribute::Bold))?;
        write!(out, "═══════════════════════════")?;
        execute!(out, SetAttribute(Attribute::Reset))?;
        execute!(out, SetForegroundColor(Color::White))?;

        execute!(out, cursor::MoveTo(dialog_x, dialog_y + 1))?;
        if let Some(target) = &ed.delete_target {
            let item_type = if target.is_dir() { "Folder" } else { "File" };
            let name = target.file_name().unwrap_or_default().to_string_lossy();
            write!(out, " Delete {}?", item_type)?;
            execute!(out, cursor::MoveTo(dialog_x, dialog_y + 2))?;
            write!(out, "  {}", name)?;
        }

        execute!(out, cursor::MoveTo(dialog_x, dialog_y + 3))?;
        write!(out, " Y - Yes  |  N - No")?;
        execute!(out, cursor::MoveTo(dialog_x, dialog_y + 4))?;
        execute!(out, SetForegroundColor(Color::Red))?;
        write!(out, "═══════════════════════════")?;
        execute!(out, SetAttribute(Attribute::Reset))?;
        execute!(out, SetForegroundColor(Color::White))?;
    }

    if matches!(ed.mode, EditorMode::Rename) {
        let dialog_y = (rows / 2) as u16;
        let dialog_x = (cols / 2).saturating_sub(20);
        execute!(out, cursor::MoveTo(dialog_x, dialog_y))?;
        execute!(out, SetForegroundColor(Color::Cyan))?;
        execute!(out, SetAttribute(Attribute::Bold))?;
        write!(out, "═══════════════════════════")?;
        execute!(out, SetAttribute(Attribute::Reset))?;
        execute!(out, SetForegroundColor(Color::White))?;

        execute!(out, cursor::MoveTo(dialog_x, dialog_y + 1))?;
        write!(out, " Rename:")?;
        execute!(out, cursor::MoveTo(dialog_x, dialog_y + 2))?;
        let rename_name: String = ed.rename_name.iter().collect();
        write!(out, "  {}", rename_name)?;
        execute!(out, cursor::MoveTo(dialog_x, dialog_y + 3))?;
        write!(out, " Enter - Confirm  |  Esc - Cancel")?;
        execute!(out, cursor::MoveTo(dialog_x, dialog_y + 4))?;
        execute!(out, SetForegroundColor(Color::Cyan))?;
        write!(out, "═══════════════════════════")?;
        execute!(out, SetAttribute(Attribute::Reset))?;
        execute!(out, SetForegroundColor(Color::White))?;
    }

    if matches!(ed.mode, EditorMode::GoToLine) {
        let dialog_y = (rows / 2) as u16;
        let dialog_x = (cols / 2).saturating_sub(20);
        execute!(out, cursor::MoveTo(dialog_x, dialog_y))?;
        execute!(out, SetForegroundColor(Color::Yellow))?;
        execute!(out, SetAttribute(Attribute::Bold))?;
        write!(out, "═══════════════════════════")?;
        execute!(out, SetAttribute(Attribute::Reset))?;
        execute!(out, SetForegroundColor(Color::White))?;

        execute!(out, cursor::MoveTo(dialog_x, dialog_y + 1))?;
        write!(out, " Go to Line:")?;
        execute!(out, cursor::MoveTo(dialog_x, dialog_y + 2))?;
        let line_input: String = ed.goto_line_input.iter().collect();
        write!(out, "  {}", line_input)?;
        execute!(out, cursor::MoveTo(dialog_x, dialog_y + 3))?;
        write!(out, " Enter - Go  |  Esc - Cancel")?;
        execute!(out, cursor::MoveTo(dialog_x, dialog_y + 4))?;
        execute!(out, SetForegroundColor(Color::Yellow))?;
        write!(out, "═══════════════════════════")?;
        execute!(out, SetAttribute(Attribute::Reset))?;
        execute!(out, SetForegroundColor(Color::White))?;
    }

    if ed.show_tree {
        let tree_max_scroll = ed.tree.len().saturating_sub(max_lines as usize);
        ed.tree_scroll = ed.tree_scroll.min(tree_max_scroll);

        let tree_scroll_changed = ed.tree_scroll != ed.last_tree_scroll || ed.needs_full_redraw;
        if tree_scroll_changed {
            for y in 0..max_lines {
                execute!(out, cursor::MoveTo(0, y))?;
                write!(out, "{:width$}", "", width = TREE_WIDTH as usize)?;
            }
        }

        for (screen_i, tree_i) in (ed.tree_scroll..ed.tree.len())
            .enumerate()
            .take(max_lines as usize)
        {
            if let Some(n) = ed.tree.get(tree_i) {
                execute!(out, cursor::MoveTo(0, screen_i as u16))?;
                let mark = if tree_i == ed.tree_cursor { ">" } else { " " };
                let icon = if n.is_dir { "📁" } else { "📄" };
                let prefix = if !n.is_dir && ed.dirty_files.contains(&n.path) {
                    "."
                } else {
                    ""
                };
                let name_display = format!(
                    "{} {}{} {}{}",
                    mark,
                    "  ".repeat(n.depth),
                    icon,
                    prefix,
                    n.name
                );
                let truncated: String = name_display.chars().take(TREE_WIDTH as usize).collect();
                write!(out, "{:<width$}", truncated, width = TREE_WIDTH as usize)?;
            }
        }

        let visible_tree_items =
            (ed.tree.len().saturating_sub(ed.tree_scroll)).min(max_lines as usize);
        if visible_tree_items < max_lines as usize {
            for y in visible_tree_items..max_lines as usize {
                execute!(out, cursor::MoveTo(0, y as u16))?;
                write!(out, "{:width$}", "", width = TREE_WIDTH as usize)?;
            }
        }

        ed.last_tree_scroll = ed.tree_scroll;
    }

    if ed.show_line_numbers {
        for screen_y in 0..max_lines {
            let buf_y = ed.scroll_y + screen_y as usize;
            if ed.buffer.get(buf_y).is_some() {
                execute!(out, cursor::MoveTo(tree_offset, screen_y))?;
                let line_num = buf_y + 1;
                let line_num_str = format!("{:>4} │", line_num);
                write!(out, "{}", line_num_str)?;
            }
        }
    }

    let available_width = (cols - text_offset) as usize;
    let keywords = get_keywords(&ed.language);
    for screen_y in 0..max_lines {
        let buf_y = ed.scroll_y + screen_y as usize;
        execute!(out, cursor::MoveTo(text_offset, screen_y))?;
        if let Some(line) = ed.buffer.get(buf_y) {
            let s: String = line.iter().collect();
            let line_len = s.chars().count();

            let start_char_idx = ed.scroll_x.min(line_len);
            let end_char_idx = (ed.scroll_x + available_width).min(line_len);

            if start_char_idx >= line_len {
                write!(out, "{:width$}", "", width = available_width)?;
            } else {
                let visible_part: String = s
                    .chars()
                    .skip(start_char_idx)
                    .take(end_char_idx - start_char_idx)
                    .collect();

                let tokens = if ed.language != Language::None {
                    tokenize_line(&s, &ed.language, &keywords)
                } else {
                    vec![(0, s.len(), TokenType::Normal)]
                };

                let is_search_mode =
                    !ed.search_results.is_empty() && matches!(ed.mode, EditorMode::Search);
                let query = if is_search_mode {
                    ed.search_query.iter().collect::<String>()
                } else {
                    String::new()
                };

                let (actual_start_y, actual_start_x, actual_end_y, actual_end_x) =
                    if let (Some((sel_start_y, sel_start_x)), Some((sel_end_y, sel_end_x))) =
                        (ed.selection_start, ed.selection_end)
                    {
                        if (sel_start_y, sel_start_x) < (sel_end_y, sel_end_x) {
                            (
                                Some(sel_start_y),
                                Some(sel_start_x),
                                Some(sel_end_y),
                                Some(sel_end_x),
                            )
                        } else {
                            (
                                Some(sel_end_y),
                                Some(sel_end_x),
                                Some(sel_start_y),
                                Some(sel_start_x),
                            )
                        }
                    } else {
                        (None, None, None, None)
                    };

                let is_char_selected = |char_idx: usize| -> bool {
                    if let (Some(start_y), Some(start_x), Some(end_y), Some(end_x)) =
                        (actual_start_y, actual_start_x, actual_end_y, actual_end_x)
                    {
                        if buf_y >= start_y && buf_y <= end_y {
                            if buf_y == start_y && buf_y == end_y {
                                char_idx >= start_x && char_idx < end_x
                            } else if buf_y == start_y {
                                char_idx >= start_x
                            } else if buf_y == end_y {
                                char_idx < end_x
                            } else {
                                true
                            }
                        } else {
                            false
                        }
                    } else {
                        false
                    }
                };

                if is_search_mode || ed.language != Language::None {
                    let mut char_idx = start_char_idx;
                    let mut token_idx = 0;
                    let mut output_len = 0;

                    while char_idx < end_char_idx && output_len < available_width {
                        while token_idx < tokens.len() && tokens[token_idx].1 <= char_idx {
                            token_idx += 1;
                        }

                        let token = if token_idx < tokens.len() {
                            tokens[token_idx]
                        } else {
                            (s.len(), s.len(), TokenType::Normal)
                        };

                        let search_match = if is_search_mode {
                            let search_str: String = s.chars().skip(char_idx).collect();
                            search_str.find(&query).map(|p| char_idx + p)
                        } else {
                            None
                        };

                        if let Some(match_start) = search_match {
                            if match_start < end_char_idx {
                                if match_start > char_idx {
                                    let token_type = if token.0 <= char_idx && token.1 > char_idx {
                                        token.2
                                    } else {
                                        TokenType::Normal
                                    };

                                    let mut current_idx = char_idx;
                                    while current_idx < match_start && output_len < available_width
                                    {
                                        let is_selected = is_char_selected(current_idx);
                                        let segment_end = {
                                            let mut end = current_idx + 1;
                                            while end < match_start
                                                && is_char_selected(end) == is_selected
                                            {
                                                end += 1;
                                            }
                                            end
                                        };

                                        execute!(
                                            out,
                                            SetForegroundColor(get_token_color(token_type))
                                        )?;
                                        if is_selected {
                                            execute!(out, SetAttribute(Attribute::Reverse))?;
                                        }

                                        let segment_text: String = s
                                            .chars()
                                            .skip(current_idx)
                                            .take(segment_end - current_idx)
                                            .collect();
                                        write!(out, "{}", segment_text)?;
                                        output_len += segment_text.chars().count();

                                        if is_selected {
                                            execute!(out, SetAttribute(Attribute::Reset))?;
                                        }
                                        execute!(out, SetForegroundColor(Color::White))?;

                                        current_idx = segment_end;
                                    }
                                }

                                let match_end =
                                    (match_start + query.chars().count()).min(end_char_idx);
                                let current_result = ed.search_results[ed.current_search_index];

                                let mut current_idx = match_start;
                                while current_idx < match_end && output_len < available_width {
                                    let is_selected = is_char_selected(current_idx);
                                    let segment_end = {
                                        let mut end = current_idx + 1;
                                        while end < match_end
                                            && is_char_selected(end) == is_selected
                                        {
                                            end += 1;
                                        }
                                        end
                                    };

                                    if current_result == (buf_y, match_start) {
                                        execute!(out, SetAttribute(Attribute::Reverse))?;
                                    } else {
                                        execute!(out, SetAttribute(Attribute::Bold))?;
                                    }
                                    execute!(out, SetForegroundColor(Color::White))?;

                                    if is_selected && current_result != (buf_y, match_start) {
                                        execute!(out, SetAttribute(Attribute::Reverse))?;
                                    }

                                    let segment_text: String = s
                                        .chars()
                                        .skip(current_idx)
                                        .take(segment_end - current_idx)
                                        .collect();
                                    write!(out, "{}", segment_text)?;
                                    output_len += segment_text.chars().count();

                                    execute!(out, SetAttribute(Attribute::Reset))?;
                                    current_idx = segment_end;
                                }
                                char_idx = match_end;
                            } else {
                                break;
                            }
                        } else {
                            let token_end = token.1.min(end_char_idx);
                            if token_end > char_idx {
                                let is_matched_bracket = if let Some((match_y, match_x)) =
                                    ed.matched_bracket
                                {
                                    (buf_y == match_y && char_idx <= match_x && match_x < token_end)
                                        || (buf_y == ed.cursor_y
                                            && char_idx <= ed.cursor_x
                                            && ed.cursor_x < token_end)
                                } else {
                                    false
                                };

                                let mut current_idx = char_idx;
                                while current_idx < token_end && output_len < available_width {
                                    let is_selected = is_char_selected(current_idx);
                                    let segment_end = {
                                        let mut end = current_idx + 1;
                                        while end < token_end
                                            && is_char_selected(end) == is_selected
                                        {
                                            end += 1;
                                        }
                                        end
                                    };

                                    let bracket_in_segment = is_matched_bracket
                                        && ((buf_y == ed.cursor_y
                                            && current_idx <= ed.cursor_x
                                            && ed.cursor_x < segment_end)
                                            || (if let Some((match_y, match_x)) = ed.matched_bracket
                                            {
                                                buf_y == match_y
                                                    && current_idx <= match_x
                                                    && match_x < segment_end
                                            } else {
                                                false
                                            }));

                                    if bracket_in_segment {
                                        execute!(out, SetForegroundColor(Color::Yellow))?;
                                        execute!(out, SetAttribute(Attribute::Bold))?;
                                    } else {
                                        execute!(
                                            out,
                                            SetForegroundColor(get_token_color(token.2))
                                        )?;
                                    }

                                    if is_selected {
                                        execute!(out, SetAttribute(Attribute::Reverse))?;
                                    }

                                    let segment_text: String = s
                                        .chars()
                                        .skip(current_idx)
                                        .take(segment_end - current_idx)
                                        .collect();
                                    write!(out, "{}", segment_text)?;
                                    output_len += segment_text.chars().count();

                                    if is_selected {
                                        execute!(out, SetAttribute(Attribute::Reset))?;
                                    }
                                    if bracket_in_segment {
                                        execute!(out, SetAttribute(Attribute::Reset))?;
                                    }
                                    execute!(out, SetForegroundColor(Color::White))?;

                                    current_idx = segment_end;
                                }
                                char_idx = token_end;
                                token_idx += 1;
                            } else {
                                break;
                            }
                        }
                    }

                    let remaining_width = available_width.saturating_sub(output_len);
                    if remaining_width > 0 {
                        write!(out, "{:width$}", "", width = remaining_width)?;
                    }
                } else {
                    if let (Some((sel_start_y, sel_start_x)), Some((sel_end_y, sel_end_x))) =
                        (ed.selection_start, ed.selection_end)
                    {
                        let (actual_start_y, actual_start_x, actual_end_y, actual_end_x) =
                            if (sel_start_y, sel_start_x) < (sel_end_y, sel_end_x) {
                                (sel_start_y, sel_start_x, sel_end_y, sel_end_x)
                            } else {
                                (sel_end_y, sel_end_x, sel_start_y, sel_start_x)
                            };

                        if buf_y >= actual_start_y && buf_y <= actual_end_y {
                            let mut char_idx = start_char_idx;
                            let mut output_len = 0;

                            while char_idx < end_char_idx && output_len < available_width {
                                let is_selected =
                                    if buf_y == actual_start_y && buf_y == actual_end_y {
                                        char_idx >= actual_start_x && char_idx < actual_end_x
                                    } else if buf_y == actual_start_y {
                                        char_idx >= actual_start_x
                                    } else if buf_y == actual_end_y {
                                        char_idx < actual_end_x
                                    } else {
                                        true
                                    };

                                let next_pos = if is_selected {
                                    if buf_y == actual_start_y && buf_y == actual_end_y {
                                        actual_end_x.min(end_char_idx)
                                    } else if buf_y == actual_start_y {
                                        end_char_idx
                                    } else if buf_y == actual_end_y {
                                        actual_end_x.min(end_char_idx)
                                    } else {
                                        end_char_idx
                                    }
                                } else {
                                    if buf_y == actual_start_y {
                                        actual_start_x.min(end_char_idx)
                                    } else if buf_y == actual_end_y {
                                        actual_end_x.min(end_char_idx)
                                    } else {
                                        end_char_idx
                                    }
                                };

                                if next_pos > char_idx {
                                    let is_matched_bracket =
                                        if let Some((match_y, match_x)) = ed.matched_bracket {
                                            (buf_y == match_y
                                                && char_idx <= match_x
                                                && match_x < next_pos)
                                                || (buf_y == ed.cursor_y
                                                    && char_idx <= ed.cursor_x
                                                    && ed.cursor_x < next_pos)
                                        } else {
                                            false
                                        };

                                    if is_selected {
                                        execute!(out, SetAttribute(Attribute::Reverse))?;
                                    }

                                    if is_matched_bracket {
                                        execute!(out, SetForegroundColor(Color::Yellow))?;
                                        execute!(out, SetAttribute(Attribute::Bold))?;
                                    }

                                    let text: String = s
                                        .chars()
                                        .skip(char_idx)
                                        .take(next_pos - char_idx)
                                        .collect();
                                    write!(out, "{}", text)?;

                                    if is_matched_bracket {
                                        execute!(out, SetAttribute(Attribute::Reset))?;
                                        execute!(out, SetForegroundColor(Color::White))?;
                                    }

                                    if is_selected {
                                        execute!(out, SetAttribute(Attribute::Reset))?;
                                    }
                                    output_len += text.chars().count();
                                    char_idx = next_pos;
                                } else {
                                    break;
                                }
                            }

                            let remaining_width = available_width.saturating_sub(output_len);
                            if remaining_width > 0 {
                                write!(out, "{:width$}", "", width = remaining_width)?;
                            }
                        } else {
                            write!(out, "{:<width$}", visible_part, width = available_width)?;
                        }
                    } else {
                        write!(out, "{:<width$}", visible_part, width = available_width)?;
                    }
                }
            }
        }
    }

    if matches!(ed.mode, EditorMode::Normal) || matches!(ed.mode, EditorMode::Autocomplete) {
        let cursor_screen_x = ed.cursor_x.saturating_sub(ed.scroll_x);
        let cursor_screen_y = ed.cursor_y.saturating_sub(ed.scroll_y);

        if cursor_screen_y < max_lines as usize {
            let available_width = (cols - text_offset) as usize;
            if cursor_screen_x < available_width {
                execute!(
                    out,
                    cursor::MoveTo(text_offset + cursor_screen_x as u16, cursor_screen_y as u16),
                    SetAttribute(Attribute::Reverse)
                )?;
                if let Some(line) = ed.buffer.get(ed.cursor_y) {
                    if ed.cursor_x < line.len() {
                        write!(out, "{}", line[ed.cursor_x])?;
                    } else {
                        write!(out, " ")?;
                    }
                } else {
                    write!(out, " ")?;
                }
                execute!(out, SetAttribute(Attribute::Reset))?;
            }
        }
    }

    if matches!(ed.mode, EditorMode::Autocomplete) && !ed.autocomplete_suggestions.is_empty() {
        let cursor_screen_x = ed.cursor_x.saturating_sub(ed.scroll_x);
        let cursor_screen_y = ed.cursor_y.saturating_sub(ed.scroll_y);

        let popup_x = text_offset + cursor_screen_x as u16;
        let popup_y = cursor_screen_y as u16 + 1;

        let max_suggestions = 8.min(ed.autocomplete_suggestions.len());
        let max_width = ed
            .autocomplete_suggestions
            .iter()
            .take(max_suggestions)
            .map(|s| s.len())
            .max()
            .unwrap_or(10)
            .max(10);

        for (i, suggestion) in ed
            .autocomplete_suggestions
            .iter()
            .take(max_suggestions)
            .enumerate()
        {
            let y = popup_y + i as u16;
            if y >= max_lines {
                break;
            }

            execute!(out, cursor::MoveTo(popup_x, y))?;

            if i == ed.autocomplete_index {
                execute!(out, crossterm::style::SetBackgroundColor(Color::Blue))?;
                execute!(out, SetForegroundColor(Color::White))?;
                execute!(out, SetAttribute(Attribute::Bold))?;
                write!(out, " {:<width$} ", suggestion, width = max_width)?;
                execute!(out, SetAttribute(Attribute::Reset))?;
                execute!(out, crossterm::style::SetBackgroundColor(Color::Reset))?;
            } else {
                execute!(out, crossterm::style::SetBackgroundColor(Color::DarkGrey))?;
                execute!(out, SetForegroundColor(Color::White))?;
                write!(out, " {:<width$} ", suggestion, width = max_width)?;
                execute!(out, crossterm::style::SetBackgroundColor(Color::Reset))?;
            }
        }
    }

    execute!(out, cursor::MoveTo(0, rows - 1))?;
    let status_text = match ed.mode {
        EditorMode::Dashboard => "Dashboard".to_string(),
        EditorMode::Search => {
            let query: String = ed.search_query.iter().collect();
            format!(
                "Search: {} | {} results found{}",
                query,
                ed.search_results.len(),
                if !ed.search_results.is_empty() {
                    format!(
                        " ({}/{})",
                        ed.current_search_index + 1,
                        ed.search_results.len()
                    )
                } else {
                    String::new()
                }
            )
        }
        EditorMode::CreateFile | EditorMode::CreateDir => {
            let name: String = ed.create_name.iter().collect();
            let prompt = if matches!(ed.mode, EditorMode::CreateFile) {
                "New file name"
            } else {
                "New folder name"
            };
            format!("{}: {}", prompt, name)
        }
        EditorMode::DeleteConfirm => ed.status.clone(),
        EditorMode::Rename => {
            let name: String = ed.rename_name.iter().collect();
            format!("Rename: {}", name)
        }
        EditorMode::GoToLine => {
            let line_input: String = ed.goto_line_input.iter().collect();
            format!("Go to line: {}", line_input)
        }
        EditorMode::Terminal => "Terminal mode".to_string(),
        EditorMode::Autocomplete => {
            format!(
                "Autocomplete: ↑↓ select | Tab/Enter confirm | Esc cancel | {}/{}",
                ed.autocomplete_index + 1,
                ed.autocomplete_suggestions.len()
            )
        }
        EditorMode::Normal => {
            format!(
                "[{}] Line:{} Col:{} | {}",
                ed.file_name.as_deref().unwrap_or("New"),
                ed.cursor_y + 1,
                ed.cursor_x + 1,
                ed.status
            )
        }
    };

    let status_text_truncated: String = status_text.chars().take(cols as usize).collect();
    write!(
        out,
        "{:<width$}",
        status_text_truncated,
        width = cols as usize
    )?;

    out.flush()?;

    ed.last_scroll_y = ed.scroll_y;
    ed.last_scroll_x = ed.scroll_x;
    ed.needs_full_redraw = false;

    Ok(())
}

fn main() -> io::Result<()> {
    let args: Vec<String> = env::args().collect();

    let initial_path = if args.len() > 1 { &args[1] } else { "." };

    terminal::enable_raw_mode()?;
    let mut out = io::stdout();
    execute!(
        out,
        terminal::EnterAlternateScreen,
        cursor::Hide,
        EnableMouseCapture
    )?;

    let mut ed = Editor::new_with_options(initial_path, args.len() == 1);

    loop {
        let (cols, rows) = terminal::size()?;
        ed.drain_terminal_output();

        if !ed.cursor_locked {
            ed.ensure_cursor_visible(rows, cols);
        }

        if ed.dirty || ed.needs_full_redraw {
            draw(&mut ed, &mut out)?;
            ed.dirty = false;
            ed.needs_full_redraw = false;
        }

        if event::poll(Duration::from_millis(100))? {
            match event::read()? {
                Event::Resize(_, _) => {
                    let (cols, rows) = terminal::size()?;
                    ed.resize_terminal_session(rows, cols);
                    ed.needs_full_redraw = true;
                }
                Event::Mouse(MouseEvent {
                    kind,
                    column,
                    row,
                    modifiers,
                    ..
                }) => {
                    let (cols, rows) = terminal::size()?;
                    if matches!(ed.mode, EditorMode::Terminal) {
                        match kind {
                            MouseEventKind::ScrollUp => {
                                ed.scroll_terminal(TERMINAL_SCROLL_STEP as isize);
                            }
                            MouseEventKind::ScrollDown => {
                                ed.scroll_terminal(-(TERMINAL_SCROLL_STEP as isize));
                            }
                            _ => {}
                        }
                    } else {
                        match kind {
                            MouseEventKind::Down(MouseButton::Left) => {
                                ed.handle_mouse_click(
                                    column,
                                    row,
                                    rows,
                                    cols,
                                    modifiers.contains(KeyModifiers::SHIFT),
                                );
                            }
                            MouseEventKind::Drag(MouseButton::Left) => {
                                ed.handle_mouse_drag(column, row, rows, cols);
                            }
                            MouseEventKind::Up(MouseButton::Left) => {
                                ed.handle_mouse_release();
                            }
                            MouseEventKind::ScrollUp => {
                                ed.handle_mouse_scroll(rows, true);
                            }
                            MouseEventKind::ScrollDown => {
                                ed.handle_mouse_scroll(rows, false);
                            }
                            _ => {}
                        }
                    }
                }
                Event::Key(KeyEvent {
                    code,
                    modifiers,
                    kind: KeyEventKind::Press,
                    ..
                }) => {
                    match ed.mode {
                        EditorMode::Dashboard => match (code, modifiers) {
                            (KeyCode::Char('q'), KeyModifiers::CONTROL) => {
                                break;
                            }
                            (KeyCode::Up, _) | (KeyCode::Char('k'), _) => {
                                if ed.dashboard_selection > 0 {
                                    ed.dashboard_selection -= 1;
                                    ed.dirty = true;
                                }
                            }
                            (KeyCode::Down, _) | (KeyCode::Char('j'), _) => {
                                let action_count = ed.dashboard_actions().len();
                                if ed.dashboard_selection + 1 < action_count {
                                    ed.dashboard_selection += 1;
                                    ed.dirty = true;
                                }
                            }
                            (KeyCode::Home, _) => {
                                ed.dashboard_selection = 0;
                                ed.dirty = true;
                            }
                            (KeyCode::End, _) => {
                                ed.dashboard_selection =
                                    ed.dashboard_actions().len().saturating_sub(1);
                                ed.dirty = true;
                            }
                            (KeyCode::Enter, _) => {
                                ed.activate_dashboard_selection();
                            }
                            (KeyCode::Esc, _) | (KeyCode::Char('o'), _) => {
                                ed.dashboard_selection = 0;
                                ed.activate_dashboard_selection();
                            }
                            (KeyCode::Char('n'), _) => {
                                ed.dashboard_selection = 1;
                                ed.activate_dashboard_selection();
                            }
                            (KeyCode::Char('r'), _) => {
                                ed.refresh_dashboard_recents();
                            }
                            (KeyCode::Char(c), _) if c.is_ascii_digit() => {
                                let index = c.to_digit(10).unwrap_or_default() as usize;
                                let action_count = ed.dashboard_actions().len();
                                if index > 0 && index <= action_count.min(9) {
                                    ed.dashboard_selection = index - 1;
                                    ed.activate_dashboard_selection();
                                }
                            }
                            _ => {}
                        },
                        EditorMode::Search => match (code, modifiers) {
                            (KeyCode::Esc, _) | (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                                ed.cancel_search();
                            }
                            (KeyCode::Enter, _) => {
                                ed.update_search();
                            }
                            (KeyCode::Backspace, _) => {
                                ed.search_query.pop();
                                ed.update_search();
                                ed.dirty = true;
                            }
                            (KeyCode::Tab, _) | (KeyCode::F(3), _) => {
                                ed.next_search_result();
                            }
                            (KeyCode::Char(c), m) if !m.contains(KeyModifiers::CONTROL) => {
                                ed.search_query.push(c);
                                ed.update_search();
                                ed.dirty = true;
                            }
                            _ => {}
                        },
                        EditorMode::CreateFile | EditorMode::CreateDir => match (code, modifiers) {
                            (KeyCode::Esc, _) | (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                                ed.cancel_create();
                            }
                            (KeyCode::Enter, _) => {
                                let _ = ed.create_file_or_dir();
                            }
                            (KeyCode::Backspace, _) => {
                                ed.create_name.pop();
                                ed.dirty = true;
                            }
                            (KeyCode::Char(c), m) if !m.contains(KeyModifiers::CONTROL) => {
                                ed.create_name.push(c);
                                ed.dirty = true;
                            }
                            _ => {}
                        },
                        EditorMode::DeleteConfirm => match (code, modifiers) {
                            (KeyCode::Char('y') | KeyCode::Char('Y'), _) => {
                                let _ = ed.confirm_delete();
                            }
                            (KeyCode::Char('n') | KeyCode::Char('N'), _) | (KeyCode::Esc, _) => {
                                ed.cancel_delete();
                            }
                            _ => {}
                        },
                        EditorMode::Rename => match (code, modifiers) {
                            (KeyCode::Esc, _) | (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                                ed.cancel_rename();
                            }
                            (KeyCode::Enter, _) => {
                                let _ = ed.confirm_rename();
                            }
                            (KeyCode::Backspace, _) => {
                                ed.rename_name.pop();
                                ed.dirty = true;
                            }
                            (KeyCode::Char(c), m) if !m.contains(KeyModifiers::CONTROL) => {
                                ed.rename_name.push(c);
                                ed.dirty = true;
                            }
                            _ => {}
                        },
                        EditorMode::GoToLine => match (code, modifiers) {
                            (KeyCode::Esc, _) | (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                                ed.cancel_goto_line();
                            }
                            (KeyCode::Enter, _) => {
                                ed.confirm_goto_line();
                            }
                            (KeyCode::Backspace, _) => {
                                ed.goto_line_input.pop();
                                ed.dirty = true;
                            }
                            (KeyCode::Char(c), m)
                                if !m.contains(KeyModifiers::CONTROL) && c.is_ascii_digit() =>
                            {
                                ed.goto_line_input.push(c);
                                ed.dirty = true;
                            }
                            _ => {}
                        },
                        EditorMode::Terminal => {
                            if matches!(
                                (code, modifiers),
                                (KeyCode::Char('c'), KeyModifiers::CONTROL)
                            ) {
                                ed.interrupt_terminal();
                            } else if matches!((code, modifiers), (KeyCode::Enter, _)) {
                                ed.execute_terminal_command();
                            } else {
                                ed.handle_terminal_key_event(code, modifiers);
                            }
                        }
                        EditorMode::Autocomplete => match (code, modifiers) {
                            (KeyCode::Esc, _) | (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                                ed.cancel_autocomplete();
                            }
                            (KeyCode::Enter, _) | (KeyCode::Tab, _) => {
                                ed.apply_autocomplete();
                            }
                            (KeyCode::Down, _) => {
                                ed.next_autocomplete();
                            }
                            (KeyCode::Up, _) => {
                                ed.prev_autocomplete();
                            }
                            (KeyCode::Char(c), m) if !m.contains(KeyModifiers::CONTROL) => {
                                ed.cancel_autocomplete();
                                ed.insert(c);
                                ed.start_autocomplete();
                            }
                            (KeyCode::Backspace, _) => {
                                ed.cancel_autocomplete();
                                ed.backspace();
                                ed.start_autocomplete();
                            }
                            _ => {
                                ed.cancel_autocomplete();
                            }
                        },
                        EditorMode::Normal => {
                            if ed.quit_confirm
                                && !matches!(
                                    (code, modifiers),
                                    (KeyCode::Char('q'), KeyModifiers::CONTROL)
                                )
                            {
                                ed.quit_confirm = false;
                                ed.needs_full_redraw = true;
                                ed.status = "Ctrl+O Tree | Ctrl+N File | Ctrl+M Folder | F2 Rename | Del Delete | Ctrl+S Save | Ctrl+F Find | Ctrl+G Go to Line | Shift+Arrow Select | Ctrl+C Copy | Ctrl+V Paste | Ctrl+Arrow Word | Ctrl+1 Terminal | Ctrl+Q Quit".into();
                            }
                            match (code, modifiers) {
                                (KeyCode::Char('q'), KeyModifiers::CONTROL) => {
                                    if ed.quit_confirm {
                                        break;
                                    } else if ed.dirty {
                                        ed.quit_confirm = true;
                                        ed.status = "File not saved! Press Ctrl+Q again to quit, any other key to cancel".into();
                                        ed.needs_full_redraw = true;
                                    } else {
                                        break;
                                    }
                                }
                                (KeyCode::Char('s'), KeyModifiers::CONTROL) => {
                                    let _ = ed.save();
                                }
                                (KeyCode::Char('o'), KeyModifiers::CONTROL) => {
                                    ed.show_tree = !ed.show_tree;
                                    ed.needs_full_redraw = true;
                                    ed.dirty = true;
                                }
                                (KeyCode::Char('f'), KeyModifiers::CONTROL) => {
                                    ed.start_search();
                                }
                                (KeyCode::Char('g'), KeyModifiers::CONTROL) => {
                                    ed.start_goto_line();
                                }
                                (KeyCode::Char('z'), KeyModifiers::CONTROL) => {
                                    ed.undo();
                                }
                                (KeyCode::Char('y'), KeyModifiers::CONTROL) => {
                                    ed.redo();
                                }
                                (KeyCode::Char('1'), KeyModifiers::CONTROL) => {
                                    ed.toggle_terminal();
                                }
                                (KeyCode::Char('a'), KeyModifiers::CONTROL) => {
                                    ed.select_all();
                                }
                                (KeyCode::Char(' '), KeyModifiers::CONTROL) => {
                                    ed.start_autocomplete();
                                }
                                (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                                    ed.copy_selection();
                                    ed.is_selecting = false;
                                }
                                (KeyCode::Char('v'), KeyModifiers::CONTROL) => {
                                    ed.paste();
                                }
                                (KeyCode::Char('n'), m)
                                    if ed.show_tree
                                        && m.contains(KeyModifiers::CONTROL)
                                        && !m.contains(KeyModifiers::SHIFT) =>
                                {
                                    ed.start_create_file();
                                }
                                (KeyCode::Char('m'), m)
                                    if ed.show_tree
                                        && m.contains(KeyModifiers::CONTROL)
                                        && !m.contains(KeyModifiers::SHIFT)
                                        && !m.contains(KeyModifiers::ALT) =>
                                {
                                    ed.start_create_dir();
                                }
                                (KeyCode::Char('r'), KeyModifiers::CONTROL)
                                | (KeyCode::F(2), _)
                                    if ed.show_tree =>
                                {
                                    ed.start_rename();
                                }
                                (KeyCode::Delete, _) | (KeyCode::F(8), _) if ed.show_tree => {
                                    ed.start_delete();
                                }

                                (KeyCode::Up, m)
                                    if ed.show_tree && !m.contains(KeyModifiers::SHIFT) =>
                                {
                                    if ed.tree_cursor > 0 {
                                        ed.tree_cursor -= 1;
                                        let (_, rows) = terminal::size().unwrap_or((80, 24));
                                        let max_tree_lines = (rows - STATUS_HEIGHT) as usize;
                                        if ed.tree_cursor < ed.tree_scroll {
                                            ed.tree_scroll = ed.tree_cursor;
                                        }
                                        ed.dirty = true;
                                    }
                                }
                                (KeyCode::Down, m)
                                    if ed.show_tree && !m.contains(KeyModifiers::SHIFT) =>
                                {
                                    if ed.tree_cursor + 1 < ed.tree.len() {
                                        ed.tree_cursor += 1;
                                        let (_, rows) = terminal::size().unwrap_or((80, 24));
                                        let max_tree_lines = (rows - STATUS_HEIGHT) as usize;
                                        if ed.tree_cursor >= ed.tree_scroll + max_tree_lines {
                                            ed.tree_scroll = ed.tree_cursor - max_tree_lines + 1;
                                        }
                                        ed.dirty = true;
                                    }
                                }
                                (KeyCode::Enter, _) if ed.show_tree => {
                                    let n = ed.tree[ed.tree_cursor].clone();
                                    if n.is_dir {
                                        ed.toggle_dir(ed.tree_cursor);
                                    } else {
                                        let _ = ed.open_file(&n.path);
                                    }
                                    ed.dirty = true;
                                }

                                (KeyCode::Left, m) => {
                                    if m.contains(KeyModifiers::CONTROL)
                                        && m.contains(KeyModifiers::SHIFT)
                                    {
                                        if !ed.is_selecting {
                                            ed.start_selection();
                                        }
                                        ed.word_left();
                                        ed.update_selection_end();
                                    } else if m.contains(KeyModifiers::CONTROL) {
                                        if ed.is_selecting {
                                            ed.is_selecting = false;
                                            ed.selection_start = None;
                                            ed.selection_end = None;
                                        }
                                        ed.word_left();
                                    } else if m.contains(KeyModifiers::SHIFT) {
                                        if !ed.is_selecting {
                                            ed.start_selection();
                                        }
                                        ed.left();
                                    } else {
                                        if ed.is_selecting {
                                            ed.is_selecting = false;
                                            ed.selection_start = None;
                                            ed.selection_end = None;
                                        }
                                        ed.left();
                                    }
                                }
                                (KeyCode::Right, m) => {
                                    if m.contains(KeyModifiers::CONTROL)
                                        && m.contains(KeyModifiers::SHIFT)
                                    {
                                        if !ed.is_selecting {
                                            ed.start_selection();
                                        }
                                        ed.word_right();
                                        ed.update_selection_end();
                                    } else if m.contains(KeyModifiers::CONTROL) {
                                        if ed.is_selecting {
                                            ed.is_selecting = false;
                                            ed.selection_start = None;
                                            ed.selection_end = None;
                                        }
                                        ed.word_right();
                                    } else if m.contains(KeyModifiers::SHIFT) {
                                        if !ed.is_selecting {
                                            ed.start_selection();
                                        }
                                        ed.right();
                                    } else {
                                        if ed.is_selecting {
                                            ed.is_selecting = false;
                                            ed.selection_start = None;
                                            ed.selection_end = None;
                                        }
                                        ed.right();
                                    }
                                }
                                (KeyCode::Up, m) => {
                                    if m.contains(KeyModifiers::SHIFT) {
                                        if !ed.is_selecting {
                                            ed.start_selection();
                                        }
                                        ed.up();
                                    } else {
                                        if ed.is_selecting {
                                            ed.is_selecting = false;
                                            ed.selection_start = None;
                                            ed.selection_end = None;
                                        }
                                        ed.up();
                                    }
                                }
                                (KeyCode::Down, m) => {
                                    if m.contains(KeyModifiers::SHIFT) {
                                        if !ed.is_selecting {
                                            ed.start_selection();
                                        }
                                        ed.down();
                                    } else {
                                        if ed.is_selecting {
                                            ed.is_selecting = false;
                                            ed.selection_start = None;
                                            ed.selection_end = None;
                                        }
                                        ed.down();
                                    }
                                }

                                (KeyCode::Backspace, m) => {
                                    if ed.is_selecting {
                                        ed.is_selecting = false;
                                        ed.selection_start = None;
                                        ed.selection_end = None;
                                    }
                                    if m.contains(KeyModifiers::CONTROL) {
                                        ed.delete_word_backward();
                                    } else {
                                        ed.backspace();
                                    }
                                }
                                (KeyCode::Delete, m) => {
                                    if ed.is_selecting {
                                        ed.is_selecting = false;
                                        ed.selection_start = None;
                                        ed.selection_end = None;
                                    }
                                    if m.contains(KeyModifiers::CONTROL) {
                                        ed.delete_word_forward();
                                    } else {
                                        ed.delete();
                                    }
                                }
                                (KeyCode::Enter, _) => {
                                    if ed.is_selecting {
                                        ed.is_selecting = false;
                                        ed.selection_start = None;
                                        ed.selection_end = None;
                                    }
                                    ed.newline();
                                }
                                (KeyCode::Tab, m) => {
                                    if m.contains(KeyModifiers::SHIFT) {
                                        ed.unindent();
                                    } else {
                                        ed.indent();
                                    }
                                }
                                (KeyCode::Char(c), m) => {
                                    // Tüm karakterleri kabul et, sadece tek başına CONTROL kombinasyonlarını reddet
                                    // ALT/ALTGR (ALT+CTRL) tuşlarıyla yazılan karakterleri de kabul et (#, |, @, vb.)
                                    if !m.contains(KeyModifiers::CONTROL)
                                        || m.contains(KeyModifiers::ALT)
                                    {
                                        if ed.is_selecting {
                                            ed.is_selecting = false;
                                            ed.selection_start = None;
                                            ed.selection_end = None;
                                        }
                                        ed.insert(c);
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }

    ed.close_terminal_session();
    ed.close_discord();
    execute!(
        out,
        DisableMouseCapture,
        terminal::LeaveAlternateScreen,
        cursor::Show
    )?;
    terminal::disable_raw_mode()?;
    Ok(())
}
