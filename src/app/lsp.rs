//! What the editor does with language servers: which buffers they hear
//! about, when, and what becomes of their answers.
//!
//! Servers are started lazily, one per language, the first time a file in
//! that language opens — and only if the program is installed, so a machine
//! without `rust-analyzer` simply edits Rust without it. Text is kept in
//! step by sending each buffer's whole text after it changes, which every
//! server understands and costs nothing next to what the server does with it.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::mode::Mode;
use super::state::App;
use crate::editor::buffer::BufferId;
use crate::editor::cursor::Position;
use crate::editor::edit::Edit;
use crate::editor::selection::Range;
use crate::editor::window::Window;
use crate::lsp::protocol::{self, CompletionItem, Diagnostic, Encoding, TextEdit};
use crate::lsp::{Client, Event, RequestKind, ServerConfig};
use crate::picker::{Picker, PickerItem, PickerTarget};

/// Every server, and what they have said.
#[derive(Debug, Default)]
pub struct LspState {
    /// Running servers, by language name.
    pub clients: HashMap<String, Client>,
    /// Languages whose server could not be started, and why.
    pub unavailable: HashMap<String, String>,
    /// The latest diagnostics for each file.
    pub diagnostics: HashMap<PathBuf, Vec<Diagnostic>>,
    /// The revision of each file the server last saw.
    synced: HashMap<PathBuf, u64>,
    /// The revision each file was at when the server last heard it saved.
    saved: HashMap<PathBuf, u64>,
    /// The completion menu, while one is open.
    pub menu: Option<CompletionMenu>,
}

/// Completions on offer at the caret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletionMenu {
    /// Everything the server offered.
    pub items: Vec<CompletionItem>,
    /// The ones that still match what has been typed, by index.
    pub shown: Vec<usize>,
    /// The highlighted one, as an index into `shown`.
    pub selected: usize,
    /// The line being completed.
    pub line: usize,
    /// Where the word being completed starts.
    pub start: usize,
}

impl CompletionMenu {
    /// The item that Tab or Enter would insert.
    #[must_use]
    pub fn current(&self) -> Option<&CompletionItem> {
        self.shown
            .get(self.selected)
            .and_then(|index| self.items.get(*index))
    }
}

/// What a key does to an open completion menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuAction {
    /// Highlight the next item.
    Next,
    /// Highlight the previous item.
    Previous,
    /// Insert the highlighted item.
    Accept,
    /// Close the menu, inserting nothing.
    Close,
}

/// The server a language should use: the config's, or the usual one.
fn server_for(app: &App, language: &str) -> Option<ServerConfig> {
    app.config
        .languages
        .get(language)
        .and_then(|settings| settings.language_server.clone())
        .or_else(|| crate::lsp::default_server(language))
}

/// Tell the right server about a buffer, starting the server if needed.
pub fn attach(app: &mut App, index: BufferId) {
    if !app.config.lsp {
        return;
    }
    let Some(buffer) = app.buffers.get(index) else {
        return;
    };
    let Some(path) = buffer.document.path().map(Path::to_path_buf) else {
        return;
    };
    if buffer.large {
        return;
    }
    let language = buffer.syntax.language_name().to_string();
    if language == "plain" {
        return;
    }
    if !app.lsp.clients.contains_key(&language) {
        if app.ephemeral || app.lsp.unavailable.contains_key(&language) {
            return;
        }
        let Some(config) = server_for(app, &language) else {
            return;
        };
        let root = crate::git::repository(&path)
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(|| PathBuf::from("."));
        match Client::spawn(&config, &root) {
            Ok(client) => {
                app.lsp.clients.insert(language.clone(), client);
            }
            Err(error) => {
                app.lsp.unavailable.insert(language, error.to_string());
                return;
            }
        }
    }
    let text = app.buffers[index].document.text().to_string();
    let revision = app.buffers[index].document.revision();
    if let Some(client) = app.lsp.clients.get_mut(&language)
        && !client.is_open(&path)
    {
        client.did_open(&path, &language, &text);
        app.lsp.synced.insert(path.clone(), revision);
        app.lsp.saved.insert(path, revision);
    }
}

