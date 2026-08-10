//! # Action dispatch
//!
//! **Purpose:** carry out what the input layer decided.
//!
//! **Responsibility:** the single `match` from [`Action`] to an effect on the
//! application. Every feature the editor has passes through here exactly once,
//! which makes this file the place to look when asking "what does this key
//! actually do?".
//!
//! Dispatch owns the *policy* around edits — when to close an undo step, when a
//! mode change implies clamping the cursor, whether an operation applies to a
//! selection or to the current line. The mechanics live in the editor layer.
//!
//! **Public API:** [`apply`].

use anyhow::Result;

use super::commands;
use super::mode::Mode;
use super::state::App;
use crate::editor::cursor::Motion;
use crate::editor::document::indent;
use crate::editor::selection::Range;
use crate::editor::window::WindowId;
use crate::input::Action;

/// Apply one action to the editor.
///
/// # Errors
/// Returns an error only for failures the user must see and that leave the
/// editor in a valid state, such as a failed save.
pub fn apply(app: &mut App, action: Action) -> Result<()> {
    // A popup is modal: it swallows the keystroke that dismisses it, so the key
    // that closes it cannot also edit the buffer underneath.
    if app.popup.is_some() && !matches!(action, Action::None) {
        app.popup = None;
        return Ok(());
    }
    // Any deliberate keystroke dismisses the previous message; keeping it around
    // makes it ambiguous which action it refers to.
    if !matches!(action, Action::None) {
        app.clear_status();
    }

    match action {
        Action::None => {}

        Action::EnterMode(mode) => enter_mode(app, mode),
        Action::Move(motion) => move_cursors(app, motion, false),
        Action::Extend(motion) => move_cursors(app, motion, true),
        Action::Scroll(delta) => {
            let focus = app.windows.focus();
            app.scroll_window(focus, delta);
        }
        Action::Page { down, half } => page(app, down, half),

        Action::Insert(ch) => insert_char(app, ch),
        Action::InsertNewline => {
            let (mut edit, config) = app.edit_and_config();
            edit.insert_newline(config);
        }
        Action::InsertIndent => {
            let (mut edit, config) = app.edit_and_config();
            edit.insert_indent(config);
        }
        Action::DeleteBackward => {
            let (mut edit, config) = app.edit_and_config();
            edit.delete_backward(config);
        }
        Action::DeleteForward => app.edit().delete_forward(),
        Action::Delete => delete_target(app),

        Action::OpenLineBelow => open_line(app, true),
        Action::OpenLineAbove => open_line(app, false),
        Action::AppendAfter => {
            enter_mode(app, Mode::Insert);
            move_cursors(app, Motion::Right, false);
        }
        Action::AppendAtLineEnd => {
            enter_mode(app, Mode::Insert);
            move_cursors(app, Motion::LineEnd, false);
        }
        Action::InsertAtLineStart => {
            enter_mode(app, Mode::Insert);
            move_cursors(app, Motion::LineFirstNonBlank, false);
        }

        Action::Undo => {
            if !app.edit().undo() {
                app.info("already at the oldest change");
            }
        }
        Action::Redo => {
            if !app.edit().redo() {
                app.info("already at the newest change");
            }
        }

        Action::Yank => yank(app, false),
        Action::Cut => yank(app, true),
        Action::Paste => paste(app),

        Action::AddCursor { below } => add_cursor(app, below),
        Action::ClearCursors => {
            app.window_mut().clear_secondary_cursors();
            app.window_mut().collapse_selections();
        }

        Action::CycleBuffer { forward } => app.cycle_buffer(forward),
        Action::Save => save(app),
        Action::Quit => quit(app),

        Action::SplitWindow { axis } => {
            app.windows.split(axis);
        }
        Action::CloseWindow => close_window(app),
        Action::OnlyWindow => app.windows.close_others(),
        Action::FocusWindow(side) => {
            if let Some(id) = app.windows.neighbour(side) {
                focus_window(app, id);
            }
        }
        Action::CycleWindow => cycle_window(app),
        Action::ResizeWindow { axis, delta } => app.windows.resize(axis, delta),
        Action::EqualiseWindows => app.windows.equalise(),
        Action::FocusAt { x, y } => {
            if let Some(id) = app.windows.at(x, y) {
                focus_window(app, id);
            }
        }
        Action::ScrollAt { x, y, delta } => {
            if let Some(id) = app.windows.at(x, y) {
                app.scroll_window(id, delta);
            }
        }

        Action::ToggleTree => toggle_tree(app),
        Action::TreeMove(delta) => move_tree_selection(app, delta),
        Action::TreeActivate => activate_tree_entry(app),

        Action::SearchStart { forward } => {
            let origin = origin_offset(app);
            app.search.begin(origin, forward);
            enter_mode(app, Mode::Search);
        }
        Action::SearchInput(ch) => {
            app.search.push(ch);
            seek_from_origin(app);
        }
        Action::SearchBackspace => {
            if app.search.query.is_empty() {
                return cancel_search(app);
            }
            app.search.pop();
            seek_from_origin(app);
        }
        Action::SearchSubmit => {
            if app.search.is_active() {
                let count = app.search.count(&app.buffer().document, MATCH_COUNT_CAP);
                app.info(format!(
                    "{count} match{}",
                    if count == 1 { "" } else { "es" }
                ));
            }
            enter_mode(app, Mode::Normal);
        }
        Action::SearchCancel => return cancel_search(app),
        Action::SearchRepeat { forward } => repeat_search(app, forward),

        Action::CommandInput(ch) => app.command_line.push(ch),
        Action::CommandBackspace => {
            app.command_line.pop();
            if app.command_line.is_empty() {
                // Backspacing past the `:` leaves command mode, as in vi.
                enter_mode(app, Mode::Normal);
            }
        }
        Action::CommandCancel => {
            app.command_line.clear();
            enter_mode(app, Mode::Normal);
        }
        Action::CommandSubmit => {
            // Take the line before leaving the mode: `enter_mode` clears it.
            let line = std::mem::take(&mut app.command_line);
            enter_mode(app, Mode::Normal);
            commands::run(app, &line);
        }
    }
    Ok(())
}

