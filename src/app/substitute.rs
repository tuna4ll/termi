//! `:s`, with ranges and confirmation, and `:greplace` across the project.
//!
//! Every substitution is a single undo step, however many lines it touches —
//! and so is a confirmed one, however many questions it asked.

use std::path::Path;

use super::mode::Mode;
use super::state::App;
use crate::editor::command::{Address, Base, LineRange};
use crate::editor::cursor::Position;
use crate::editor::document::indent;
use crate::editor::edit::Edit;
use crate::editor::selection::Range;
use crate::editor::window::Window;
use crate::search::Matcher;

/// A `:s///c` waiting on the user, one match at a time.
#[derive(Debug)]
pub struct ReplaceSession {
    matcher: Matcher,
    replacement: String,
    all: bool,
    /// The line being looked at, and the column to look from.
    line: usize,
    col: usize,
    /// The last line in the range.
    last: usize,
    /// The match on show: its span on `line` and what it would become.
    current: Option<(usize, usize, String)>,
    /// How many replacements were made.
    replaced: usize,
}

/// The lines a range covers, `start <= end`, or what is wrong with it.
///
/// # Errors
/// Returns a message for a mark that is not set.
pub fn resolve_range(app: &App, range: Option<LineRange>) -> Result<(usize, usize), String> {
    let document = &app.buffer().document;
    let caret = app.window().cursor().head.line;
    let Some(range) = range else {
        return Ok((caret, caret));
    };
    let line = |address: Address| -> Result<usize, String> {
        let base = match address.base {
            Base::Line(line) => line.saturating_sub(1),
            Base::Current => caret,
            Base::Last => document.last_line(),
            Base::Mark(name) => app
                .buffer()
                .marks
                .get(&name)
                .map(|position| position.line)
                .ok_or_else(|| format!("mark {name} is not set"))?,
        };
        Ok(base
            .saturating_add_signed(address.offset)
            .min(document.last_line()))
    };
    let (start, end) = (line(range.start)?, line(range.end)?);
    Ok((start.min(end), start.max(end)))
}

/// Compile a pattern the way `:s` reads it: by the search settings, unless a
/// flag overrides the case.
fn matcher_for(app: &App, pattern: &str, case_sensitive: Option<bool>) -> Result<Matcher, String> {
    Matcher::new(
        pattern,
        app.search.regex,
        case_sensitive.or(app.search.case_sensitive),
    )
}

/// `:[range]s/pattern/replacement/[flags]`.
pub fn substitute(
    app: &mut App,
    pattern: &str,
    replacement: &str,
    flags: (bool, bool, Option<bool>),
    range: Option<LineRange>,
) {
    let (all, confirm, case_sensitive) = flags;
    let matcher = match matcher_for(app, pattern, case_sensitive) {
        Ok(matcher) => matcher,
        Err(error) => return app.error(error),
    };
    let (first, last) = match resolve_range(app, range) {
        Ok(lines) => lines,
        Err(error) => return app.error(error),
    };
    // The pattern becomes the search, so `n` finds what was just replaced and
    // the matches are highlighted while confirming.
    app.search.set_query(pattern.to_string());
    app.mode = Mode::Normal;

    if confirm {
        app.edit().begin_group();
        app.replace_session = Some(ReplaceSession {
            matcher,
            replacement: replacement.to_string(),
            all,
            line: first,
            col: 0,
            last,
            current: None,
            replaced: 0,
        });
        return advance(app);
    }

    let document = &app.buffer().document;
    let mut replacements = Vec::new();
    let mut count = 0;
    let mut last_changed = None;
    for line in first..=last {
        let text = document.line_string(line);
        let (replaced, found) = matcher.replace(&text, replacement, all);
        if found == 0 {
            continue;
        }
        let start = document.line_start(line);
        replacements.push((Range::new(start, start + text.chars().count()), replaced));
        count += found;
        last_changed = Some(line);
    }
    let Some(line) = last_changed else {
        return app.error(format!("no match for {pattern}"));
    };
    let mut edit = app.edit();
    edit.begin_group();
    edit.replace_many(&replacements);
    edit.end_group();
    let column = indent::first_non_blank(&app.buffer().document, line);
    app.window_mut().clear_secondary_cursors();
    app.window_mut()
        .cursor_mut()
        .move_to(Position::new(line, column), false);
    app.clamp_cursors(false);
    let lines = replacements.len();
    app.info(format!(
        "{count} replacement{} on {lines} line{}",
        if count == 1 { "" } else { "s" },
        if lines == 1 { "" } else { "s" }
    ));
}