/// Tell a server a buffer has closed.
pub fn detach(app: &mut App, index: BufferId) {
    let Some(buffer) = app.buffers.get(index) else {
        return;
    };
    let Some(path) = buffer.document.path().map(Path::to_path_buf) else {
        return;
    };
    let language = buffer.syntax.language_name().to_string();
    if let Some(client) = app.lsp.clients.get_mut(&language) {
        client.did_close(&path);
    }
    app.lsp.diagnostics.remove(&path);
    app.lsp.synced.remove(&path);
    app.lsp.saved.remove(&path);
}

/// Send every changed buffer's text, and every save, to its server.
pub fn sync(app: &mut App) {
    let App { buffers, lsp, .. } = app;
    for buffer in buffers.iter() {
        let Some(path) = buffer.document.path() else {
            continue;
        };
        let Some(client) = lsp.clients.get_mut(buffer.syntax.language_name()) else {
            continue;
        };
        if !client.is_open(path) {
            continue;
        }
        let revision = buffer.document.revision();
        if lsp.synced.get(path) != Some(&revision) {
            client.did_change(path, &buffer.document.text().to_string());
            lsp.synced.insert(path.to_path_buf(), revision);
        }
        if !buffer.document.is_dirty() && lsp.saved.get(path) != Some(&revision) {
            client.did_save(path);
            lsp.saved.insert(path.to_path_buf(), revision);
        }
    }
}

/// Act on everything the servers said since the last call.
pub fn poll(app: &mut App) {
    let mut events = Vec::new();
    for (language, client) in &mut app.lsp.clients {
        let encoding = client.encoding;
        events.extend(
            client
                .poll()
                .into_iter()
                .map(|event| (language.clone(), encoding, event)),
        );
    }
    let gone: Vec<String> = app
        .lsp
        .clients
        .iter()
        .filter(|(_, client)| client.is_closed())
        .map(|(language, _)| language.clone())
        .collect();
    for language in gone {
        app.lsp.clients.remove(&language);
        app.lsp
            .unavailable
            .insert(language.clone(), "the server stopped".to_string());
        app.error(format!("the {language} language server stopped"));
    }
    for (language, encoding, event) in events {
        handle(app, &language, encoding, event);
    }
}

fn handle(app: &mut App, language: &str, encoding: Encoding, event: Event) {
    match event {
        Event::Diagnostics { path, items } => {
            let document = app
                .index_of(&path)
                .map(|index| &app.buffers[index].document);
            let diagnostics = protocol::read_diagnostics(&items, document, encoding);
            app.lsp.diagnostics.insert(path, diagnostics);
        }
        Event::Message(text) => app.info(text),
        Event::Failed { kind, message } => {
            if kind != RequestKind::Completion {
                app.error(format!("{language} server: {message}"));
            }
        }
        Event::Response { kind, path, result } => match kind {
            RequestKind::Hover => match protocol::hover_text(&result) {
                Some(text) => app.show_popup("hover", text),
                None => app.info("nothing to show here"),
            },
            RequestKind::Definition => show_locations(app, &result, encoding, "definitions"),
            RequestKind::References => show_locations(app, &result, encoding, "references"),
            RequestKind::Completion => open_menu(app, &path, &result, encoding),
            RequestKind::Rename => apply_workspace_edit(app, &result, encoding),
            RequestKind::Formatting => {
                let edits = protocol::read_edits(&result);
                if let Some(index) = app.index_of(&path) {
                    let count = edits.len();
                    apply_edits(app, index, &edits, encoding);
                    app.info(if count == 0 {
                        "already formatted".to_string()
                    } else {
                        "formatted".to_string()
                    });
                }
            }
            RequestKind::Initialize => {}
        },
    }
}