/// Insert one typed character, re-indenting first when it closes a block.
///
/// Typing `}` on an otherwise blank, indented line pulls the line back one level
/// so the bracket lines up with its opener — the one piece of "smart" behaviour
/// that a per-line editor can get right without a parser.
fn insert_char(app: &mut App, ch: char) {
    let (mut edit, config) = app.edit_and_config();
    let head = edit.window.cursor().head;

    if config.auto_indent && indent::should_dedent(&edit.buffer.document, head.line, head.col, ch) {
        edit.delete_backward(config);
    }
    edit.insert_text(&ch.to_string());
}

/// Upper bound on how many matches `:search` will count.
///
/// Counting is O(document); on a file with millions of hits the exact number is
/// not useful anyway, so it stops early and reports the cap.
const MATCH_COUNT_CAP: usize = 10_000;

/// Character offset of the primary caret.
fn origin_offset(app: &App) -> usize {
    app.buffer()
        .document
        .pos_to_char(app.window().cursor().head)
}

/// Jump to the first match at or after where the search started.
///
/// Searching from the origin rather than from the current match is what makes
/// deleting a character in the query walk *backwards* through the file instead
/// of leaving the caret stranded further down.
fn seek_from_origin(app: &mut App) {
    let origin = app.search.origin();
    let forward = app.search.forward;
    let Some(found) = app.search.find(&app.buffer().document, origin, forward) else {
        return;
    };
    let position = app.buffer().document.char_to_pos(found.start);
    app.window_mut().cursor_mut().move_to(position, false);
}

/// Leave search mode and put the caret back where it started.
fn cancel_search(app: &mut App) -> Result<()> {
    let origin = app.search.origin();
    let position = app.buffer().document.char_to_pos(origin);
    app.window_mut().cursor_mut().move_to(position, false);
    app.search.set_query(String::new());
    enter_mode(app, Mode::Normal);
    Ok(())
}

