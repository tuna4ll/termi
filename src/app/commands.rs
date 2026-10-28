//! Ex commands: `:w`, `:q`, `:set` and friends, plus the `:help` popup.

use std::path::PathBuf;

use super::mode::Mode;
use super::state::App;
use crate::config;
use crate::editor::command::{Command, parser};
use crate::editor::cursor::{Motion, Position};
use crate::picker::PickerKind;
use crate::theme::Theme;

const HELP: &str = "\
MODES     i a I A o O insert    v visual    V line    Alt+v block
          : command    / ? search

MOVE      h j k l     w b e  W B E words    0 ^ $ line    gg G file
          f t F T x  find on the line       ; , repeat the find
          % bracket   { } paragraph         H M L screen  zt zz zb scroll
FOLDS     za toggle   zc close   zo open   zR open all   zM close all
          Ctrl+left/right by word           Ctrl+Home/End file
          a count first repeats it: 3j  5w  10G
MARKS     ma set     'a line    `a place   (A-Z work across files)
JUMPS     Alt+left back     Alt+right forward
SELECT    Ctrl+A all        Shift+arrows      Ctrl+Shift+arrows by word
          click to place the caret, drag to select
          Backspace or Delete removes it, typing replaces it

OPERATORS d delete    c change    y yank    > < indent    gc comment
          gu gU g~ case — follow with a motion, a text object, or
          themselves for whole lines: dw  c$  y3j  dd  gcc  >ip