/// The server and position for a request at the caret.
fn here(app: &mut App, capability: &str) -> Option<(String, PathBuf, Value)> {
    let path = app.buffer().document.path()?.to_path_buf();
    let language = app.buffer().syntax.language_name().to_string();
    let Some(client) = app.lsp.clients.get(&language) else {
        let reason = app
            .lsp
            .unavailable
            .get(&language)
            .cloned()
            .unwrap_or_else(|| format!("no language server for {language}"));
        app.error(reason);
        return None;
    };
    if !client.supports(capability) {
        app.error(format!("{} does not offer that", client.name));
        return None;
    }
    let head = app.window().cursor().head;
    let position = protocol::position(&app.buffer().document, head, client.encoding);
    let params = json!({
        "textDocument": { "uri": protocol::uri(&path) },
        "position": position,
    });
    Some((language, path, params))
}

/// Ask the server about the caret: `K`, `gd`, `gr`.
pub fn request(app: &mut App, kind: RequestKind) {
    let capability = match kind {
        RequestKind::Hover => "hoverProvider",
        RequestKind::Definition => "definitionProvider",
        RequestKind::References => "referencesProvider",
        RequestKind::Completion => "completionProvider",
        _ => return,
    };
    let Some((language, path, mut params)) = here(app, capability) else {
        return;
    };
    if kind == RequestKind::References {
        params["context"] = json!({ "includeDeclaration": true });
    }
    if let Some(client) = app.lsp.clients.get_mut(&language) {
        client.request(kind, &path, params);
    }
}

/// `:rename <name>` — rename the symbol under the caret everywhere.
pub fn rename(app: &mut App, name: &str) {
    let Some((language, path, mut params)) = here(app, "renameProvider") else {
        return;
    };
    params["newName"] = json!(name);
    if let Some(client) = app.lsp.clients.get_mut(&language) {
        client.request(RequestKind::Rename, &path, params);
    }
}

/// `:format` — ask the server to format the file, or run the configured
/// formatter when there is no server that can.
pub fn format(app: &mut App) {
    let language = app.buffer().syntax.language_name().to_string();
    let formatter = app
        .config
        .languages
        .get(&language)
        .and_then(|settings| settings.formatter.clone());
    let server_formats = app
        .lsp
        .clients
        .get(&language)
        .is_some_and(|client| client.supports("documentFormattingProvider"));
    if !server_formats {
        match formatter {
            Some(command) => return format_with(app, &command),
            None => {
                app.error(format!(
                    "nothing formats {language} — set languages.{language}.formatter"
                ));
                return;
            }
        }
    }
    let Some(path) = app.buffer().document.path().map(Path::to_path_buf) else {
        return;
    };
    let config = app.buffer_config(app.window().buffer);
    let params = json!({
        "textDocument": { "uri": protocol::uri(&path) },
        "options": { "tabSize": config.tab_width, "insertSpaces": config.expand_tabs },
    });
    if let Some(client) = app.lsp.clients.get_mut(&language) {
        client.request(RequestKind::Formatting, &path, params);
    }
}

/// Pipe the buffer through a formatter command and take what comes back.
fn format_with(app: &mut App, command: &str) {
    use std::io::Write;
    use std::process::{Command, Stdio};

    let (shell, flag) = if cfg!(windows) {
        ("cmd", "/C")
    } else {
        ("sh", "-c")
    };
    let text = app.buffer().document.text().to_string();
    let output = Command::new(shell)
        .args([flag, command])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            if let Some(mut stdin) = child.stdin.take() {
                stdin.write_all(text.as_bytes())?;
            }
            child.wait_with_output()
        });
    match output {
        Ok(output) if output.status.success() => {
            let formatted = String::from_utf8_lossy(&output.stdout).into_owned();
            if formatted == text {
                app.info("already formatted");
                return;
            }
            let caret = app.window().cursor().head;
            let length = app.buffer().document.len_chars();
            let mut edit = app.edit();
            edit.begin_group();
            edit.replace(0, length, &formatted);
            edit.end_group();
            let position = app.buffer().document.clamp(caret, false);
            app.window_mut().cursor_mut().move_to(position, false);
            app.info("formatted");
        }
        Ok(output) => {
            let error = String::from_utf8_lossy(&output.stderr);
            app.error(format!(
                "{command}: {}",
                error.lines().next().unwrap_or("failed")
            ));
        }
        Err(error) => app.error(format!("cannot run {command}: {error}")),
    }
}