/// Find the next match for a confirming session and ask about it, or finish.
fn advance(app: &mut App) {
    let Some(session) = app.replace_session.as_mut() else {
        return;
    };
    let document = &app.buffers[app.windows.focused().buffer].document;
    while session.line <= session.last.min(document.last_line()) {
        let text = document.line_string(session.line);
        if let Some(found) = session
            .matcher
            .next_match(&text, session.col, &session.replacement)
        {
            let position = Position::new(session.line, found.0);
            session.current = Some(found);
            app.mode = Mode::Replace;
            app.window_mut().clear_secondary_cursors();
            app.window_mut().cursor_mut().move_to(position, false);
            let replacement = app
                .replace_session
                .as_ref()
                .and_then(|session| session.current.as_ref())
                .map(|(_, _, text)| text.clone())
                .unwrap_or_default();
            app.info(format!(
                "replace with {replacement:?}? y/n  a all  l last  q quit"
            ));
            return;
        }
        session.line += 1;
        session.col = 0;
    }
    finish(app);
}

/// The user's answer to one `:s///c` question.
pub fn answer(app: &mut App, answer: char) {
    match answer {
        'y' | 'l' => {
            replace_current(app);
            if answer == 'l' {
                return finish(app);
            }
            advance(app);
        }
        'n' => {
            if let Some(session) = app.replace_session.as_mut()
                && let Some((_, end, _)) = session.current.take()
            {
                step_past(session, end);
            }
            advance(app);
        }
        'a' => {
            while app
                .replace_session
                .as_ref()
                .is_some_and(|session| session.current.is_some())
            {
                replace_current(app);
                advance(app);
            }
        }
        _ => finish(app),
    }
}

/// Move a session past the match that ended at `end` — on along the line
/// with `g`, on to the next line without it.
fn step_past(session: &mut ReplaceSession, end: usize) {
    if session.all {
        session.col = end;
    } else {
        session.line += 1;
        session.col = 0;
    }
}

fn replace_current(app: &mut App) {
    let index = app.windows.focused().buffer;
    let Some(session) = app.replace_session.as_mut() else {
        return;
    };
    let Some((start, end, text)) = session.current.take() else {
        return;
    };
    let document = &app.buffers[index].document;
    let line_start = document.line_start(session.line);
    let length = text.chars().count();
    session.replaced += 1;
    step_past(session, start + length);
    // Replacing nothing with nothing would find the same empty spot forever.
    if !session.all && start == end && length == 0 {
        return;
    }
    app.edit()
        .replace(line_start + start, line_start + end, &text);
}

fn finish(app: &mut App) {
    let Some(session) = app.replace_session.take() else {
        return;
    };
    app.edit().end_group();
    app.mode = Mode::Normal;
    app.clamp_cursors(false);
    let count = session.replaced;
    app.info(format!(
        "{count} replacement{}",
        if count == 1 { "" } else { "s" }
    ));
}

/// `:greplace/pattern/replacement/` — replace in every file of the project.
///
/// Changed files are opened, edited as one undo step each, and left unsaved,
/// so the result can be read through before `:wa` makes it permanent.
pub fn project_replace(
    app: &mut App,
    pattern: &str,
    replacement: &str,
    case_sensitive: Option<bool>,
) {
    let matcher = match matcher_for(app, pattern, case_sensitive) {
        Ok(matcher) => matcher,
        Err(error) => return app.error(error),
    };
    let root = std::env::current_dir().unwrap_or_else(|_| ".".into());
    let mut files = 0;
    let mut total = 0;
    for path in crate::picker::project_files(&root) {
        let already_open = app.index_of(&path).is_some();
        if !already_open
            && crate::search::project::text_of(&path)
                .is_none_or(|text| text.lines().all(|line| matcher.find_all(line).is_empty()))
        {
            continue;
        }
        let count = match replace_in_file(app, &path, &matcher, replacement) {
            Ok(count) => count,
            Err(error) => {
                app.error(error);
                return;
            }
        };
        if count > 0 {
            files += 1;
            total += count;
        }
    }
    if total == 0 {
        app.error(format!("no match for {pattern} in the project"));
    } else {
        app.info(format!(
            "{total} replacement{} in {files} file{} — :wa writes them",
            if total == 1 { "" } else { "s" },
            if files == 1 { "" } else { "s" }
        ));
    }
}

fn replace_in_file(
    app: &mut App,
    path: &Path,
    matcher: &Matcher,
    replacement: &str,
) -> Result<usize, String> {
    let index = app
        .load(path.to_path_buf())
        .map_err(|error| error.to_string())?;
    let buffer = &mut app.buffers[index];
    let document = &buffer.document;
    let mut replacements = Vec::new();
    let mut count = 0;
    for line in 0..document.len_lines() {
        let text = document.line_string(line);
        let (replaced, found) = matcher.replace(&text, replacement, true);
        if found > 0 {
            let start = document.line_start(line);
            replacements.push((Range::new(start, start + text.chars().count()), replaced));
            count += found;
        }
    }
    if count == 0 {
        return Ok(0);
    }
    // The buffer may not be on screen; a throwaway window gives the edit a
    // caret to record without disturbing any real one.
    let mut window = Window::new(index);
    let mut edit = Edit::new(buffer, &mut window);
    edit.begin_group();
    edit.replace_many(&replacements);
    edit.end_group();
    app.clamp_windows_on(index);
    Ok(count)
}