OBJECTS   iw aw  iW aW  ip ap  i( a(  i[ a[  i{ a{  i< a<  i\" a\"  i' a'  it at
EDIT      x X delete   s S C D change/delete   r replace   ~ case   J join
          p P put      u undo   Ctrl+R redo    . repeat the last change
          g- g+ step through every undo branch    :earlier :later
          Ctrl+/ toggle comments     Ctrl+Backspace / Ctrl+Del a word
REGISTERS \"a-\"z named   \"0 last yank   \"1-\"9 deletes   \"_ discard   \"+ clipboard
MACROS    qa record   q stop   @a play   @@ again   @: last command
CLIPBOARD Ctrl+C copy    Ctrl+X cut    Ctrl+V paste
SEARCH    / next      ? previous      n / N repeat     * # word under caret
FILES     Ctrl+O find     Ctrl+B tree                  Ctrl+S save
          Ctrl+N / Ctrl+P buffers    gt gT too
TREE      a file    A directory    r rename    c copy    m move
          d delete  . hidden files
WINDOWS   Ctrl+W then     s / v split     c close     o only      t terminal
                          h j k l focus   w next      = even
                          + - taller      < > wider
TERMINAL  :terminal [command]        Ctrl+W t opens another terminal

COMMANDS  :w [path]  :q[!]  :wq  :wa  :qa[!]  :wqa  :e[!] path
          :touch path  :mkdir path
          :find  :buffers  :commands  :themes  :bn  :bp
          :sp  :vs  :clo  :on
          :set <option> [value]     :theme <name>     :<line>
          :w! write read-only   :sudow write with sudo   :recover :dropswap
          :set ff=unix|dos   :set fenc=utf-8|utf-8-bom|latin1   :set ro
          :set! saves a setting to the config file   :config [reload]
          :set transparent   leaves the background to the terminal
          :set list  indentguides  minimap  matchbrackets
          :[range]s/a/b/[gciI]   ranges: 5,9  .,+3  %  '<,'>  'a,$
          :grep [text]  (Ctrl+G)   :greplace/a/b/   :help [topic]
GIT       ]c [c next/previous change    :blame who changed this line
          :%s/pattern/replacement/g";

/// The section headings of the help text, for `:help <Tab>`.
#[must_use]
pub fn help_topics() -> Vec<String> {
    HELP.lines()
        .filter_map(|line| line.split_whitespace().next())
        .filter(|word| word.chars().all(|ch| ch.is_ascii_uppercase()) && word.len() > 2)
        .map(str::to_lowercase)
        .collect()
}

/// The key reference, with the actions a `[keys]` table can name.
fn help_text() -> String {
    let mut text = HELP.to_string();
    text.push_str(
        "\n\nMAPPINGS  [keys.normal] [keys.insert] [keys.visual] in the config map key\n",
    );
    text.push_str("          notation (<C-s>, <Space>f, jk) to a :command, keys, or one of:\n");
    let mut line = String::from("         ");
    for name in crate::input::mappings::ACTION_NAMES {
        if line.len() + name.len() + 1 > 78 {
            text.push_str(&line);
            text.push('\n');
            line = String::from("         ");
        }
        line.push(' ');
        line.push_str(name);
    }
    text.push_str(&line);
    text
}

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
        Command::Write { path, force } => write(app, path, force),
        Command::SudoWrite => sudo_write(app),
        Command::Recover => recover(app),
        Command::DropSwap => drop_swap(app),
        Command::Quit { force } => quit(app, force),
        Command::WriteQuit { path, force } => {
            write(app, path, force);
            if !app.status.is_error {
                quit(app, force);
            }
        }
        Command::WriteAll => {
            write_all(app);
        }
        Command::QuitAll { force } => quit_all(app, force),
        Command::WriteQuitAll => {
            if write_all(app) {
                quit_all(app, false);
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
        Command::Rename(path) => move_tree_path(app, path, "renamed"),
        Command::Copy(path) => copy_tree_path(app, path),
        Command::Move(path) => move_tree_path(app, path, "moved"),
        Command::Reload => reload(app),
        Command::GotoLine(line) => {
            app.window_mut().record_jump();
            app.move_cursors(Motion::ToLine(line), false, false);
        }
        Command::GotoAddress(address) => {
            let range = crate::editor::command::LineRange {
                start: address,
                end: address,
            };
            match super::substitute::resolve_range(app, Some(range)) {
                Ok((line, _)) => {
                    app.window_mut().record_jump();
                    app.move_cursors(Motion::ToLine(line), false, false);
                }
                Err(error) => app.error(error),
            }
        }
        Command::Set {
            key,
            value,
            persist,
        } => {
            set_option(app, &key, &value);
            if persist && !app.status.is_error {
                persist_option(app, &key, &value);
            }
        }
        Command::Config { reload: true } => app.reload_config(),
        Command::Config { reload: false } => open_config(app),
        Command::Theme(name) => set_theme(app, &name),
        Command::CycleBuffer { forward } => app.cycle_buffer(forward),
        Command::Substitute {
            pattern,
            replacement,
            all,
            confirm,
            case_sensitive,
            range,
        } => super::substitute::substitute(
            app,
            &pattern,
            &replacement,
            (all, confirm, case_sensitive),
            range,
        ),
        Command::ProjectReplace {
            pattern,
            replacement,
            case_sensitive,
        } => super::substitute::project_replace(app, &pattern, &replacement, case_sensitive),
        Command::Grep(pattern) => app.open_grep(pattern.as_deref()),
        Command::Blame => {
            let line = app.window().cursor().head.line;
            match app
                .buffer()
                .document
                .path()
                .and_then(|path| crate::git::blame(path, line))
            {
                Some(text) => app.info(text),
                None => app.error("no blame — is the file in a git repository?"),
            }
        }
        Command::HelpTopic(topic) => open_help(app, Some(&topic)),
        Command::Earlier(count) => {
            if !app.edit().earlier(count) {
                app.info("already at the oldest state");
            }
        }
        Command::Later(count) => {
            if !app.edit().later(count) {
                app.info("already at the newest state");
            }
        }
        Command::Help => open_help(app, None),
        Command::Find => app.open_picker(PickerKind::Files),
        Command::Buffers => app.open_picker(PickerKind::Buffers),
        Command::Commands => app.open_picker(PickerKind::Commands),
        Command::Themes => app.open_picker(PickerKind::Themes),
    }
}

fn move_tree_path(app: &mut App, destination: PathBuf, verb: &str) {
    let Some(source) = app.tree_source.clone() else {
        return app.error("select a tree entry first");
    };
    match crate::filesystem::move_path(&source, &destination) {
        Ok(()) => {
            app.relocate_buffers(&source, &destination);
            reveal_tree_path(app, &destination);
            app.info(format!("{verb} {}", destination.display()));
        }
        Err(error) => app.error(error.to_string()),
    }
    restore_tree_focus(app);
}

fn copy_tree_path(app: &mut App, destination: PathBuf) {
    let Some(source) = app.tree_source.clone() else {
        return app.error("select a tree entry first");
    };
    match crate::filesystem::copy_path(&source, &destination) {
        Ok(()) => {
            reveal_tree_path(app, &destination);
            app.info(format!("copied {}", destination.display()));
        }
        Err(error) => app.error(error.to_string()),
    }
    restore_tree_focus(app);
}

fn reveal_tree_path(app: &mut App, path: &std::path::Path) {
    if let Some(tree) = app.tree.as_mut()
        && let Some(index) = tree.reveal(path)
    {
        app.tree_selected = index;
    }
}

fn restore_tree_focus(app: &mut App) {
    if app.tree_visible {
        app.mode = Mode::Tree;
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
            if !directory {
                if let Err(error) = app.open(path) {
                    app.error(error.to_string());
                    return;
                }
                app.tree_visible = false;
                app.mode = Mode::Normal;
                return;
            }
        }
        Err(error) => app.error(error.to_string()),
    }
    if app.tree_visible {
        app.mode = Mode::Tree;
    }
}

/// `:w`, `:w!` and Ctrl+S.
///
/// A read-only buffer is only written when forced, or to another path.
pub(super) fn write(app: &mut App, path: Option<PathBuf>, force: bool) {
    let name = app.buffer().document.display_name().to_string();
    if path.is_none() && !force && app.buffer().document.is_readonly() {
        let hint = if cfg!(unix) {
            ", :w! to try anyway, or :sudow to write it with sudo"
        } else {
            " — :w! to try anyway"
        };
        app.error(format!("{name} is read-only{hint}"));
        return;
    }
    let config = app.buffer_config(app.window().buffer);
    if config.trim_trailing_whitespace {
        app.edit().trim_trailing_whitespace();
    }
    let renamed = path.is_some();
    let undo_dir = app.undo_dir();
    let result = match path {
        Some(path) => app.buffer_mut().save_as(path, undo_dir.as_deref()),
        None => app.buffer_mut().save(undo_dir.as_deref()),
    };
    match result {
        Ok(()) => {
            // `:w hello.c` is how an unnamed buffer gets its name, and the name
            // is what picks the language — so the highlighter has to be chosen
            // again rather than left at whatever the old path implied.
            if renamed {
                app.buffer_mut().detect_language();
                if app.config.git {
                    app.buffer_mut().refresh_git();
                }
            }
            let name = app.buffer().document.display_name().to_string();
            let lines = app.buffer().document.len_lines();
            app.info(format!("wrote {name} — {lines} lines"));
        }
        Err(error) => app.error(error.to_string()),
    }
}

/// Write every modified buffer, returning whether all of them made it.
///
/// One failure does not stop the rest: a read-only file among ten should not
/// cost the user the other nine.
fn write_all(app: &mut App) -> bool {
    let undo_dir = app.undo_dir();
    let global = app.config.clone();
    let mut written = 0;
    let mut failures = Vec::new();
    for buffer in &mut app.buffers {
        if !buffer.document.is_dirty() {
            continue;
        }
        let trim = buffer.config(&global).trim_trailing_whitespace;
        if buffer.document.is_readonly() {
            failures.push(format!("{} is read-only", buffer.document.display_name()));
            continue;
        }
        if buffer.document.path().is_none() {
            failures.push(format!(
                "{} has no file name",
                buffer.document.display_name()
            ));
            continue;
        }
        if trim {
            let mut window = crate::editor::window::Window::new(0);
            crate::editor::edit::Edit::new(buffer, &mut window).trim_trailing_whitespace();
        }
        match buffer.save(undo_dir.as_deref()) {
            Ok(()) => written += 1,
            Err(error) => failures.push(error.to_string()),
        }
    }
    if failures.is_empty() {
        app.info(format!(
            "wrote {written} file{}",
            if written == 1 { "" } else { "s" }
        ));
        true
    } else {
        app.error(failures.join("; "));
        false
    }
}

/// `:sudow` — hand the encoded text to the event loop, which suspends the
/// interface and runs `sudo tee`.
fn sudo_write(app: &mut App) {
    if !cfg!(unix) {
        app.error(":sudow needs sudo, which this platform does not have");
        return;
    }
    let Some(path) = app
        .buffer()
        .document
        .path()
        .map(std::path::Path::to_path_buf)
    else {
        app.error("no file name — use :w <path>");
        return;
    };
    match app.buffer().document.to_disk_bytes() {
        Ok(bytes) => {
            app.pending_sudo = Some(super::state::SudoWrite {
                buffer: app.window().buffer,
                path,
                bytes,
            });
        }
        Err(error) => app.error(error.to_string()),
    }
}

/// `:recover` — replace the buffer's text with what its swap file kept, as
/// one undoable change.
fn recover(app: &mut App) {
    let (Some(directory), Some(path)) = (
        app.swap_dir(),
        app.buffer()
            .document
            .path()
            .map(std::path::Path::to_path_buf),
    ) else {
        app.error("nothing to recover");
        return;
    };
    let Some((swap, _)) = crate::filesystem::swap::read(&directory, &path) else {
        app.error(format!("no swap file for {}", path.display()));
        return;
    };
    let length = app.buffer().document.len_chars();
    let mut edit = app.edit();
    edit.begin_group();
    edit.replace(0, length, &swap.text);
    edit.end_group();
    app.clamp_cursors(false);
    app.info("recovered — check the text, then :w to keep it");
}

/// `:dropswap` — forget a swap file's unsaved text.
fn drop_swap(app: &mut App) {
    if let (Some(directory), Some(path)) = (app.swap_dir(), app.buffer().document.path()) {
        crate::filesystem::swap::remove(&directory, path);
        app.info("swap file removed");
    }
}

fn quit_all(app: &mut App, force: bool) {
    if !force && let Some(message) = app.unsaved_message() {
        app.error(message);
        return;
    }
    app.quit();
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
    match app.buffer_mut().reload() {
        Ok(()) => {
            app.window_mut().clear_secondary_cursors();
            app.clamp_cursors(false);
            app.info("reloaded from disk");
        }
        Err(error) => app.error(error.to_string()),
    }
}

fn set_theme(app: &mut App, name: &str) {
    let (mut theme, error) = Theme::load(name, &config::themes_dir());
    if app.config.transparent_background {
        theme.make_transparent();
    }
    app.theme = theme;
    app.config.theme = name.to_string();
    match error {
        Some(message) => app.error(message),
        None => app.info(format!("theme: {}", app.theme.name)),
    }
}

/// The name an option has in the config file, and how to write its value
/// there; `None` for options that only make sense for one session or buffer.
/// Turns what `:set` was given into a value for the config file.
type Convert = fn(&str) -> Option<config::persist::Value>;

fn config_key(key: &str) -> Option<(&'static str, Convert)> {
    use config::persist::Value;
    let flag = |value: &str| Some(Value::Bool(matches!(value, "true" | "on" | "yes" | "1")));
    let number = |value: &str| value.parse::<i64>().ok().map(Value::Int);
    let text = |value: &str| Some(Value::Text(value.to_string()));
    Some(match key {
        "number" | "nu" | "line_numbers" => ("line_numbers", flag),
        "relativenumber" | "rnu" | "relative_line_numbers" => ("relative_line_numbers", flag),
        "wrap" | "word_wrap" => ("word_wrap", flag),
        "autoindent" | "ai" | "auto_indent" => ("auto_indent", flag),
        "autopairs" | "ap" | "auto_pairs" => ("auto_pairs", flag),
        "expandtab" | "et" | "expand_tabs" => ("expand_tabs", flag),
        "cursorline" | "cul" | "highlight_current_line" => ("highlight_current_line", flag),
        "tabs" | "show_tabs" => ("show_tabs", flag),
        "tabstop" | "ts" | "tab_width" | "shiftwidth" | "sw" => ("tab_width", number),
        "scrolloff" | "so" => ("scrolloff", number),
        "trim" | "trim_trailing_whitespace" => ("trim_trailing_whitespace", flag),
        "finalnewline" | "fixeol" | "insert_final_newline" => ("insert_final_newline", flag),
        "transparent" | "transparent_background" => ("transparent_background", flag),
        "list" | "show_whitespace" => ("show_whitespace", flag),
        "indentguides" | "ig" | "indent_guides" => ("indent_guides", flag),
        "matchbrackets" | "mb" | "match_brackets" => ("match_brackets", flag),
        "minimap" => ("minimap", flag),
        "theme" => ("theme", text),
        _ => return None,
    })
}

/// `:set!` — write what `:set` just did into the config file.
fn persist_option(app: &mut App, key: &str, value: &str) {
    let Some((name, convert)) = config_key(key) else {
        app.error(format!(
            "{key} only applies to this session and cannot be saved"
        ));
        return;
    };
    let Some(converted) = convert(value) else {
        app.error(format!("{value} is not a value for {name}"));
        return;
    };
    let path = app
        .startup
        .config_path
        .clone()
        .unwrap_or_else(config::config_file);
    match config::persist::store(&path, name, converted) {
        Ok(()) => app.info(format!("{name} = {value}, saved to {}", path.display())),
        Err(error) => app.error(error.to_string()),
    }
}

/// `:config` — open the config file, creating a commented one if there is
/// none yet.
fn open_config(app: &mut App) {
    let path = app
        .startup
        .config_path
        .clone()
        .unwrap_or_else(config::config_file);
    if !path.exists() {
        let created = path
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| std::fs::write(&path, CONFIG_TEMPLATE));
        if let Err(error) = created {
            app.error(format!("cannot create {}: {error}", path.display()));
            return;
        }
    }
    edit(app, path, false);
}

/// What a new config file starts as: every setting, commented out.
const CONFIG_TEMPLATE: &str = include_str!("../../config.example.toml");

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
        "expandtab" | "et" | "expand_tabs" => {
            app.config.expand_tabs = boolean();
            app.buffer_mut().options.expand_tabs = Some(boolean());
        }
        "trim" | "trim_trailing_whitespace" => {
            app.config.trim_trailing_whitespace = boolean();
            app.buffer_mut().options.trim_trailing_whitespace = Some(boolean());
        }
        "finalnewline" | "fixeol" | "insert_final_newline" => {
            app.config.insert_final_newline = boolean();
            app.buffer_mut().options.insert_final_newline = Some(boolean());
        }
        "fileformat" | "ff" => match crate::editor::document::LineEnding::from_name(value) {
            Some(ending) => app.buffer_mut().document.set_line_ending(ending),
            None => return app.error("fileformat is unix or dos"),
        },
        "encoding" | "enc" | "fileencoding" | "fenc" => {
            match crate::filesystem::Encoding::from_name(value) {
                Some(encoding) => app.buffer_mut().document.set_encoding(encoding),
                None => return app.error("encoding is utf-8, utf-8-bom or latin1"),
            }
        }
        "readonly" | "ro" => app.buffer_mut().document.set_readonly(boolean()),
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
                return app.error(format!(
                    "unknown language: {value} (known: {})",
                    crate::syntax::names().join(", ")
                ));
            }
        }
        "tabs" | "show_tabs" => app.config.show_tabs = boolean(),
        "tabstop" | "ts" | "tab_width" | "shiftwidth" | "sw" => {
            let width = number(app.config.tab_width).clamp(1, 16);
            app.config.tab_width = width;
            app.buffer_mut().options.tab_width = Some(width);
        }
        "scrolloff" | "so" => app.config.scrolloff = number(app.config.scrolloff).min(32),
        "ignorecase" | "ic" => app.search.case_sensitive = Some(!boolean()),
        "regex" => app.search.regex = boolean(),
        "theme" => return set_theme(app, value),
        "list" | "show_whitespace" => app.config.show_whitespace = boolean(),
        "indentguides" | "ig" | "indent_guides" => app.config.indent_guides = boolean(),
        "matchbrackets" | "mb" | "match_brackets" => app.config.match_brackets = boolean(),
        "minimap" => app.config.minimap = boolean(),
        "transparent" | "transparent_background" => {
            app.config.transparent_background = boolean();
            let name = app.config.theme.clone();
            set_theme(app, &name);
        }
        other => return app.error(format!("unknown option: {other}")),
    }
    app.info(format!("{key} = {value}"));
}