/// Go to one location, or offer a list of several.
fn show_locations(app: &mut App, result: &Value, encoding: Encoding, title: &str) {
    let locations = protocol::read_locations(result);
    let resolve = |app: &App, location: &protocol::Location| {
        let document = app
            .index_of(&location.path)
            .map(|index| &app.buffers[index].document);
        protocol::read_position(&location.start, document, encoding).unwrap_or_default()
    };
    match locations.as_slice() {
        [] => app.info(format!("no {title} found")),
        [only] => {
            let position = resolve(app, only);
            app.window_mut().record_jump();
            match app.open(only.path.clone()) {
                Ok(()) => {
                    let position = app.buffer().document.clamp(position, false);
                    app.window_mut().clear_secondary_cursors();
                    app.window_mut().cursor_mut().move_to(position, false);
                }
                Err(error) => app.error(error.to_string()),
            }
        }
        many => {
            let root = std::env::current_dir().unwrap_or_default();
            let items = many
                .iter()
                .map(|location| {
                    let position = resolve(app, location);
                    let name = location
                        .path
                        .strip_prefix(&root)
                        .unwrap_or(&location.path)
                        .display()
                        .to_string();
                    let line = std::fs::read_to_string(&location.path)
                        .ok()
                        .and_then(|text| {
                            text.lines()
                                .nth(position.line)
                                .map(|l| l.trim().to_string())
                        })
                        .unwrap_or_default();
                    PickerItem::new(
                        format!("{name}:{}: {line}", position.line + 1),
                        PickerTarget::Location(location.path.clone(), position),
                    )
                })
                .collect();
            app.picker = Some(Picker::new(title, items));
            app.mode = Mode::Picker;
        }
    }
}

/// Apply edits to one buffer as a single undo step.
fn apply_edits(app: &mut App, index: BufferId, edits: &[TextEdit], encoding: Encoding) {
    let document = &app.buffers[index].document;
    let mut replacements: Vec<(Range, String)> = edits
        .iter()
        .filter_map(|edit| {
            let (start, end) = protocol::read_range(&edit.range, Some(document), encoding)?;
            Some((
                Range::new(document.pos_to_char(start), document.pos_to_char(end)),
                edit.text.clone(),
            ))
        })
        .collect();
    if replacements.is_empty() {
        return;
    }
    replacements.sort_by_key(|(range, _)| range.start);
    let focused = app.window().buffer == index;
    if focused {
        let mut edit = app.edit();
        edit.begin_group();
        edit.replace_many(&replacements);
        edit.end_group();
    } else {
        let mut window = Window::new(index);
        let mut edit = Edit::new(&mut app.buffers[index], &mut window);
        edit.begin_group();
        edit.replace_many(&replacements);
        edit.end_group();
    }
    app.clamp_windows_on(index);
}

/// Apply a rename's edits across the files it touches, opening them as
/// needed and leaving them unsaved for review.
fn apply_workspace_edit(app: &mut App, result: &Value, encoding: Encoding) {
    let files = protocol::read_workspace_edit(result);
    if files.is_empty() {
        app.info("nothing to rename");
        return;
    }
    let mut changed = 0;
    for (path, edits) in files {
        match app.load(path) {
            Ok(index) => {
                apply_edits(app, index, &edits, encoding);
                changed += 1;
            }
            Err(error) => {
                app.error(error.to_string());
                return;
            }
        }
    }
    app.info(format!(
        "renamed in {changed} file{} — :wa writes them",
        if changed == 1 { "" } else { "s" }
    ));
}

/// Where the word ending at the caret starts.
fn word_start(app: &App) -> usize {
    let head = app.window().cursor().head;
    let line: Vec<char> = app
        .buffer()
        .document
        .line_string(head.line)
        .chars()
        .collect();
    let mut start = head.col.min(line.len());
    while start > 0 && (line[start - 1].is_alphanumeric() || line[start - 1] == '_') {
        start -= 1;
    }
    start
}

