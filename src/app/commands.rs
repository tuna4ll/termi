//! Ex commands: `:w`, `:q`, `:set` and friends, plus the `:help` popup.

use std::path::PathBuf;

use super::mode::Mode;
use super::state::App;
use crate::config;
use crate::editor::command::{Command, parser};
use crate::editor::cursor::Motion;
use crate::theme::Theme;

const HELP: &str = "\
MODES     i insert    v visual    V line    : command    / search

MOVE      h j k l     w b e words     0 ^ $ line     gg G file
          Ctrl+left/right by word           Ctrl+Home/End file
SELECT    Ctrl+A all        Shift+arrows      Ctrl+Shift+arrows by word
          click to place the caret, drag to select
          Backspace or Delete removes it, typing replaces it
EDIT      x delete    dd line     yy yank    p paste    u undo
          Ctrl+Backspace / Ctrl+Del delete a word or a run of blanks
CLIPBOARD Ctrl+C copy    Ctrl+X cut    Ctrl+V paste
SEARCH    / next      ? previous      n / N repeat
FILES     Ctrl+B tree     a file / A directory        Ctrl+S save
          Ctrl+N / Ctrl+P buffers
WINDOWS   Ctrl+W then     s / v split     c close     o only      t terminal
                          h j k l focus   w next      = even
                          + - taller      < > wider
TERMINAL  :terminal [command]        Ctrl+W t opens another terminal

COMMANDS  :w [path]  :q[!]  :wq  :e[!] path  :touch path  :mkdir path
          :bn  :bp
          :sp  :vs  :clo  :on
          :set <option> [value]     :theme <name>     :<line>
          :%s/pattern/replacement/g";

pub fn run(app: &mut App, line: &str) {
    if let Some(command) = terminal_command(line) {
        match app.open_terminal(command) {
            Ok(_) => app.clear_status(),
            Err(error) => app.error(format!("unable to open terminal: {error}")),
        }
        return;
    }
    match parser::parse(line) {
        Ok(command) => execute(app, command),
        Err(message) => app.error(message),
    }
}

/// Recognise the application-owned terminal command before the editor command
/// parser sees it. The editor core stays unaware of processes this way.
fn terminal_command(input: &str) -> Option<Option<&str>> {
    let input = input.trim();
    let (name, rest) = input
        .split_once(char::is_whitespace)
        .map_or((input, ""), |(name, rest)| (name, rest.trim()));
    matches!(name, "term" | "terminal").then_some((!rest.is_empty()).then_some(rest))
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
            app.split_window(axis);
        }
        Command::CloseWindow => {
            if !app.close_window(app.windows.focus()) {
                app.info("only one window");
            }
        }
        Command::OnlyWindow => app.close_other_windows(),
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
    let renamed = path.is_some();
    let result = match path {
        Some(path) => app.buffer_mut().document.save_as(path),
        None => app.buffer_mut().document.save(),
    };
    match result {
        Ok(()) => {
            // `:w hello.c` is how an unnamed buffer gets its name, and the name
            // is what picks the language — so the highlighter has to be chosen
            // again rather than left at whatever the old path implied.
            if renamed {
                app.buffer_mut().detect_language();
            }
            let name = app.buffer().document.display_name().to_string();
            let lines = app.buffer().document.len_lines();
            app.info(format!("wrote {name} — {lines} lines"));
        }
        Err(error) => app.error(error.to_string()),
    }
}

fn quit(app: &mut App, force: bool) {
    if app.windows.count() > 1 {
        app.close_window(app.windows.focus());
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
        "autopairs" | "ap" | "auto_pairs" => app.config.auto_pairs = boolean(),
        "expandtab" | "et" | "expand_tabs" => app.config.expand_tabs = boolean(),
        "cursorline" | "cul" | "highlight_current_line" => {
            app.config.highlight_current_line = boolean();
        }
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

    app.buffer_mut().invalidate_syntax_from(0);
    app.clamp_cursors(false);
    app.edit().checkpoint();

    if replaced == 0 {
        app.error(format!("no match for {pattern}"));
    } else {
        app.info(format!("replaced {replaced} occurrence(s)"));
    }
    app.mode = Mode::Normal;
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    fn app() -> App {
        App::with_config(Config {
            system_clipboard: false,
            ..Config::default()
        })
    }

    fn fixture(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("termi-commands-{name}"));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("create fixture");
        root
    }

    #[test]
    fn terminal_commands_keep_the_optional_command_intact() {
        assert_eq!(terminal_command("terminal"), Some(None));
        assert_eq!(
            terminal_command("term cargo test --all"),
            Some(Some("cargo test --all"))
        );
        assert_eq!(terminal_command("theme terminal"), None);
    }

    #[test]
    fn writing_an_unnamed_buffer_to_a_path_picks_up_its_language() {
        let path = fixture("write-detects-language").join("hello.c");
        let mut app = app();
        assert_eq!(app.buffer().syntax.language_name(), "plain");

        run(&mut app, &format!("w {}", path.display()));

        assert!(!app.status.is_error, "{}", app.status.text);
        assert_eq!(app.buffer().syntax.language_name(), "c");
    }

    #[test]
    fn writing_under_a_new_name_changes_the_language_with_it() {
        let root = fixture("write-changes-language");
        let mut app = app();

        run(&mut app, &format!("w {}", root.join("a.c").display()));
        assert_eq!(app.buffer().syntax.language_name(), "c");

        run(&mut app, &format!("w {}", root.join("a.py").display()));
        assert_eq!(app.buffer().syntax.language_name(), "python");
    }
}