/// The label the help buffer goes by.
const HELP_NAME: &str = "[help]";

/// `:help [topic]` — show the reference in a read-only buffer, so it can be
/// searched, scrolled and split like any other text; with a topic, start at
/// its first mention.
fn open_help(app: &mut App, topic: Option<&str>) {
    let index = app
        .buffers
        .iter()
        .position(|buffer| buffer.document.display_name() == HELP_NAME)
        .unwrap_or_else(|| {
            let mut document = crate::editor::document::Document::scratch(&help_text(), HELP_NAME);
            document.set_readonly(true);
            app.buffers
                .push(crate::editor::buffer::Buffer::new(document));
            app.buffers.len() - 1
        });
    app.window_mut().show(index);
    let Some(topic) = topic else {
        return;
    };
    let document = &app.buffer().document;
    let needle = topic.to_lowercase();
    let found = (0..document.len_lines())
        .find(|line| document.line_string(*line).to_lowercase().contains(&needle));
    match found {
        Some(line) => {
            app.window_mut().record_jump();
            app.window_mut()
                .cursor_mut()
                .move_to(Position::new(line, 0), false);
            app.window_mut().view.top_line = line;
        }
        None => app.error(format!("no help for {topic}")),
    }
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
    fn undo_after_a_reload_does_not_replay_edits_into_the_new_text() {
        let path = fixture("reload-forgets-history").join("notes.txt");
        std::fs::write(&path, "one\n").expect("write fixture");
        let mut app = app();
        app.open(path.clone()).expect("open fixture");
        app.edit().insert_text("zero ");
        run(&mut app, "w");

        std::fs::write(&path, "something else entirely\n").expect("rewrite fixture");
        run(&mut app, "e!");
        assert!(!app.edit().undo(), "the old history must be gone");
        assert_eq!(
            app.buffer().document.text().to_string(),
            "something else entirely\n"
        );
    }

    #[test]
    fn quit_all_names_the_unsaved_files() {
        let root = fixture("quit-all-names");
        let mut app = app();
        app.open(root.join("a.txt")).expect("open a");
        app.edit().insert_text("x");
        app.open(root.join("b.txt")).expect("open b");

        run(&mut app, "qa");
        assert!(app.status.is_error);
        assert!(app.status.text.contains("a.txt"), "{}", app.status.text);
        assert!(!app.status.text.contains("b.txt"));
        assert!(!app.should_quit());

        run(&mut app, "qa!");
        assert!(app.should_quit());
    }

    #[test]
    fn write_all_saves_every_modified_buffer() {
        let root = fixture("write-all");
        let mut app = app();
        for name in ["a.txt", "b.txt"] {
            app.open(root.join(name)).expect("open");
            app.edit().insert_text(name);
        }
        run(&mut app, "wa");
        assert!(!app.status.is_error, "{}", app.status.text);
        assert_eq!(
            std::fs::read_to_string(root.join("a.txt")).unwrap(),
            "a.txt"
        );
        assert_eq!(
            std::fs::read_to_string(root.join("b.txt")).unwrap(),
            "b.txt"
        );
    }

    #[test]
    fn a_read_only_buffer_needs_a_bang_to_write() {
        let path = fixture("read-only-write").join("locked.txt");
        std::fs::write(&path, "locked").expect("write fixture");
        let mut app = app();
        app.open(path.clone()).expect("open fixture");
        app.buffer_mut().document.set_readonly(true);
        app.edit().insert_text("still ");

        run(&mut app, "w");
        assert!(app.status.is_error);
        assert!(app.status.text.contains("read-only"), "{}", app.status.text);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "locked");

        run(&mut app, "w!");
        assert!(!app.status.is_error, "{}", app.status.text);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "still locked");
    }

    #[test]
    fn a_latin1_file_is_written_back_byte_for_byte() {
        let path = fixture("latin1-round-trip").join("old.txt");
        std::fs::write(&path, [b'c', b'a', b'f', 0xe9]).expect("write fixture");
        let mut app = app();
        app.open(path.clone()).expect("open fixture");
        assert_eq!(app.buffer().document.text().to_string(), "café");
        app.edit().insert_text("> ");
        run(&mut app, "w");
        assert_eq!(std::fs::read(&path).unwrap(), b"> caf\xe9");
    }

    #[test]
    fn line_endings_switch_with_fileformat() {
        let path = fixture("fileformat").join("lines.txt");
        std::fs::write(&path, "a\nb\n").expect("write fixture");
        let mut app = app();
        app.open(path.clone()).expect("open fixture");
        run(&mut app, "set ff=dos");
        assert!(app.buffer().document.is_dirty());
        run(&mut app, "w");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "a\r\nb\r\n");
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

    #[test]
    fn a_created_file_opens_immediately() {
        let path = fixture("create-opens").join("notes.txt");
        let mut app = app();

        run(&mut app, &format!("touch {}", path.display()));

        assert_eq!(app.buffer().document.path(), Some(path.as_path()));
        assert!(!app.tree_visible);
        assert_eq!(app.mode, Mode::Normal);
    }

    #[test]
    fn renaming_updates_an_open_buffer_path() {
        let root = fixture("rename-buffer");
        let source = root.join("old.rs");
        let destination = root.join("new.py");
        std::fs::write(&source, "print('hi')").expect("write fixture");
        let mut app = app();
        app.open(source.clone()).expect("open fixture");
        app.tree_source = Some(source.clone());

        run(&mut app, &format!("rename {}", destination.display()));

        assert!(!source.exists());
        assert!(destination.exists());
        assert_eq!(app.buffer().document.path(), Some(destination.as_path()));
        assert_eq!(app.buffer().syntax.language_name(), "python");
    }

    #[test]
    fn copying_keeps_the_source_and_creates_the_destination() {
        let root = fixture("copy-entry");
        let source = root.join("source.txt");
        let destination = root.join("copy.txt");
        std::fs::write(&source, "hello").expect("write fixture");
        let mut app = app();
        app.tree_source = Some(source.clone());

        run(&mut app, &format!("copy {}", destination.display()));

        assert!(source.exists());
        assert_eq!(
            std::fs::read_to_string(destination).expect("read copy"),
            "hello"
        );
    }
}