fn open_menu(app: &mut App, path: &Path, result: &Value, encoding: Encoding) {
    if !app.mode.is_insert() || app.buffer().document.path() != Some(path) {
        return;
    }
    let items = protocol::read_completions(result);
    if items.is_empty() {
        app.lsp.menu = None;
        return;
    }
    let head = app.window().cursor().head;
    let start = items
        .iter()
        .find_map(|item| item.replace_from.as_ref())
        .and_then(|from| protocol::read_position(from, Some(&app.buffer().document), encoding))
        .filter(|from| from.line == head.line && from.col <= head.col)
        .map_or_else(|| word_start(app), |from| from.col);
    app.lsp.menu = Some(CompletionMenu {
        items,
        shown: Vec::new(),
        selected: 0,
        line: head.line,
        start,
    });
    filter_menu(app);
}

/// Narrow the menu to what still matches the word typed so far, closing it
/// when nothing does or the caret has left the word.
pub fn filter_menu(app: &mut App) {
    let head = app.window().cursor().head;
    let line = app.buffer().document.line_string(head.line);
    let Some(menu) = app.lsp.menu.as_mut() else {
        return;
    };
    if head.line != menu.line || head.col < menu.start {
        app.lsp.menu = None;
        return;
    }
    let typed: String = line
        .chars()
        .skip(menu.start)
        .take(head.col - menu.start)
        .collect::<String>()
        .to_lowercase();
    let mut shown: Vec<(usize, usize)> = menu
        .items
        .iter()
        .enumerate()
        .filter_map(|(index, item)| {
            let filter = item.filter.to_lowercase();
            if filter.starts_with(&typed) {
                Some((0, index))
            } else if filter.contains(&typed) {
                Some((1, index))
            } else {
                None
            }
        })
        .collect();
    shown.sort_unstable();
    menu.shown = shown.into_iter().map(|(_, index)| index).collect();
    menu.selected = menu.selected.min(menu.shown.len().saturating_sub(1));
    // A menu offering only exactly what was typed has nothing left to offer.
    let exhausted = menu.shown.len() == 1
        && menu
            .current()
            .is_some_and(|item| item.insert.to_lowercase() == typed);
    if menu.shown.is_empty() || exhausted {
        app.lsp.menu = None;
    }
}

/// Up, Down, Tab, Enter and Esc while the menu is open.
pub fn menu_action(app: &mut App, action: MenuAction) {
    let Some(menu) = app.lsp.menu.as_mut() else {
        return;
    };
    let count = menu.shown.len().max(1);
    match action {
        MenuAction::Next => menu.selected = (menu.selected + 1) % count,
        MenuAction::Previous => menu.selected = (menu.selected + count - 1) % count,
        MenuAction::Close => app.lsp.menu = None,
        MenuAction::Accept => {
            let Some(menu) = app.lsp.menu.take() else {
                return;
            };
            let Some(item) = menu.current().cloned() else {
                return;
            };
            let head = app.window().cursor().head;
            let document = &app.buffer().document;
            let start = document.pos_to_char(Position::new(menu.line, menu.start));
            let end = document.pos_to_char(head);
            let mut edit = app.edit();
            edit.replace(start, end, &item.insert);
            edit.checkpoint();
            let landed = app
                .buffer()
                .document
                .char_to_pos(start + item.insert.chars().count());
            app.window_mut().clear_secondary_cursors();
            app.window_mut().cursor_mut().move_to(landed, false);
        }
    }
}

/// After a character is typed in insert mode: keep an open menu in step, or
/// ask for completions when the typing suggests some are wanted.
pub fn after_typing(app: &mut App, typed: Option<char>) {
    if app.lsp.menu.is_some() {
        filter_menu(app);
        if app.lsp.menu.is_some() {
            return;
        }
    }
    if !app.config.auto_complete || !app.mode.is_insert() {
        return;
    }
    let Some(ch) = typed else {
        return;
    };
    let language = app.buffer().syntax.language_name().to_string();
    let Some(client) = app.lsp.clients.get(&language) else {
        return;
    };
    if !client.supports("completionProvider") || client.is_waiting(RequestKind::Completion) {
        return;
    }
    let head = app.window().cursor().head;
    let word = head.col - word_start(app);
    let trigger = client.trigger_characters().contains(&ch);
    if trigger || ((ch.is_alphanumeric() || ch == '_') && word >= 2) {
        request(app, RequestKind::Completion);
    }
}

