//! Tab completion and history for the command line.
//!
//! Completion looks at the word under the caret and what came before it: the
//! first word completes to a command name, the argument of `:e` or `:w` to a
//! path, of `:set` to an option, of `:theme` to a theme. Tab steps through
//! the candidates in place, Shift+Tab steps back, and the list is shown above
//! the command line while it lasts.

use std::path::PathBuf;

use super::state::App;

/// Every command name, for completing the first word.
pub const COMMANDS: &[&str] = &[
    "bnext",
    "bprev",
    "buffers",
    "close",
    "commands",
    "config",
    "copy",
    "dropswap",
    "earlier",
    "edit",
    "find",
    "grep",
    "greplace",
    "help",
    "later",
    "mkdir",
    "move",
    "only",
    "qall",
    "quit",
    "recover",
    "rename",
    "set",
    "split",
    "sudowrite",
    "terminal",
    "theme",
    "themes",
    "touch",
    "vsplit",
    "wall",
    "wq",
    "wqall",
    "write",
];

/// Options `:set` knows, for completing its argument.
const OPTIONS: &[&str] = &[
    "autoindent",
    "autopairs",
    "cursorline",
    "encoding",
    "expandtab",
    "fileformat",
    "finalnewline",
    "ignorecase",
    "indentguides",
    "list",
    "matchbrackets",
    "minimap",
    "number",
    "readonly",
    "regex",
    "relativenumber",
    "scrolloff",
    "syntax",
    "tabs",
    "tabstop",
    "theme",
    "transparent",
    "trim",
    "wrap",
];

/// Commands whose argument is a path.
const PATH_COMMANDS: &[&str] = &[
    "e", "edit", "w", "write", "wq", "x", "touch", "mkdir", "rename", "copy", "move", "sp",
    "split", "vs", "vsplit",
];

/// A completion in progress.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Completion {
    /// Every candidate for the word being completed.
    pub candidates: Vec<String>,
    /// The candidate on show; `None` before the first Tab lands on one.
    pub selected: Option<usize>,
    /// Where in the command line the completed word starts, in bytes.
    pub start: usize,
    /// The word as it was typed, restored when stepping back past the start.
    pub typed: String,
}

/// The candidates for the last word of `line`, and where that word starts.
#[must_use]
pub fn candidates(line: &str) -> (usize, Vec<String>) {
    let start = line.rfind(char::is_whitespace).map_or(0, |at| at + 1);
    let word = &line[start..];
    if start == 0 {
        return (0, filter(COMMANDS.iter().map(ToString::to_string), word));
    }
    let command = line
        .split_whitespace()
        .next()
        .unwrap_or("")
        .trim_end_matches('!');
    let found = match command {
        "set" | "se" => match word.split_once('=') {
            Some((option, value)) => {
                let values = option_values(option);
                let offset = option.len() + 1;
                return (start + offset, filter(values.into_iter(), value));
            }
            None => filter(OPTIONS.iter().map(ToString::to_string), word),
        },
        "theme" => filter(theme_names().into_iter(), word),
        "help" | "h" => filter(help_topics().into_iter(), word),
        "config" => filter(["reload".to_string()].into_iter(), word),
        "syntax" => filter(crate::syntax::names().into_iter().map(str::to_string), word),
        command if PATH_COMMANDS.contains(&command) => paths(word),
        _ => Vec::new(),
    };
    (start, found)
}

fn filter(options: impl Iterator<Item = String>, prefix: &str) -> Vec<String> {
    let mut found: Vec<String> = options
        .filter(|option| option.starts_with(prefix))
        .collect();
    found.sort();
    found.dedup();
    found
}

fn option_values(option: &str) -> Vec<String> {
    let names: Vec<String> = match option {
        "theme" => return theme_names(),
        "syntax" => {
            return crate::syntax::names()
                .into_iter()
                .map(str::to_string)
                .collect();
        }
        "fileformat" | "ff" => vec!["unix".into(), "dos".into()],
        "encoding" | "enc" | "fileencoding" | "fenc" => {
            vec!["utf-8".into(), "utf-8-bom".into(), "latin1".into()]
        }
        _ => vec!["on".into(), "off".into()],
    };
    names
}