/// Jump to the next or previous match of the current query.
fn repeat_search(app: &mut App, forward: bool) {
    if !app.search.is_active() {
        app.info("no previous search");
        return;
    }
    let from = origin_offset(app);
    let Some(found) = app.search.find(&app.buffer().document, from, forward) else {
        app.error(format!("no match for {}", app.search.query));
        return;
    };
    let position = app.buffer().document.char_to_pos(found.start);
    app.window_mut().cursor_mut().move_to(position, false);
}

/// Close the focused window, keeping its buffer open.
///
/// Nothing is at risk here: the buffer stays in the list whether or not it was
/// saved, so unlike `:q` this never needs a `!`.
fn close_window(app: &mut App) {
    let focus = app.windows.focus();
    if !app.windows.close(focus) {
        app.info("only one window");
    }
}

/// Hand the keyboard to another window.
fn focus_window(app: &mut App, id: WindowId) {
    if id == app.windows.focus() {
        return;
    }
    // Settle the window being left first: a selection, and a caret sitting past
    // the end of a line, both belong to the mode it is being left in.
    enter_mode(app, Mode::Normal);
    app.windows.set_focus(id);
}

/// Move the focus to the next window in screen order, wrapping around.
fn cycle_window(app: &mut App) {
    let order = app.windows.ids();
    let current = order
        .iter()
        .position(|id| *id == app.windows.focus())
        .unwrap_or(0);
    let next = order[(current + 1) % order.len()];
    focus_window(app, next);
}

/// Show or hide the file tree, focusing it when it appears.
///
/// The tree is built on first use and then kept: rebuilding it would collapse
/// every directory the user had opened.
fn toggle_tree(app: &mut App) {
    app.tree_visible = !app.tree_visible;
    if !app.tree_visible {
        enter_mode(app, Mode::Normal);
        return;
    }
    if app.tree.is_none() {
        let root = app.tree_root();
        app.tree = Some(crate::filesystem::tree::Tree::new(root));
    }
    enter_mode(app, Mode::Tree);
}

/// Move the tree selection, clamped to the list.
fn move_tree_selection(app: &mut App, delta: isize) {
    let Some(tree) = app.tree.as_ref() else {
        return;
    };
    let last = tree.entries().len().saturating_sub(1);
    app.tree_selected = match delta {
        isize::MIN => 0,
        isize::MAX => last,
        delta if delta < 0 => app.tree_selected.saturating_sub(delta.unsigned_abs()),
        delta => app
            .tree_selected
            .saturating_add(delta.unsigned_abs())
            .min(last),
    };
}

/// Expand a directory, or open a file and hand the keyboard back to the editor.
fn activate_tree_entry(app: &mut App) {
    let Some(tree) = app.tree.as_mut() else {
        return;
    };
    let Some(path) = tree.activate(app.tree_selected) else {
        // A directory was expanded or collapsed; the selection may now be past
        // the end of a shorter list.
        let last = app
            .tree
            .as_ref()
            .map_or(0, |t| t.entries().len().saturating_sub(1));
        app.tree_selected = app.tree_selected.min(last);
        return;
    };
    match app.open(path) {
        Ok(()) => {
            app.tree_visible = false;
            enter_mode(app, Mode::Normal);
        }
        Err(error) => app.error(error.to_string()),
    }
}

/// Switch modes, applying the invariants each mode requires.
fn enter_mode(app: &mut App, mode: Mode) {
    if app.mode == mode {
        return;
    }
    // A mode change always ends an undo step: undoing should return to the
    // state before this burst of typing, not the middle of it.
    app.edit().checkpoint();

    match mode {
        Mode::Normal => {
            app.window_mut().collapse_selections();
            // Normal mode's caret sits *on* a character, so a caret parked past
            // the end of a line in insert mode has to come back.
            app.clamp_cursors(false);
        }
        Mode::Visual | Mode::VisualLine => app.window_mut().anchor_selections(),
        Mode::Command => app.command_line.clear(),
        Mode::Insert | Mode::Search | Mode::Tree => {}
    }
    app.mode = mode;
}