/// The diagnostics for the focused buffer.
#[must_use]
pub fn diagnostics_here(app: &App) -> &[Diagnostic] {
    app.buffer()
        .document
        .path()
        .and_then(|path| app.lsp.diagnostics.get(path))
        .map_or(&[], Vec::as_slice)
}

/// `]d` and `[d` — the next or previous diagnostic.
pub fn jump_diagnostic(app: &mut App, forward: bool) {
    let head = app.window().cursor().head;
    let found = {
        let list = diagnostics_here(app);
        if forward {
            list.iter().find(|diagnostic| diagnostic.start > head)
        } else {
            list.iter().rev().find(|diagnostic| diagnostic.start < head)
        }
        .map(|diagnostic| (diagnostic.start, diagnostic.message.clone()))
    };
    match found {
        Some((position, message)) => {
            app.window_mut().record_jump();
            let position = app.buffer().document.clamp(position, false);
            app.window_mut().clear_secondary_cursors();
            app.window_mut().cursor_mut().move_to(position, false);
            app.info(message);
        }
        None => app.info("no more diagnostics"),
    }
}

/// `:diagnostics` — every diagnostic in the file, in a picker.
pub fn list_diagnostics(app: &mut App) {
    let Some(path) = app.buffer().document.path().map(Path::to_path_buf) else {
        return;
    };
    let items: Vec<PickerItem> = diagnostics_here(app)
        .iter()
        .map(|diagnostic| {
            PickerItem::new(
                format!(
                    "{}:{} {} {}",
                    diagnostic.start.line + 1,
                    diagnostic.start.col + 1,
                    diagnostic.severity.letter(),
                    diagnostic.message.lines().next().unwrap_or_default()
                ),
                PickerTarget::Location(path.clone(), diagnostic.start),
            )
        })
        .collect();
    if items.is_empty() {
        app.info("no diagnostics");
        return;
    }
    app.picker = Some(Picker::new("diagnostics", items));
    app.mode = Mode::Picker;
}

/// `:lsp` — which servers are running and which could not start.
pub fn describe(app: &mut App) {
    let mut lines = Vec::new();
    for (language, client) in &app.lsp.clients {
        let state = if client.is_ready() {
            "running"
        } else {
            "starting"
        };
        lines.push(format!("{language:<12} {} — {state}", client.name));
    }
    for (language, reason) in &app.lsp.unavailable {
        lines.push(format!("{language:<12} {reason}"));
    }
    if lines.is_empty() {
        lines.push("no language servers — open a file in a language that has one".into());
    }
    lines.sort();
    app.show_popup("language servers", lines.join("\n"));
}

/// `:lsp restart` — stop every server and start them again for the open
/// buffers.
pub fn restart(app: &mut App) {
    shutdown(app);
    app.lsp = LspState::default();
    for index in 0..app.buffers.len() {
        attach(app, index);
    }
    app.info("language servers restarted");
}