fn theme_names() -> Vec<String> {
    let mut names = vec!["dark".to_string(), "light".to_string()];
    if let Ok(files) = std::fs::read_dir(crate::config::themes_dir()) {
        names.extend(files.flatten().filter_map(|file| {
            let path = file.path();
            (path.extension()? == "toml")
                .then(|| path.file_stem()?.to_str().map(str::to_string))
                .flatten()
        }));
    }
    names
}

/// The headings of the help text, lower case.
fn help_topics() -> Vec<String> {
    super::commands::help_topics()
}

/// Files and directories whose path starts with `word`, directories with a
/// trailing separator so a second Tab carries on into them.
fn paths(word: &str) -> Vec<String> {
    let expanded = expand_home(word);
    let (directory, prefix) = match expanded.rfind(['/', std::path::MAIN_SEPARATOR]) {
        Some(at) => (expanded[..=at].to_string(), expanded[at + 1..].to_string()),
        None => (String::new(), expanded.clone()),
    };
    let read_from: PathBuf = if directory.is_empty() {
        PathBuf::from(".")
    } else {
        PathBuf::from(&directory)
    };
    let Ok(entries) = std::fs::read_dir(&read_from) else {
        return Vec::new();
    };
    let shown_prefix = &word[..word.len() - prefix.len().min(word.len())];
    let mut found: Vec<String> = entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !name.starts_with(&prefix) || (name.starts_with('.') && !prefix.starts_with('.')) {
                return None;
            }
            let slash = if entry.path().is_dir() { "/" } else { "" };
            Some(format!("{shown_prefix}{name}{slash}"))
        })
        .collect();
    found.sort_by_key(|path| (!path.ends_with('/'), path.clone()));
    found
}

fn expand_home(word: &str) -> String {
    match word.strip_prefix('~') {
        Some(rest) if rest.is_empty() || rest.starts_with('/') => dirs::home_dir().map_or_else(
            || word.to_string(),
            |home| format!("{}{rest}", home.display()),
        ),
        _ => word.to_string(),
    }
}

/// Tab and Shift+Tab: step through the candidates, starting a completion if
/// none is in progress.
pub fn step(app: &mut App, forward: bool) {
    if app.completion.is_none() {
        let line = app.command_line.clone();
        let (start, candidates) = candidates(&line);
        if candidates.is_empty() {
            return;
        }
        // A single candidate is simply filled in.
        if candidates.len() == 1 {
            app.command_line.truncate(start);
            app.command_line.push_str(&candidates[0]);
            return;
        }
        app.completion = Some(Completion {
            typed: line[start..].to_string(),
            candidates,
            selected: None,
            start,
        });
    }
    let Some(completion) = app.completion.as_mut() else {
        return;
    };
    let count = completion.candidates.len();
    completion.selected = match (completion.selected, forward) {
        (None, true) => Some(0),
        (None, false) => Some(count - 1),
        (Some(index), true) => (index + 1 < count).then_some(index + 1),
        (Some(index), false) => index.checked_sub(1),
    };
    let word = completion.selected.map_or_else(
        || completion.typed.clone(),
        |index| completion.candidates[index].clone(),
    );
    app.command_line.truncate(completion.start);
    app.command_line.push_str(&word);
}

/// Stored command-line and search history, oldest first.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct History {
    /// Command lines run.
    pub commands: Vec<String>,
    /// Searches made.
    pub searches: Vec<String>,
}

/// How many entries of each kind are kept.
const HISTORY_LIMIT: usize = 200;

impl History {
    fn location() -> Option<PathBuf> {
        (!cfg!(test)).then(|| crate::config::data_dir().join("history.json"))
    }

    /// The history saved by the last session, or an empty one.
    #[must_use]
    pub fn load() -> Self {
        Self::location()
            .and_then(|path| std::fs::read_to_string(path).ok())
            .and_then(|json| serde_json::from_str(&json).ok())
            .unwrap_or_default()
    }

