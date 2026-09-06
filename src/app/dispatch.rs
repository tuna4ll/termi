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
use crate::ui::widgets::editor_view;

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
        Action::Select(motion) => move_cursors(app, motion, true),
        Action::SelectAll => select_all(app),
        Action::Scroll(delta) => {
            let focus = app.windows.focus();
            app.scroll_window(focus, delta);
        }
        Action::Page { down, half } => page(app, down, half),

        Action::Insert(ch) => insert_char(app, ch),
        Action::InsertNewline => {
            let (mut edit, config) = app.edit_and_config();
            edit.delete_selections();
            edit.insert_newline(config);
        }
        Action::InsertIndent => {
            let (mut edit, config) = app.edit_and_config();
            edit.delete_selections();
            edit.insert_indent(config);
        }
        // Backspace and Delete take the selection when there is one, and fall
        // back to a single character when there is not.
        Action::DeleteBackward => {
            let (mut edit, config) = app.edit_and_config();
            if !edit.delete_selections() {
                edit.delete_backward(config);
            }
        }
        Action::DeleteForward => {
            let mut edit = app.edit();
            if !edit.delete_selections() {
                edit.delete_forward();
            }
        }
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
        Action::ClickAt { x, y } => click(app, x, y),
        Action::DragTo { x, y } => drag(app, x, y),
        Action::ScrollAt { x, y, delta } => {
            if let Some(id) = app.windows.at(x, y) {
                app.scroll_window(id, delta);
            }
        }

        Action::ToggleTree => toggle_tree(app),
        Action::TreeMove(delta) => move_tree_selection(app, delta),
        Action::TreeActivate => activate_tree_entry(app),
        Action::TreeCreate { directory } => begin_tree_creation(app, directory),

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
///
/// Whether the character also brings a closing half with it is decided per
/// cursor, from the text around it, by the editor layer.
fn insert_char(app: &mut App, ch: char) {
    let (mut edit, config) = app.edit_and_config();
    // Typing over a selection replaces it, so the character has to land where
    // the selection was rather than beside it.
    edit.delete_selections();
    let head = edit.window.cursor().head;

    if config.auto_indent && indent::should_dedent(&edit.buffer.document, head.line, head.col, ch) {
        edit.delete_backward(config);
    }
    edit.insert_char(ch, config);
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

/// Focus the window under the pointer and put the caret where it points.
fn click(app: &mut App, x: u16, y: u16) {
    let Some(id) = app.windows.at(x, y) else {
        return;
    };
    if id != app.windows.focus() {
        focus_window(app, id);
    } else if app.mode.is_visual() {
        // A fresh click drops the selection it lands in. Insert mode is left
        // alone on purpose: clicking elsewhere while typing should move the
        // caret, not throw the user out of the mode.
        enter_mode(app, Mode::Normal);
    }
    place_caret(app, x, y, false);
}

/// Extend the selection to wherever the pointer has been dragged.
fn drag(app: &mut App, x: u16, y: u16) {
    // A drag that leaves the window keeps selecting along the edge it left by,
    // rather than stopping dead or jumping into the neighbouring window.
    let area = app.window().area;
    let x = x.clamp(area.x, area.right().saturating_sub(1).max(area.x));
    let y = y.clamp(area.y, area.bottom().saturating_sub(1).max(area.y));
    place_caret(app, x, y, true);
}

/// Move the primary caret onto the character drawn at a screen cell.
fn place_caret(app: &mut App, x: u16, y: u16, extend: bool) {
    let window = app.window();
    let position = editor_view::position_at(
        &app.buffers[window.buffer],
        window,
        &app.config,
        window.area,
        x,
        y,
    );
    let Some(position) = position else {
        return;
    };
    let allow_eol = app.mode.is_insert() || (extend && !app.mode.is_visual());
    let position = app.buffer().document.clamp(position, allow_eol);

    app.edit().checkpoint();
    app.window_mut().clear_secondary_cursors();
    app.window_mut().cursor_mut().move_to(position, extend);
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

/// Open the command line with the selected entry's directory already filled in.
fn begin_tree_creation(app: &mut App, directory: bool) {
    let Some(tree) = app.tree.as_ref() else {
        return;
    };
    let parent = tree.creation_directory(app.tree_selected);
    let command = if directory { "mkdir" } else { "touch" };
    let separator = std::path::MAIN_SEPARATOR;
    let mut prefix = parent.as_os_str().to_string_lossy().into_owned();
    if !prefix.ends_with(separator) {
        prefix.push(separator);
    }

    enter_mode(app, Mode::Command);
    app.command_line = format!("{command} {prefix}");
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
        Mode::Visual | Mode::VisualLine => {
            // A caret parked past the end of a line by insert mode has to come
            // back before it anchors a selection out there.
            app.clamp_cursors(false);
            app.window_mut().anchor_selections();
        }
        Mode::Command => app.command_line.clear(),
        // Typing starts at a point, so `i` after selecting puts the caret where
        // the selection ended rather than keeping an invisible one alive.
        Mode::Insert => app.window_mut().collapse_selections(),
        Mode::Search | Mode::Tree => {}
    }
    app.mode = mode;
}

/// Move every cursor, extending the selection when asked.
///
/// A motion that does not extend collapses the selection onto where it lands,
/// which is the whole of "click somewhere else and the selection goes away".
fn move_cursors(app: &mut App, motion: Motion, extend: bool) {
    let extend = extend || app.mode.is_visual();
    // A modeless selection puts its head where a bar cursor would go, so it has
    // to be able to reach past the last character of a line — otherwise
    // Shift+End would stop one short of the end. Visual mode's caret always
    // covers a real character, and insert mode's may always sit past one.
    let allow_eol = app.mode.is_insert() || (extend && !app.mode.is_visual());
    app.edit().checkpoint();
    app.move_cursors(motion, extend, allow_eol);
}

/// Select the whole buffer, without leaving the current mode.
///
/// The head lands past the final character, where a bar cursor sits, so the
/// selection covers the last one rather than stopping short of it. Extra
/// cursors go: "everything" is one span, and leaving three carets behind would
/// mean the next keystroke edited it in three places.
fn select_all(app: &mut App) {
    let document = &app.buffer().document;
    let start = document.char_to_pos(0);
    let end = document.char_to_pos(document.len_chars());

    app.edit().checkpoint();
    app.window_mut().clear_secondary_cursors();
    let cursor = app.window_mut().cursor_mut();
    cursor.move_to(start, false);
    cursor.move_to(end, true);
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

/// The span an operator applies to, and whether it is a whole-line one.
///
/// A standing selection wins wherever there is one — that is what makes `d`,
/// `y` and Ctrl+C act on what the user dragged out. With nothing selected the
/// operator falls back to the current line, the way vi's `dd` and `yy` do.
fn target_range(app: &App) -> (Range, bool) {
    let document = &app.buffer().document;
    let cursor = app.window().cursor();
    match app.mode {
        Mode::Visual => (Range::of(&cursor, document), false),
        Mode::VisualLine => (Range::of_lines(&cursor, document), true),
        _ if cursor.has_selection() => (Range::between(&cursor, document), false),
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
    let characters = text.chars().count();
    app.clipboard.set(text, line_wise);

    if cut {
        app.edit().delete_range(range);
    }
    enter_mode(app, Mode::Normal);

    // A line count is the useful measure for a whole-line operation and a
    // misleading one for three characters out of the middle of a line.
    let verb = if cut { "cut" } else { "copied" };
    app.info(if line_wise {
        format!("{verb} {lines} line{}", if lines == 1 { "" } else { "s" })
    } else {
        format!(
            "{verb} {characters} character{}",
            if characters == 1 { "" } else { "s" }
        )
    });
}

fn paste(app: &mut App) {
    let text = app.clipboard.get();
    if text.is_empty() {
        app.info("clipboard is empty");
        return;
    }
    // Pasting over a selection replaces it, which is the other half of being
    // able to select something and type over it.
    app.edit().delete_selections();
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::editor::buffer::Buffer;
    use crate::editor::cursor::Position;
    use crate::editor::document::Document;
    use crate::editor::window::{Area, Axis, Side};

    /// An editor that reads neither the installed configuration nor the system
    /// clipboard, so these behave the same on every machine.
    fn app() -> App {
        App::with_config(Config {
            system_clipboard: false,
            ..Config::default()
        })
    }

    fn press(app: &mut App, action: Action) {
        apply(app, action).expect("dispatching should not fail");
    }

    fn type_text(app: &mut App, text: &str) {
        press(app, Action::EnterMode(Mode::Insert));
        for ch in text.chars() {
            if ch == '\n' {
                press(app, Action::InsertNewline);
            } else {
                press(app, Action::Insert(ch));
            }
        }
        press(app, Action::EnterMode(Mode::Normal));
    }

    fn text_of(app: &App) -> String {
        app.buffer().document.text().to_string()
    }

    /// Width of the line-number gutter at the sizes these tests use.
    const GUTTER: u16 = 5;

    /// Draw one frame so the windows learn their areas, and return the focused
    /// one's. A click means nothing until the editor has been rendered once:
    /// only a frame records where each window went.
    fn clicked_area(app: &mut App) -> Area {
        let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(60, 12))
            .expect("the test backend always builds");
        terminal
            .draw(|frame| crate::ui::draw(frame, app))
            .expect("drawing into the test backend should not fail");
        app.window().area
    }

    /// The window that is not the focused one.
    fn other_window(app: &App) -> WindowId {
        app.windows
            .ids()
            .into_iter()
            .find(|id| *id != app.windows.focus())
            .expect("there should be a second window")
    }

    /// Put two windows side by side with the areas a render would have given
    /// them, which is what the geometry-driven commands read.
    fn side_by_side(app: &mut App) -> (WindowId, WindowId) {
        let right = app.windows.split(Axis::Horizontal);
        let left = app.windows.ids()[0];
        app.windows.get_mut(left).area = Area::new(0, 0, 40, 20);
        app.windows.get_mut(right).area = Area::new(41, 0, 40, 20);
        (left, right)
    }

    #[test]
    fn selecting_with_shift_stays_in_the_mode_it_started_in() {
        let mut app = app();
        type_text(&mut app, "hello");
        press(&mut app, Action::Move(Motion::LineStart));

        press(&mut app, Action::Select(Motion::Right));
        assert_eq!(app.mode, Mode::Normal, "selecting is not a mode change");
        assert!(app.has_selection());

        press(&mut app, Action::Select(Motion::Right));
        let cursor = app.window().cursor();
        assert_eq!(cursor.anchor.col, 0, "the anchor stays where it started");
        assert_eq!(cursor.head.col, 2);
    }

    #[test]
    fn a_shifted_arrow_selects_exactly_one_character() {
        let mut app = app();
        type_text(&mut app, "hello");
        press(&mut app, Action::Move(Motion::LineStart));
        press(&mut app, Action::Select(Motion::Right));
        press(&mut app, Action::DeleteBackward);

        assert_eq!(text_of(&app), "ello");
        assert_eq!(app.mode, Mode::Normal);
    }

    #[test]
    fn backspace_takes_the_whole_selection() {
        let mut app = app();
        type_text(&mut app, "hello");
        press(&mut app, Action::Move(Motion::LineStart));
        for _ in 0..3 {
            press(&mut app, Action::Select(Motion::Right));
        }
        press(&mut app, Action::DeleteBackward);

        assert_eq!(text_of(&app), "lo");
        assert!(!app.has_selection(), "the selection is gone with the text");
    }

    #[test]
    fn delete_forward_takes_the_selection_before_the_character() {
        let mut app = app();
        type_text(&mut app, "hello");
        press(&mut app, Action::Move(Motion::LineStart));
        press(&mut app, Action::Select(Motion::Right));
        press(&mut app, Action::Select(Motion::Right));
        press(&mut app, Action::DeleteForward);

        assert_eq!(text_of(&app), "llo");
    }

    #[test]
    fn a_selection_can_run_to_the_end_of_a_line() {
        let mut app = app();
        type_text(&mut app, "hello");
        press(&mut app, Action::Move(Motion::LineStart));
        press(&mut app, Action::Select(Motion::LineEnd));
        press(&mut app, Action::DeleteBackward);

        // The head has to be able to rest past the last character, or Shift+End
        // would leave the final one behind.
        assert_eq!(text_of(&app), "");
    }

    #[test]
    fn typing_replaces_the_selection() {
        let mut app = app();
        type_text(&mut app, "hello");
        press(&mut app, Action::EnterMode(Mode::Insert));
        press(&mut app, Action::Move(Motion::LineStart));
        for _ in 0..4 {
            press(&mut app, Action::Select(Motion::Right));
        }
        press(&mut app, Action::Insert('y'));

        assert_eq!(text_of(&app), "yo");
        assert_eq!(app.mode, Mode::Insert);
    }

    #[test]
    fn entering_insert_mode_drops_the_selection_rather_than_replacing_it() {
        let mut app = app();
        type_text(&mut app, "hello");
        press(&mut app, Action::Move(Motion::LineStart));
        press(&mut app, Action::Select(Motion::Right));
        // `i` is the modal key for "insert where the caret is", and it keeps
        // that meaning: replacing a selection is what typing over it does.
        press(&mut app, Action::EnterMode(Mode::Insert));
        press(&mut app, Action::Insert('y'));

        assert_eq!(text_of(&app), "hyello");
    }

    #[test]
    fn selecting_in_insert_mode_does_not_leave_it() {
        let mut app = app();
        type_text(&mut app, "ab");
        press(&mut app, Action::EnterMode(Mode::Insert));
        press(&mut app, Action::Move(Motion::LineEnd));
        press(&mut app, Action::Select(Motion::Left));

        assert_eq!(app.mode, Mode::Insert);
        let cursor = app.window().cursor();
        assert_eq!(cursor.anchor, Position::new(0, 2), "anchored past the end");
        assert_eq!(cursor.head, Position::new(0, 1));
    }

    #[test]
    fn select_all_covers_the_whole_buffer() {
        let mut app = app();
        type_text(&mut app, "alpha\nbravo\ncharlie");
        press(&mut app, Action::Move(Motion::DocStart));

        press(&mut app, Action::SelectAll);
        assert_eq!(app.mode, Mode::Normal, "selecting all is not a mode change");

        press(&mut app, Action::DeleteBackward);
        assert_eq!(text_of(&app), "");
    }

    #[test]
    fn select_all_reaches_the_very_last_character() {
        let mut app = app();
        type_text(&mut app, "abc");
        press(&mut app, Action::SelectAll);
        press(&mut app, Action::Yank);

        assert_eq!(app.clipboard.get(), "abc");
    }

    #[test]
    fn select_all_collapses_to_a_single_cursor() {
        let mut app = app();
        type_text(&mut app, "one\ntwo");
        press(&mut app, Action::Move(Motion::DocStart));
        press(&mut app, Action::AddCursor { below: true });
        assert_eq!(app.window().cursors().len(), 2);

        press(&mut app, Action::SelectAll);
        assert_eq!(app.window().cursors().len(), 1);

        press(&mut app, Action::DeleteBackward);
        assert_eq!(text_of(&app), "");
    }

    #[test]
    fn select_all_in_insert_mode_can_be_typed_over() {
        let mut app = app();
        type_text(&mut app, "throw this away");
        press(&mut app, Action::EnterMode(Mode::Insert));
        press(&mut app, Action::SelectAll);
        press(&mut app, Action::Insert('x'));

        assert_eq!(text_of(&app), "x");
    }

    #[test]
    fn a_plain_motion_drops_the_selection() {
        let mut app = app();
        type_text(&mut app, "hello");
        press(&mut app, Action::Move(Motion::LineStart));
        press(&mut app, Action::Select(Motion::Right));
        press(&mut app, Action::Move(Motion::Right));

        assert!(!app.has_selection());
        press(&mut app, Action::DeleteBackward);
        assert_eq!(text_of(&app), "hllo", "one character, not the selection");
    }

    #[test]
    fn copying_a_selection_takes_the_characters_not_the_line() {
        let mut app = app();
        type_text(&mut app, "hello world");
        press(&mut app, Action::Move(Motion::LineStart));
        for _ in 0..5 {
            press(&mut app, Action::Select(Motion::Right));
        }
        press(&mut app, Action::Yank);

        assert_eq!(app.clipboard.get(), "hello");
        assert!(!app.clipboard.is_line_wise());
        assert_eq!(text_of(&app), "hello world", "copying changes nothing");
    }

    #[test]
    fn cutting_and_pasting_moves_a_selection() {
        let mut app = app();
        type_text(&mut app, "hello world");
        press(&mut app, Action::Move(Motion::LineStart));
        for _ in 0..6 {
            press(&mut app, Action::Select(Motion::Right));
        }
        press(&mut app, Action::Cut);
        assert_eq!(text_of(&app), "world");

        press(&mut app, Action::Move(Motion::LineEnd));
        press(&mut app, Action::EnterMode(Mode::Insert));
        press(&mut app, Action::Move(Motion::LineEnd));
        press(&mut app, Action::Paste);
        assert_eq!(text_of(&app), "worldhello ");
    }

    #[test]
    fn pasting_over_a_selection_replaces_it() {
        let mut app = app();
        type_text(&mut app, "one two");
        app.clipboard.set("three".to_string(), false);
        press(&mut app, Action::Move(Motion::LineStart));
        for _ in 0..3 {
            press(&mut app, Action::Select(Motion::Right));
        }
        press(&mut app, Action::Paste);

        assert_eq!(text_of(&app), "three two");
    }

    #[test]
    fn with_nothing_selected_a_copy_still_takes_the_line() {
        let mut app = app();
        type_text(&mut app, "alpha\nbravo");
        press(&mut app, Action::Yank);

        assert!(app.clipboard.is_line_wise());
        assert_eq!(app.clipboard.get(), "bravo");
    }

    #[test]
    fn every_cursor_loses_its_own_selection() {
        let mut app = app();
        type_text(&mut app, "aaa\nbbb");
        press(&mut app, Action::Move(Motion::DocStart));
        press(&mut app, Action::AddCursor { below: true });
        press(&mut app, Action::Select(Motion::Right));
        press(&mut app, Action::DeleteBackward);

        assert_eq!(text_of(&app), "aa\nbb");
        assert_eq!(app.window().cursors().len(), 2, "both cursors survive");
    }

    #[test]
    fn visual_mode_still_covers_the_character_under_the_caret() {
        let mut app = app();
        type_text(&mut app, "hello");
        press(&mut app, Action::Move(Motion::LineStart));
        press(&mut app, Action::EnterMode(Mode::Visual));
        press(&mut app, Action::Extend(Motion::Right));
        press(&mut app, Action::Delete);

        // `v l d` deletes two characters, as it always has.
        assert_eq!(text_of(&app), "llo");
    }

    #[test]
    fn splitting_gives_two_windows_on_one_buffer() {
        let mut app = app();
        type_text(&mut app, "shared");
        press(
            &mut app,
            Action::SplitWindow {
                axis: Axis::Horizontal,
            },
        );

        assert_eq!(app.windows.count(), 2);
        assert_eq!(app.buffers.len(), 1);
        let other = other_window(&app);
        assert_eq!(app.windows.get(other).buffer, app.window().buffer);
    }

    #[test]
    fn typing_in_one_window_changes_the_file_the_other_shows() {
        let mut app = app();
        press(
            &mut app,
            Action::SplitWindow {
                axis: Axis::Horizontal,
            },
        );
        type_text(&mut app, "hello");

        let other = other_window(&app);
        let buffer = app.windows.get(other).buffer;
        assert_eq!(app.buffers[buffer].document.text().to_string(), "hello");
    }

    #[test]
    fn each_window_keeps_its_own_caret() {
        let mut app = app();
        type_text(&mut app, "one\ntwo\nthree");
        press(&mut app, Action::Move(Motion::DocStart));
        press(
            &mut app,
            Action::SplitWindow {
                axis: Axis::Vertical,
            },
        );

        let other = other_window(&app);
        let before = app.windows.get(other).cursor().head;
        press(&mut app, Action::Move(Motion::Down(1)));

        assert_ne!(app.window().cursor().head, before);
        assert_eq!(
            app.windows.get(other).cursor().head,
            before,
            "the window that was not moved in should not have moved"
        );
    }

    #[test]
    fn undo_reaches_across_windows_on_one_file() {
        let mut app = app();
        type_text(&mut app, "typed");
        press(
            &mut app,
            Action::SplitWindow {
                axis: Axis::Horizontal,
            },
        );
        // The new window undoes an edit made through the other one, because the
        // history belongs to the file rather than to the view of it.
        press(&mut app, Action::Undo);
        assert_eq!(text_of(&app), "");
    }

    #[test]
    fn the_focus_moves_to_a_neighbouring_window() {
        let mut app = app();
        let (left, right) = side_by_side(&mut app);
        app.windows.set_focus(left);

        press(&mut app, Action::FocusWindow(Side::Right));
        assert_eq!(app.windows.focus(), right);

        press(&mut app, Action::FocusWindow(Side::Right));
        assert_eq!(app.windows.focus(), right, "no neighbour, no movement");
    }

    #[test]
    fn a_click_focuses_the_window_under_it() {
        let mut app = app();
        let (left, right) = side_by_side(&mut app);

        press(&mut app, Action::ClickAt { x: 10, y: 4 });
        assert_eq!(app.windows.focus(), left);
        press(&mut app, Action::ClickAt { x: 60, y: 4 });
        assert_eq!(app.windows.focus(), right);
    }

    #[test]
    fn a_click_puts_the_caret_on_the_character_under_it() {
        let mut app = app();
        type_text(&mut app, "alpha\nbravo\ncharlie");
        let area = clicked_area(&mut app);

        press(
            &mut app,
            Action::ClickAt {
                x: area.x + GUTTER + 3,
                y: area.y + 1,
            },
        );
        assert_eq!(app.window().cursor().head, Position::new(1, 3));
    }

    #[test]
    fn a_click_past_the_end_of_a_line_lands_on_its_last_character() {
        let mut app = app();
        type_text(&mut app, "ab\nlonger line");
        let area = clicked_area(&mut app);

        press(
            &mut app,
            Action::ClickAt {
                x: area.x + GUTTER + 30,
                y: area.y,
            },
        );
        // Normal mode's caret sits *on* a character, so it stops at the last one.
        assert_eq!(app.window().cursor().head, Position::new(0, 1));
    }

    #[test]
    fn dragging_selects_and_the_selection_can_be_operated_on() {
        let mut app = app();
        type_text(&mut app, "alpha\nbravo");
        let area = clicked_area(&mut app);

        press(
            &mut app,
            Action::ClickAt {
                x: area.x + GUTTER,
                y: area.y,
            },
        );
        press(
            &mut app,
            Action::DragTo {
                x: area.x + GUTTER + 2,
                y: area.y + 1,
            },
        );

        assert_eq!(app.mode, Mode::Normal, "dragging is not a mode change");
        let cursor = app.window().cursor();
        assert_eq!(cursor.anchor, Position::ZERO);
        assert_eq!(cursor.head, Position::new(1, 2));

        press(&mut app, Action::DeleteBackward);
        assert_eq!(text_of(&app), "avo");
    }

    #[test]
    fn a_drag_beyond_the_window_keeps_selecting_along_its_edge() {
        let mut app = app();
        type_text(&mut app, "alpha\nbravo");
        let area = clicked_area(&mut app);

        press(
            &mut app,
            Action::ClickAt {
                x: area.x + GUTTER,
                y: area.y,
            },
        );
        // Far below and to the right of the window, as a drag off the edge is.
        press(&mut app, Action::DragTo { x: 500, y: 500 });

        assert!(app.has_selection());
        assert_eq!(app.window().cursor().head, Position::new(1, 5));
    }

    #[test]
    fn a_click_drops_the_selection_it_lands_in() {
        let mut app = app();
        type_text(&mut app, "alpha");
        let area = clicked_area(&mut app);

        press(&mut app, Action::EnterMode(Mode::Visual));
        press(&mut app, Action::Extend(Motion::LineEnd));
        press(
            &mut app,
            Action::ClickAt {
                x: area.x + GUTTER + 1,
                y: area.y,
            },
        );

        assert_eq!(app.mode, Mode::Normal);
        let cursor = app.window().cursor();
        assert_eq!(cursor.head, cursor.anchor);
    }

    #[test]
    fn leaving_a_window_settles_the_mode_it_was_left_in() {
        let mut app = app();
        type_text(&mut app, "text");
        let (left, right) = side_by_side(&mut app);
        app.windows.set_focus(left);

        press(&mut app, Action::EnterMode(Mode::Visual));
        press(&mut app, Action::FocusWindow(Side::Right));

        assert_eq!(app.mode, Mode::Normal);
        assert_eq!(app.windows.focus(), right);
    }

    #[test]
    fn closing_a_window_leaves_its_buffer_open() {
        let mut app = app();
        type_text(&mut app, "kept");
        press(
            &mut app,
            Action::SplitWindow {
                axis: Axis::Horizontal,
            },
        );
        press(&mut app, Action::CloseWindow);

        assert_eq!(app.windows.count(), 1);
        assert_eq!(app.buffers.len(), 1);
        assert_eq!(text_of(&app), "kept");
    }

    #[test]
    fn the_last_window_refuses_to_close() {
        let mut app = app();
        press(&mut app, Action::CloseWindow);
        assert_eq!(app.windows.count(), 1);
        assert!(!app.should_quit());
    }

    #[test]
    fn quit_closes_a_window_before_it_closes_anything_else() {
        let mut app = app();
        press(
            &mut app,
            Action::SplitWindow {
                axis: Axis::Vertical,
            },
        );

        commands::run(&mut app, "q");
        assert_eq!(app.windows.count(), 1);
        assert!(!app.should_quit());

        commands::run(&mut app, "q");
        assert!(app.should_quit());
    }

    #[test]
    fn only_collapses_the_layout_to_the_focused_window() {
        let mut app = app();
        press(
            &mut app,
            Action::SplitWindow {
                axis: Axis::Horizontal,
            },
        );
        press(
            &mut app,
            Action::SplitWindow {
                axis: Axis::Vertical,
            },
        );
        assert_eq!(app.windows.count(), 3);

        let kept = app.windows.focus();
        commands::run(&mut app, "only");
        assert_eq!(app.windows.count(), 1);
        assert_eq!(app.windows.focus(), kept);
    }

    #[test]
    fn closing_a_buffer_repairs_every_window() {
        let mut app = app();
        app.buffers
            .push(Buffer::new(Document::from_text("second", None)));
        press(
            &mut app,
            Action::SplitWindow {
                axis: Axis::Horizontal,
            },
        );
        app.windows.focused_mut().show(1);
        let showed_second = app.windows.focus();
        let showed_first = other_window(&app);

        app.close_active();

        assert_eq!(app.buffers.len(), 1);
        assert_eq!(app.windows.get(showed_second).buffer, 0);
        assert_eq!(app.windows.get(showed_first).buffer, 0);
    }

    #[test]
    fn scrolling_carries_the_caret_along_rather_than_snapping_back() {
        let mut app = app();
        let lines: String = (0..200).map(|n| format!("line {n}\n")).collect();
        app.buffers[0] = Buffer::new(Document::from_text(&lines, None));
        app.windows.focused_mut().reset(0);
        app.windows.focused_mut().area = Area::new(0, 0, 80, 10);

        press(&mut app, Action::Scroll(40));

        let window = app.window();
        assert_eq!(window.view.top_line, 40);
        // The caret started on line 0 and cannot still be there.
        assert!(window.cursor().head.line >= window.view.top_line);
        assert!(window.cursor().head.line < window.view.top_line + 10);
    }

    #[test]
    fn tree_creation_prefills_the_selected_directory() {
        let root = std::env::temp_dir().join("termi-dispatch-tree-create");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("src")).expect("create fixture");
        let mut app = app();
        app.tree = Some(crate::filesystem::tree::Tree::new(root.clone()));
        app.tree_visible = true;

        press(&mut app, Action::TreeCreate { directory: false });

        assert_eq!(app.mode, Mode::Command);
        assert_eq!(
            app.command_line,
            format!(
                "touch {}{}",
                root.join("src").display(),
                std::path::MAIN_SEPARATOR
            )
        );
    }
}