/// Move every cursor, extending the selection when asked.
fn move_cursors(app: &mut App, motion: Motion, extend: bool) {
    let allow_eol = app.mode.is_insert();
    let extend = extend || app.mode.is_visual();
    app.edit().checkpoint();
    app.move_cursors(motion, extend, allow_eol);
}

/// Move a whole or half screen.
fn page(app: &mut App, down: bool, half: bool) {
    let height = usize::from(app.window().area.height).max(1);
    let distance = if half { height / 2 } else { height }.max(1);
    let motion = if down {
        Motion::Down(distance)
    } else {
        Motion::Up(distance)
    };
    move_cursors(app, motion, false);
}

/// The span an operator applies to: the selection in visual mode, the current
/// line otherwise.
fn target_range(app: &App) -> (Range, bool) {
    let document = &app.buffer().document;
    let cursor = app.window().cursor();
    match app.mode {
        Mode::Visual => (Range::of(&cursor, document), false),
        _ => (Range::of_lines(&cursor, document), true),
    }
}

fn delete_target(app: &mut App) {
    let (range, _) = target_range(app);
    app.edit().delete_range(range);
    enter_mode(app, Mode::Normal);
}

fn yank(app: &mut App, cut: bool) {
    let (range, line_wise) = target_range(app);
    let text = app.buffer().document.slice_string(range.start, range.end);
    if text.is_empty() {
        return;
    }
    let lines = text.lines().count();
    app.clipboard.set(text, line_wise);

    if cut {
        app.edit().delete_range(range);
    }
    enter_mode(app, Mode::Normal);
    app.info(format!(
        "{} {lines} line{}",
        if cut { "cut" } else { "yanked" },
        if lines == 1 { "" } else { "s" }
    ));
}

fn paste(app: &mut App) {
    let text = app.clipboard.get();
    if text.is_empty() {
        app.info("clipboard is empty");
        return;
    }
    let line_wise = app.clipboard.is_line_wise();
    app.edit().paste(&text, line_wise);
    enter_mode(app, Mode::Normal);
}

/// Insert an empty line and start typing on it.
fn open_line(app: &mut App, below: bool) {
    enter_mode(app, Mode::Insert);
    // Splitting at the first non-blank character rather than at column zero is
    // what lets the new line inherit the current indentation.
    let motion = if below {
        Motion::LineEnd
    } else {
        Motion::LineFirstNonBlank
    };
    app.move_cursors(motion, false, true);

    let (mut edit, config) = app.edit_and_config();
    edit.insert_newline(config);
    if !below {
        // The split pushed the original text down; step back onto the blank
        // line that is now above it.
        edit.window
            .move_cursors(Motion::Up(1), &edit.buffer.document, false, true);
    }
}

/// Duplicate the primary cursor onto the neighbouring line.
fn add_cursor(app: &mut App, below: bool) {
    let mut cursor = app.window().cursor();
    let last = app.buffer().document.last_line();

    let line = if below {
        (cursor.head.line + 1).min(last)
    } else {
        cursor.head.line.saturating_sub(1)
    };
    if line == cursor.head.line {
        return;
    }
    let position = app.buffer().document.clamp(
        crate::editor::cursor::Position::new(line, cursor.goal_col()),
        false,
    );
    cursor.move_to(position, false);
    app.window_mut().add_cursor(cursor);
}

fn save(app: &mut App) {
    if app.config.trim_trailing_whitespace {
        app.edit().trim_trailing_whitespace();
    }
    match app.buffer_mut().document.save() {
        Ok(()) => {
            let name = app.buffer().document.display_name().to_string();
            app.info(format!("wrote {name}"));
        }
        Err(error) => app.error(error.to_string()),
    }
}

fn quit(app: &mut App) {
    if app.has_unsaved_changes() {
        app.error("unsaved changes — use :q! to discard them");
    } else {
        app.quit();
    }
}
