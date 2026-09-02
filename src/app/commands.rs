//! # Command execution
//!
//! **Purpose:** carry out a parsed `:` command.
//!
//! **Responsibility:** the effect half of [`crate::editor::command`]. Every arm
//! ends by putting a message in the status area, because a command line that
//! silently does nothing is indistinguishable from one that failed.
//!
//! **Public API:** [`run`].

use std::path::PathBuf;

use super::mode::Mode;
use super::state::App;
use crate::config;
use crate::editor::command::{Command, parser};
use crate::editor::cursor::Motion;
use crate::theme::Theme;

/// Text of the `:help` popup.
///
/// Kept as one literal rather than generated from the keymap: the keymap is
/// exhaustive and this is the short list worth memorising.
const HELP: &str = "\
MODES     i insert    v visual    V line    : command    / search

MOVE      h j k l     w b e words     0 ^ $ line     gg G file
EDIT      x delete    dd line     yy yank    p paste    u undo
SEARCH    / next      ? previous      n / N repeat
FILES     Ctrl+B tree     a file / A directory        Ctrl+S save
          Ctrl+N / Ctrl+P buffers
WINDOWS   Ctrl+W then     s / v split     c close     o only
                          h j k l focus   w next      = even
                          + - taller      < > wider

COMMANDS  :w [path]  :q[!]  :wq  :e[!] path  :touch path  :mkdir path
          :bn  :bp
          :sp  :vs  :clo  :on
          :set <option> [value]     :theme <name>     :<line>
          :%s/pattern/replacement/g";

/// Parse and execute a command line, without the leading `:`.
pub fn run(app: &mut App, line: &str) {
    match parser::parse(line) {
        Ok(command) => execute(app, command),
        Err(message) => app.error(message),
    }
}

fn execute(app: &mut App, command: Command) {
    match command {
        Command::Write(path) => write(app, path),
        Command::Quit { force } => quit(app, force),
        Command::WriteQuit { force } => {
            write(app, None);
            if !app.status.is_error {
                quit(app, force);
            }
        }
        Command::Split { axis } => {
            app.windows.split(axis);
        }
        Command::CloseWindow => {
            if !app.windows.close(app.windows.focus()) {
                app.info("only one window");
            }
        }
        Command::OnlyWindow => app.windows.close_others(),
        Command::Edit { path, force } => edit(app, path, force),
        Command::CreateFile(path) => create_path(app, path, false),
        Command::CreateDirectory(path) => create_path(app, path, true),
        Command::Reload => reload(app),
        Command::GotoLine(line) => app.move_cursors(Motion::ToLine(line), false, false),
        Command::Set { key, value } => set_option(app, &key, &value),
        Command::Theme(name) => set_theme(app, &name),
        Command::CycleBuffer { forward } => app.cycle_buffer(forward),
        Command::Substitute {
            pattern,
            replacement,
            all,
            whole_file,
        } => substitute(app, &pattern, &replacement, all, whole_file),
        Command::Help => app.show_popup("termi", HELP),
    }
}

/// Create a filesystem entry and immediately make it visible in an open tree.
fn create_path(app: &mut App, path: PathBuf, directory: bool) {
    let result = if directory {
        crate::filesystem::create_directory(&path)
    } else {
        crate::filesystem::create_file(&path)
    };

    match result {
        Ok(()) => {
            if let Some(tree) = app.tree.as_mut()
                && let Some(index) = tree.reveal(&path)
            {
                app.tree_selected = index;
            }
            let kind = if directory { "directory" } else { "file" };
            app.info(format!("created {kind} {}", path.display()));
        }
        Err(error) => app.error(error.to_string()),
    }
    if app.tree_visible {
        app.mode = Mode::Tree;
    }
}

fn write(app: &mut App, path: Option<PathBuf>) {
    if app.config.trim_trailing_whitespace {
        app.edit().trim_trailing_whitespace();
    }
    let result = match path {
        Some(path) => app.buffer_mut().document.save_as(path),
        None => app.buffer_mut().document.save(),
    };
    match result {
        Ok(()) => {
            let name = app.buffer().document.display_name().to_string();
            let lines = app.buffer().document.len_lines();
            app.info(format!("wrote {name} — {lines} lines"));
        }
        Err(error) => app.error(error.to_string()),
    }
}

/// `:q` closes the window, then the buffer, then the editor.
///
/// Closing a window is always safe — the buffer stays open behind it — so the
/// unsaved-changes check only applies once this is the last view of the file.
fn quit(app: &mut App, force: bool) {
    if app.windows.count() > 1 {
        app.windows.close(app.windows.focus());
        return;
    }
    if !force && app.buffer().document.is_dirty() {
        app.error("unsaved changes — use :q! to discard them");
        return;
    }
    if app.buffers.len() == 1 {
        app.quit();
    } else {
        app.close_active();
    }
}