    /// Write the history for the next session.
    pub fn save(&self) {
        if let Some(path) = Self::location() {
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if let Ok(json) = serde_json::to_string(self) {
                let _ = crate::filesystem::write_file(&path, &json);
            }
        }
    }

    /// Remember an entry, moving a repeat to the end rather than storing it
    /// twice.
    pub fn push(list: &mut Vec<String>, entry: &str) {
        if entry.trim().is_empty() {
            return;
        }
        list.retain(|existing| existing != entry);
        list.push(entry.to_string());
        if list.len() > HISTORY_LIMIT {
            list.remove(0);
        }
    }
}

/// Up and Down through a history list: `cursor` is the entry on show,
/// `draft` what was typed before travelling began. Returns the line to show.
pub fn travel(
    list: &[String],
    cursor: &mut Option<usize>,
    draft: &mut String,
    current: &str,
    back: bool,
) -> Option<String> {
    if list.is_empty() {
        return None;
    }
    // Only entries starting with what was typed are visited, so `:e` and Up
    // finds the last file opened rather than the last command of any kind.
    let prefix = match cursor {
        None => {
            *draft = current.to_string();
            current.to_string()
        }
        Some(_) => draft.clone(),
    };
    let matching = |index: &usize| list[*index].starts_with(&prefix);
    let next = if back {
        let before = cursor.unwrap_or(list.len());
        (0..before).rev().find(matching)
    } else {
        let after = (*cursor)? + 1;
        (after..list.len()).find(matching)
    };
    match next {
        Some(index) => {
            *cursor = Some(index);
            Some(list[index].clone())
        }
        None if !back => {
            *cursor = None;
            Some(draft.clone())
        }
        None => None,
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

    #[test]
    fn command_names_options_and_values() {
        assert_eq!(candidates("wr"), (0, vec!["write".to_string()]));
        let (start, found) = candidates("set tab");
        assert_eq!(start, 4);
        assert_eq!(found, vec!["tabs".to_string(), "tabstop".to_string()]);
        let (start, found) = candidates("set ff=d");
        assert_eq!(start, 7);
        assert_eq!(found, vec!["dos".to_string()]);
    }

    #[test]
    fn paths_complete_with_a_separator_after_directories() {
        let root = std::env::temp_dir().join("termi-complete-paths");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("src")).expect("create fixture");
        std::fs::write(root.join("setup.cfg"), "").expect("write fixture");
        std::fs::write(root.join(".secret"), "").expect("write fixture");
        let base = format!("{}/", root.display());
        let found = paths(&format!("{base}s"));
        assert_eq!(
            found,
            vec![format!("{base}src/"), format!("{base}setup.cfg")]
        );
        assert!(paths(&base).iter().all(|path| !path.contains(".secret")));
    }

    #[test]
    fn tab_steps_through_and_back_to_what_was_typed() {
        let mut app = app();
        app.command_line = "set tab".to_string();
        step(&mut app, true);
        assert_eq!(app.command_line, "set tabs");
        step(&mut app, true);
        assert_eq!(app.command_line, "set tabstop");
        step(&mut app, true);
        assert_eq!(app.command_line, "set tab");
    }

    #[test]
    fn history_travels_through_matching_entries() {
        let list = vec!["e a.rs".to_string(), "w".to_string(), "e b.rs".to_string()];
        let (mut cursor, mut draft) = (None, String::new());
        assert_eq!(
            travel(&list, &mut cursor, &mut draft, "e", true),
            Some("e b.rs".into())
        );
        assert_eq!(
            travel(&list, &mut cursor, &mut draft, "e b.rs", true),
            Some("e a.rs".into())
        );
        assert_eq!(
            travel(&list, &mut cursor, &mut draft, "e a.rs", false),
            Some("e b.rs".into())
        );
        assert_eq!(
            travel(&list, &mut cursor, &mut draft, "e b.rs", false),
            Some("e".into())
        );
    }
}