/// Stop every server, on the way out.
pub fn shutdown(app: &mut App) {
    for client in app.lsp.clients.values_mut() {
        client.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::editor::buffer::Buffer;
    use crate::editor::document::Document;

    /// An editor with `text` in a Rust file and the mock server attached.
    fn editor(name: &str, text: &str) -> (App, PathBuf) {
        let root = std::env::temp_dir().join(format!("termi-lsp-{name}"));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("create fixture");
        let path = root.join("main.rs");
        std::fs::write(&path, text).expect("write fixture");
        let mut app = App::with_config(Config {
            system_clipboard: false,
            auto_pairs: false,
            ..Config::default()
        });
        app.buffers[0] = Buffer::new(Document::open(path.clone()).expect("open"));
        app.windows.focused_mut().reset(0);
        let client = Client::over(crate::lsp::mock::server(), "mock", &root);
        app.lsp.clients.insert("rust".to_string(), client);
        attach(&mut app, 0);
        (app, path)
    }

    /// Poll until `done` holds, for up to two seconds.
    fn wait(app: &mut App, done: impl Fn(&App) -> bool) {
        for _ in 0..400 {
            sync(app);
            poll(app);
            if done(app) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        panic!("the server never answered");
    }

    #[test]
    fn diagnostics_arrive_for_an_opened_file() {
        let (mut app, path) = editor("diagnostics", "bad code\n");
        wait(&mut app, |app| app.lsp.diagnostics.contains_key(&path));
        let found = &app.lsp.diagnostics[&path][0];
        assert_eq!(found.message, "bad thing");
        assert_eq!(
            (found.start, found.end),
            (Position::new(0, 0), Position::new(0, 3))
        );
    }

    #[test]
    fn hover_shows_a_popup_and_definition_jumps() {
        let (mut app, _) = editor("hover", "one\ntwo three\n");
        wait(&mut app, |app| app.lsp.clients["rust"].is_ready());
        request(&mut app, RequestKind::Hover);
        wait(&mut app, |app| app.popup.is_some());
        assert_eq!(
            app.popup.as_ref().map(|(_, body)| body.as_str()),
            Some("a hover")
        );
        app.popup = None;

        request(&mut app, RequestKind::Definition);
        wait(&mut app, |app| app.window().cursor().head.line == 1);
        assert_eq!(app.window().cursor().head, Position::new(1, 2));

        request(&mut app, RequestKind::References);
        wait(&mut app, |app| app.picker.is_some());
        assert_eq!(
            app.picker.as_ref().map(|p| p.title.as_str()),
            Some("references")
        );
    }

    #[test]
    fn completion_filters_as_typed_and_inserts_the_choice() {
        let (mut app, _) = editor("completion", "\n");
        wait(&mut app, |app| app.lsp.clients["rust"].is_ready());
        app.mode = Mode::Insert;
        app.edit().insert_text("al");
        after_typing(&mut app, Some('l'));
        wait(&mut app, |app| app.lsp.menu.is_some());
        let shown: Vec<&str> = app
            .lsp
            .menu
            .as_ref()
            .map(|menu| {
                menu.shown
                    .iter()
                    .map(|i| menu.items[*i].label.as_str())
                    .collect()
            })
            .unwrap_or_default();
        assert_eq!(shown, vec!["alpha", "alphabet"]);
        menu_action(&mut app, MenuAction::Next);
        menu_action(&mut app, MenuAction::Accept);
        assert_eq!(app.buffer().document.line_string(0), "alphabet");
        assert!(app.lsp.menu.is_none());
    }

    #[test]
    fn snippet_completions_insert_their_text() {
        let (mut app, _) = editor("snippet", "\n");
        wait(&mut app, |app| app.lsp.clients["rust"].is_ready());
        app.mode = Mode::Insert;
        app.edit().insert_text("pu");
        request(&mut app, RequestKind::Completion);
        wait(&mut app, |app| app.lsp.menu.is_some());
        menu_action(&mut app, MenuAction::Accept);
        assert_eq!(app.buffer().document.line_string(0), "push(value)");
    }

    #[test]
    fn rename_and_format_edit_the_buffer_as_one_step() {
        let (mut app, _) = editor("rename", "old = 1\n");
        wait(&mut app, |app| app.lsp.clients["rust"].is_ready());
        rename(&mut app, "new");
        wait(&mut app, |app| {
            app.buffer().document.line_string(0) == "new = 1"
        });
        format(&mut app);
        wait(&mut app, |app| {
            app.buffer().document.line_string(0) == "// formatted"
        });
        app.edit().undo();
        assert_eq!(app.buffer().document.line_string(0), "new = 1");
    }

    #[test]
    fn bracket_d_walks_the_diagnostics() {
        let (mut app, path) = editor("bracket-d", "bad code\nmore\n");
        wait(&mut app, |app| app.lsp.diagnostics.contains_key(&path));
        app.window_mut()
            .cursor_mut()
            .move_to(Position::new(1, 0), false);
        jump_diagnostic(&mut app, false);
        assert_eq!(app.window().cursor().head, Position::new(0, 0));
        assert_eq!(app.status.text, "bad thing");
    }
}