fn edit(app: &mut App, path: PathBuf, force: bool) {
    if !force && app.buffer().document.is_dirty() {
        app.error("unsaved changes — use :e! to discard them");
        return;
    }
    match app.open(path) {
        Ok(()) => {
            let name = app.buffer().document.display_name().to_string();
            app.info(format!("opened {name}"));
        }
        Err(error) => app.error(error.to_string()),
    }
}

fn reload(app: &mut App) {
    match app.buffer_mut().document.reload() {
        Ok(()) => {
            // The file may have shrunk, so no cursor can be trusted afterwards.
            app.window_mut().clear_secondary_cursors();
            app.clamp_cursors(false);
            app.info("reloaded from disk");
        }
        Err(error) => app.error(error.to_string()),
    }
}

fn set_theme(app: &mut App, name: &str) {
    let (theme, error) = Theme::load(name, &config::themes_dir());
    app.theme = theme;
    app.config.theme = name.to_string();
    match error {
        Some(message) => app.error(message),
        None => app.info(format!("theme: {}", app.theme.name)),
    }
}

/// Change one setting for the current session.
///
/// Names accept the vi spellings as well as the config-file ones, so muscle
/// memory from either direction works.
fn set_option(app: &mut App, key: &str, value: &str) {
    let boolean = || matches!(value, "true" | "on" | "yes" | "1");
    let number = |fallback: usize| value.parse::<usize>().unwrap_or(fallback);

    match key {
        "number" | "nu" | "line_numbers" => app.config.line_numbers = boolean(),
        "relativenumber" | "rnu" | "relative_line_numbers" => {
            app.config.relative_line_numbers = boolean();
        }
        "wrap" | "word_wrap" => app.config.word_wrap = boolean(),
        "autoindent" | "ai" | "auto_indent" => app.config.auto_indent = boolean(),
        "expandtab" | "et" | "expand_tabs" => app.config.expand_tabs = boolean(),
        "cursorline" | "cul" | "highlight_current_line" => {
            app.config.highlight_current_line = boolean();
        }
        // `:set syntax` takes either a switch or a language name, which is how
        // both vi and every editor that copied it behave.
        "syntax" | "syntax_highlighting" => {
            if matches!(
                value,
                "true" | "false" | "on" | "off" | "yes" | "no" | "0" | "1"
            ) {
                app.config.syntax_highlighting = boolean();
            } else if let Some(language) = crate::syntax::by_name(value) {
                app.buffer_mut().syntax.set_language(Some(language));
            } else {
                return app.error(format!("unknown language: {value}"));
            }
        }
        "tabs" | "show_tabs" => app.config.show_tabs = boolean(),
        "tabstop" | "ts" | "tab_width" => {
            app.config.tab_width = number(app.config.tab_width).clamp(1, 16);
        }
        "scrolloff" | "so" => app.config.scrolloff = number(app.config.scrolloff).min(32),
        "ignorecase" | "ic" => app.search.case_sensitive = Some(!boolean()),
        "regex" => app.search.regex = boolean(),
        "theme" => return set_theme(app, value),
        other => return app.error(format!("unknown option: {other}")),
    }
    app.info(format!("{key} = {value}"));
}

fn substitute(app: &mut App, pattern: &str, replacement: &str, all: bool, whole_file: bool) {
    let previous = std::mem::take(&mut app.search.query);
    app.search.set_query(pattern.to_string());

    if let Some(error) = app.search.error() {
        let error = error.to_string();
        app.search.set_query(previous);
        return app.error(error);
    }

    let lines = if whole_file {
        0..app.buffer().document.len_lines()
    } else {
        let line = app.window().cursor().head.line;
        line..line + 1
    };

    let index = app.window().buffer;
    let App {
        buffers, search, ..
    } = app;
    let replaced = search.replace_in(&mut buffers[index].document, lines, replacement, all);

    // The replacement went straight into the document, so the highlighter is
    // still holding state it derived from the text that used to be there.
    app.buffer_mut().invalidate_syntax_from(0);
    // Substitution rewrites whole lines, so the caret may now be past the end.
    app.clamp_cursors(false);
    // Rewriting lines wholesale cannot be expressed as one coalesced edit, so
    // close the undo step to keep the next keystroke separate.
    app.edit().checkpoint();

    if replaced == 0 {
        app.error(format!("no match for {pattern}"));
    } else {
        app.info(format!("replaced {replaced} occurrence(s)"));
    }
    app.mode = Mode::Normal;
}
