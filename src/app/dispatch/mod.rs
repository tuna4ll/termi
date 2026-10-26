//! The single `match` from an `Action` to an effect on the application.
//! Dispatch owns edit policy; the mechanics live in the editor layer.

mod motion;
mod operator;
#[cfg(test)]
mod vim_tests;

use anyhow::Result;

use super::commands;
use super::mode::Mode;
use super::state::App;
use crate::clipboard::RegisterKind;
use crate::clipboard::registers::Register;
use crate::editor::cursor::Motion;
use crate::editor::document::indent;
use crate::editor::selection::Range;
use crate::editor::window::{VisualMemory, WindowId};
use crate::input::{Action, Operator, notation};
use crate::picker::PickerTarget;
use crate::ui::widgets::editor_view;

/// How many keys one burst of macro playback may feed in before it is
/// stopped, so a macro that calls itself cannot hang the editor.
const MACRO_KEY_LIMIT: usize = 100_000;

pub fn apply(app: &mut App, action: Action) -> Result<()> {
    record_change(app, &action);
    let result = execute(app, action);
    finish_change(app);
    result
}

/// Whether `action`, typed in normal mode, changes the text and so is what
/// `.` should repeat.
fn is_change(action: &Action) -> bool {
    match action {
        Action::Operate { operator, .. } => operator.changes_text(),
        Action::WithRegister { action, .. } | Action::Repeat { action, .. } => is_change(action),
        Action::Put { .. }
        | Action::Paste
        | Action::ReplaceChar { .. }
        | Action::ToggleCaseChars(_)
        | Action::Join { .. }
        | Action::EnterMode(Mode::Insert)
        | Action::OpenLineBelow
        | Action::OpenLineAbove
        | Action::AppendAfter
        | Action::AppendAtLineEnd
        | Action::InsertAtLineStart
        | Action::DeleteForward
        | Action::DeleteBackward
        | Action::ToggleComment => true,
        _ => false,
    }
}

/// Whether `action`, typed during the insert session a change opened, belongs
/// to that change.
const fn belongs_to_insert(action: &Action) -> bool {
    matches!(
        action,
        Action::Insert(_)
            | Action::InsertText(_)
            | Action::InsertNewline
            | Action::InsertIndent
            | Action::DeleteBackward
            | Action::DeleteForward
            | Action::DeleteWordBackward
            | Action::DeleteWordForward
            | Action::Move(_)
            | Action::EnterMode(Mode::Normal)
            | Action::Paste
    )
}

fn record_change(app: &mut App, action: &Action) {
    if app.replaying {
        return;
    }
    if let Some(recording) = app.change_recording.as_mut() {
        if belongs_to_insert(action) {
            recording.push(action.clone());
        }
        return;
    }
    if app.mode == Mode::Normal && is_change(action) {
        app.change_recording = Some(vec![action.clone()]);
    }
}

fn finish_change(app: &mut App) {
    if app.mode != Mode::Insert
        && let Some(recording) = app.change_recording.take()
    {
        app.last_change = recording;
    }
}

/// `.` — replay the last change, with a new count if one was typed.
fn repeat_change(app: &mut App, count: Option<usize>) -> Result<()> {
    let mut actions = app.last_change.clone();
    if actions.is_empty() {
        return Ok(());
    }
    let rounds = match count {
        Some(count)
            if actions
                .first_mut()
                .is_some_and(|first| set_count(first, count)) =>
        {
            1
        }
        Some(count) => count,
        None => 1,
    };
    app.replaying = true;
    for _ in 0..rounds {
        for action in &actions {
            if let Err(error) = execute(app, action.clone()) {
                app.replaying = false;
                return Err(error);
            }
        }
        // A replay that ended in insert mode (an interrupted recording) must
        // not leave the editor there.
        if app.mode == Mode::Insert {
            enter_mode(app, Mode::Normal);
        }
    }
    app.replaying = false;
    Ok(())
}

/// Swap the count inside a repeatable action; false when it has none.
fn set_count(action: &mut Action, new: usize) -> bool {
    match action {
        Action::Operate { count, .. }
        | Action::Put { count, .. }
        | Action::ReplaceChar { count, .. }
        | Action::Join { count, .. } => {
            *count = new;
            true
        }
        Action::ToggleCaseChars(count) => {
            *count = new;
            true
        }
        Action::WithRegister { action, .. } => set_count(action, new),
        _ => false,
    }
}

fn execute(app: &mut App, action: Action) -> Result<()> {
    if app.popup.is_some() && app.mode == Mode::Confirm {
        if !matches!(&action, Action::ConfirmDelete | Action::ConfirmCancel) {
            return Ok(());
        }
        app.popup = None;
    } else if app.popup.is_some() && !matches!(action, Action::None) {
        app.popup = None;
        return Ok(());
    }
    if !matches!(action, Action::None) {
        app.clear_status();
    }

    match action {
        Action::None => {}

        Action::EnterMode(Mode::VisualBlock) if app.mode == Mode::VisualBlock => {
            enter_mode(app, Mode::Normal);
        }
        Action::EnterMode(mode) => enter_mode(app, mode),
        Action::Move(motion) => {
            if motion.is_jump() && !app.mode.is_insert() {
                app.window_mut().record_jump();
            }
            move_cursors(app, motion, false);
        }
        Action::Goto { goal, count } => motion::goto(app, goal, count),
        Action::Operate {
            operator,
            target,
            count,
        } => operator::operate(app, operator, target, count),
        Action::OperateSelection { operator, count } => {
            if app.mode.is_visual() {
                operator::operate_selection(app, operator, count);
            }
        }
        Action::SelectObject {
            object,
            inner,
            count,
        } => operator::select_object(app, object, inner, count),
        Action::Put { before, count } => operator::put(app, before, count),
        Action::Repeat { count, action } => {
            for _ in 0..count {
                execute(app, (*action).clone())?;
            }
        }
        Action::WithRegister { register, action } => {
            app.registers.select(register);
            let result = execute(app, *action);
            app.registers.clear_selection();
            result?;
        }
        Action::RepeatChange(count) => repeat_change(app, count)?,
        Action::ReplaceChar { ch, count } => operator::replace_char(app, ch, count),
        Action::ToggleCaseChars(count) => operator::toggle_case_chars(app, count),
        Action::Join { count, spaces } => operator::join(app, count, spaces),
        Action::JoinSelection { spaces } => operator::join_selection(app, spaces),
        Action::ReplaceSelection(ch) => operator::replace_selection(app, ch),
        Action::SwapAnchor => {
            for cursor in app.window_mut().cursors_mut() {
                std::mem::swap(&mut cursor.anchor, &mut cursor.head);
            }
        }
        Action::BlockInsert { append } => operator::block_insert(app, append),
        Action::SetMark(name) => motion::set_mark(app, name),
        Action::RecordMacro(register) => {
            app.macro_recording = Some((register, Vec::new()));
            app.info(format!("recording @{register}"));
        }
        Action::StopMacro => stop_macro(app),
        Action::PlayMacro { register, count } => play_macro(app, register, count)?,
        Action::SearchWord { forward, whole } => search_word(app, forward, whole),
        Action::RunCommand(line) => commands::run(app, &line),
        Action::ScrollCaret(row) => motion::scroll_caret(app, row),
        Action::Reselect => reselect(app),
        Action::JumpBack => motion::jump(app, true),
        Action::JumpForward => motion::jump(app, false),
        Action::InsertText(text) => {
            let mut edit = app.edit();
            edit.delete_selections();
            edit.insert_text(&text);
            edit.checkpoint();
        }
        Action::ToggleComment => operator::toggle_comment(app),
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
        Action::DeleteWordBackward => {
            let mut edit = app.edit();
            if !edit.delete_selections() {
                edit.delete_word_backward();
            }
        }
        Action::DeleteWordForward => {
            let mut edit = app.edit();
            if !edit.delete_selections() {
                edit.delete_word_forward();
            }
        }

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
        Action::UndoEarlier(count) => {
            if !app.edit().earlier(count) {
                app.info("already at the oldest state");
            }
        }
        Action::UndoLater(count) => {
            if !app.edit().later(count) {
                app.info("already at the newest state");
            }
        }
        Action::Redo => {
            if !app.edit().redo() {
                app.info("already at the newest change");
            }
        }

        Action::Yank if app.mode.is_visual() => {
            operator::operate_selection(app, Operator::Yank, 1);
        }
        Action::Cut if app.mode.is_visual() => {
            operator::operate_selection(app, Operator::Delete, 1);
        }
        Action::Yank => yank(app, false),
        Action::Cut => yank(app, true),
        Action::Paste => operator::paste(app),

        Action::AddCursor { below } => add_cursor(app, below),
        Action::ClearCursors => {
            app.window_mut().clear_secondary_cursors();
            app.window_mut().collapse_selections();
        }

        Action::CycleBuffer { forward } => app.cycle_buffer(forward),
        Action::OpenPicker(kind) => app.open_picker(kind),
        Action::PickerInput(ch) => {
            if let Some(picker) = app.picker.as_mut() {
                picker.push(ch);
            }
        }
        Action::PickerBackspace => {
            if let Some(picker) = app.picker.as_mut() {
                picker.pop();
            }
        }
        Action::PickerMove(delta) => {
            if let Some(picker) = app.picker.as_mut() {
                picker.move_selection(delta);
            }
        }
        Action::PickerSubmit => submit_picker(app),
        Action::PickerCancel => close_picker(app),
        Action::Save => save(app),
        Action::Quit => quit(app),

        Action::SplitWindow { axis } => {
            app.split_window(axis);
        }
        Action::CloseWindow => close_window(app),
        Action::OnlyWindow => app.close_other_windows(),
        Action::FocusWindow(side) => {
            if let Some(id) = app.windows.neighbour(side) {
                focus_window(app, id);
            }
        }
        Action::CycleWindow => cycle_window(app),
        Action::ResizeWindow { axis, delta } => app.windows.resize(axis, delta),
        Action::EqualiseWindows => app.windows.equalise(),
        Action::OpenTerminal => {
            if let Err(error) = app.open_terminal(None) {
                app.error(format!("unable to open terminal: {error}"));
            }
        }
        Action::TerminalInput(key) => {
            if let Err(error) = app.send_terminal_key(key) {
                app.error(error.to_string());
            }
        }
        Action::ClickAt { x, y } => click(app, x, y),
        Action::DragTo { x, y } => drag(app, x, y),
        Action::ScrollAt { x, y, delta } => {
            if let Some(id) = app.windows.at(x, y) {
                if app.is_terminal(id) {
                    app.scroll_terminal(id, delta);
                } else {
                    app.scroll_window(id, delta);
                }
            }
        }

        Action::ToggleTree => toggle_tree(app),
        Action::TreeMove(delta) => move_tree_selection(app, delta),
        Action::TreeActivate => activate_tree_entry(app),
        Action::TreeCreate { directory } => begin_tree_creation(app, directory),
        Action::TreeToggleHidden => {
            if let Some(tree) = app.tree.as_mut() {
                tree.toggle_hidden();
                app.tree_selected = app
                    .tree_selected
                    .min(tree.entries().len().saturating_sub(1));
            }
        }
        Action::TreeRename => begin_tree_path(app, "rename", true),
        Action::TreeCopy => begin_tree_path(app, "copy", false),
        Action::TreeMovePath => begin_tree_path(app, "move", false),
        Action::TreeDelete => begin_tree_delete(app),
        Action::ConfirmDelete => delete_tree_path(app),
        Action::ConfirmCancel => cancel_tree_delete(app),

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
                enter_mode(app, Mode::Normal);
            }
        }
        Action::CommandCancel => {
            app.command_line.clear();
            app.tree_source = None;
            enter_mode(app, Mode::Normal);
        }
        Action::CommandSubmit => {
            let line = std::mem::take(&mut app.command_line);
            enter_mode(app, Mode::Normal);
            commands::run(app, &line);
            app.tree_source = None;
        }
    }
    Ok(())
}

fn insert_char(app: &mut App, ch: char) {
    let (mut edit, config) = app.edit_and_config();
    edit.delete_selections();
    let head = edit.window.cursor().head;

    if config.auto_indent && indent::should_dedent(&edit.buffer.document, head.line, head.col, ch) {
        edit.delete_backward(config);
    }
    edit.insert_char(ch, config);
}

const MATCH_COUNT_CAP: usize = 10_000;

fn origin_offset(app: &App) -> usize {
    app.buffer()
        .document
        .pos_to_char(app.window().cursor().head)
}

fn seek_from_origin(app: &mut App) {
    let origin = app.search.origin();
    let forward = app.search.forward;
    let Some(found) = app.search.find(&app.buffer().document, origin, forward) else {
        return;
    };
    let position = app.buffer().document.char_to_pos(found.start);
    app.window_mut().cursor_mut().move_to(position, false);
}

fn cancel_search(app: &mut App) -> Result<()> {
    let origin = app.search.origin();
    let position = app.buffer().document.char_to_pos(origin);
    app.window_mut().cursor_mut().move_to(position, false);
    app.search.set_query(String::new());
    enter_mode(app, Mode::Normal);
    Ok(())
}

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

fn close_window(app: &mut App) {
    let focus = app.windows.focus();
    if !app.close_window(focus) {
        app.info("only one window");
    }
}

fn focus_window(app: &mut App, id: WindowId) {
    if id == app.windows.focus() {
        return;
    }
    app.windows.set_focus(id);
    app.sync_mode_to_focus();
}

fn click(app: &mut App, x: u16, y: u16) {
    let Some(id) = app.windows.at(x, y) else {
        return;
    };
    if id != app.windows.focus() {
        focus_window(app, id);
    } else if app.mode.is_visual() {
        enter_mode(app, Mode::Normal);
    }
    if app.terminal_focused() {
        return;
    }
    place_caret(app, x, y, false);
}

fn drag(app: &mut App, x: u16, y: u16) {
    if app.terminal_focused() {
        return;
    }
    let area = app.window().area;
    let x = x.clamp(area.x, area.right().saturating_sub(1).max(area.x));
    let y = y.clamp(area.y, area.bottom().saturating_sub(1).max(area.y));
    place_caret(app, x, y, true);
}

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

fn cycle_window(app: &mut App) {
    let order = app.windows.ids();
    let current = order
        .iter()
        .position(|id| *id == app.windows.focus())
        .unwrap_or(0);
    let next = order[(current + 1) % order.len()];
    focus_window(app, next);
}

fn toggle_tree(app: &mut App) {
    app.tree_visible = !app.tree_visible;
    if !app.tree_visible {
        enter_mode(app, Mode::Normal);
        return;
    }
    if app.tree.is_none() {
        let root = app.tree_root();
        app.tree = Some(crate::filesystem::tree::Tree::new(root));
    } else {
        // Opening the panel is the natural way to ask "what is in there now?",
        // so it re-reads even when the watcher is off or never saw the change.
        app.refresh_tree();
    }
    enter_mode(app, Mode::Tree);
}

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

fn activate_tree_entry(app: &mut App) {
    let Some(tree) = app.tree.as_mut() else {
        return;
    };
    let Some(path) = tree.activate(app.tree_selected) else {
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

fn begin_tree_path(app: &mut App, command: &str, keep_name: bool) {
    let Some(path) = app
        .tree
        .as_ref()
        .and_then(|tree| tree.path_at(app.tree_selected))
        .map(std::path::Path::to_path_buf)
    else {
        return;
    };
    let destination = if keep_name {
        path.clone()
    } else {
        path.parent()
            .map(std::path::Path::to_path_buf)
            .unwrap_or_default()
    };
    app.tree_source = Some(path);
    enter_mode(app, Mode::Command);
    let separator = std::path::MAIN_SEPARATOR;
    let mut destination = destination.as_os_str().to_string_lossy().into_owned();
    if !keep_name && !destination.ends_with(separator) {
        destination.push(separator);
    }
    app.command_line = format!("{command} {destination}");
}

fn begin_tree_delete(app: &mut App) {
    let Some(path) = app
        .tree
        .as_ref()
        .and_then(|tree| tree.path_at(app.tree_selected))
        .map(std::path::Path::to_path_buf)
    else {
        return;
    };
    if app.has_dirty_under(&path) {
        app.error(format!(
            "save or close modified files before deleting {}",
            path.display()
        ));
        return;
    }
    let kind = if path.is_dir() { "directory" } else { "file" };
    app.pending_delete = Some(path.clone());
    app.popup = Some((
        format!("delete {kind} permanently?"),
        format!("{}\n\nThis cannot be undone.", path.display()),
    ));
    app.mode = Mode::Confirm;
}

fn cancel_tree_delete(app: &mut App) {
    app.pending_delete = None;
    app.mode = if app.tree_visible {
        Mode::Tree
    } else {
        Mode::Normal
    };
}

fn delete_tree_path(app: &mut App) {
    let Some(path) = app.pending_delete.take() else {
        return cancel_tree_delete(app);
    };
    if app.has_dirty_under(&path) {
        app.error(format!(
            "save or close modified files before deleting {}",
            path.display()
        ));
        return cancel_tree_delete(app);
    }
    match crate::filesystem::delete_path(&path) {
        Ok(()) => {
            app.close_buffers_under(&path);
            app.refresh_tree();
            app.info(format!("deleted {}", path.display()));
        }
        Err(error) => app.error(error.to_string()),
    }
    cancel_tree_delete(app);
}

fn enter_mode(app: &mut App, mode: Mode) {
    if app.mode == mode {
        return;
    }
    let leaving = app.mode;
    if leaving.is_visual() {
        remember_visual(app, leaving);
    }
    if leaving == Mode::Insert && app.insert_group_open {
        app.edit().end_group();
        app.insert_group_open = false;
    }
    app.edit().checkpoint();

    match mode {
        Mode::Normal => {
            app.window_mut().collapse_selections();
            app.clamp_cursors(false);
        }
        Mode::Visual | Mode::VisualLine | Mode::VisualBlock => {
            // Switching between the visual modes keeps the selection; only
            // arriving from elsewhere starts a new one.
            if !leaving.is_visual() {
                app.clamp_cursors(false);
                app.window_mut().anchor_selections();
            }
            if mode == Mode::VisualBlock {
                app.window_mut().clear_secondary_cursors();
            }
        }
        Mode::Command => app.command_line.clear(),
        Mode::Insert => app.window_mut().collapse_selections(),
        Mode::Search | Mode::Tree | Mode::Picker | Mode::Confirm | Mode::Terminal => {}
    }
    app.mode = mode;
}

/// Keep a visual selection for `gv`, and as the `<` and `>` marks.
fn remember_visual(app: &mut App, mode: Mode) {
    let cursor = app.window().cursor();
    app.window_mut().last_visual = Some(VisualMemory {
        mode: mode.name(),
        anchor: cursor.anchor,
        head: cursor.head,
    });
    let (start, end) = if cursor.anchor <= cursor.head {
        (cursor.anchor, cursor.head)
    } else {
        (cursor.head, cursor.anchor)
    };
    let marks = &mut app.buffer_mut().marks;
    marks.insert('<', start);
    marks.insert('>', end);
}

/// `gv` — bring the last visual selection back.
fn reselect(app: &mut App) {
    let Some(memory) = app.window().last_visual else {
        app.info("no previous selection");
        return;
    };
    let mode = match memory.mode {
        "V-LINE" => Mode::VisualLine,
        "V-BLOCK" => Mode::VisualBlock,
        _ => Mode::Visual,
    };
    if app.mode.is_visual() {
        enter_mode(app, Mode::Normal);
    }
    enter_mode(app, mode);
    let document = &app.buffer().document;
    let anchor = document.clamp(memory.anchor, false);
    let head = document.clamp(memory.head, false);
    let cursor = app.window_mut().cursor_mut();
    cursor.move_to(anchor, false);
    cursor.move_to(head, true);
}

/// `q` again — store the recording in its register as key notation.
fn stop_macro(app: &mut App) {
    let Some((register, mut keys)) = app.macro_recording.take() else {
        return;
    };
    // The `q` that stopped the recording was recorded too.
    keys.pop();
    let text = notation::encode(&keys);
    app.registers.set_named(
        register.to_ascii_lowercase(),
        Register::new(text, RegisterKind::Charwise),
    );
    app.last_macro = Some(register.to_ascii_lowercase());
    app.info(format!("recorded @{register}"));
}

/// `@a` — feed a register's keys back in, `count` times; `@@` repeats the
/// last one and `@:` the last command line.
fn play_macro(app: &mut App, register: char, count: usize) -> Result<()> {
    let register = if register == '@' {
        match app.last_macro {
            Some(last) => last,
            None => {
                app.info("no macro played yet");
                return Ok(());
            }
        }
    } else {
        register
    };
    if register == ':' {
        let Some(line) = app.registers.named(':').map(|r| r.text.clone()) else {
            app.info("no previous command");
            return Ok(());
        };
        for _ in 0..count.max(1) {
            commands::run(app, &line);
        }
        return Ok(());
    }
    let Some(text) = app.registers.named(register).map(|r| r.text.clone()) else {
        app.error(format!("register {register} is empty"));
        return Ok(());
    };
    let keys = match notation::parse(&text) {
        Ok(keys) => keys,
        Err(error) => {
            app.error(error);
            return Ok(());
        }
    };
    if keys.len().saturating_mul(count) > MACRO_KEY_LIMIT {
        app.error("macro too long to play");
        return Ok(());
    }
    app.last_macro = Some(register);
    for _ in 0..count.max(1) {
        app.pending_keys.extend(keys.iter().copied());
    }
    Ok(())
}

/// `*` and `#` — search for the word under the caret.
fn search_word(app: &mut App, forward: bool, whole: bool) {
    let Some(word) = operator::word_under_caret(app) else {
        app.error("no word under the caret");
        return;
    };
    app.search.set_word(&word, whole, forward);
    app.registers.set_named(
        '/',
        Register::new(app.search.query.clone(), RegisterKind::Charwise),
    );
    app.window_mut().record_jump();
    repeat_search(app, forward);
}

fn close_picker(app: &mut App) {
    app.picker = None;
    enter_mode(app, Mode::Normal);
}

fn submit_picker(app: &mut App) {
    let target = app
        .picker
        .as_ref()
        .and_then(|picker| picker.selected())
        .map(|item| item.target.clone());
    close_picker(app);
    match target {
        Some(PickerTarget::File(path)) => {
            if let Err(error) = app.open(path) {
                app.error(error.to_string());
            }
        }
        Some(PickerTarget::Buffer(index)) if index < app.buffers.len() => {
            app.windows.focused_mut().show(index);
        }
        Some(PickerTarget::Command(command)) => commands::run(app, &command),
        Some(PickerTarget::Theme(theme)) => commands::run(app, &format!("theme {theme}")),
        _ => {}
    }
}

pub(super) fn move_cursors(app: &mut App, motion: Motion, extend: bool) {
    let extend = extend || app.mode.is_visual();
    let allow_eol = app.mode.is_insert() || (extend && !app.mode.is_visual());
    app.edit().checkpoint();
    app.move_cursors(motion, extend, allow_eol);
}

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

fn yank(app: &mut App, cut: bool) {
    let (range, line_wise) = target_range(app);
    let text = app.buffer().document.slice_string(range.start, range.end);
    if text.is_empty() {
        return;
    }
    let lines = text.lines().count();
    let characters = text.chars().count();
    let kind = if line_wise {
        RegisterKind::Linewise
    } else {
        RegisterKind::Charwise
    };
    if cut {
        app.registers.delete(text, kind);
    } else {
        app.registers.yank(text, kind);
    }

    if cut {
        app.edit().delete_range(range);
    }
    enter_mode(app, Mode::Normal);

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

fn open_line(app: &mut App, below: bool) {
    enter_mode(app, Mode::Insert);
    let motion = if below {
        Motion::LineEnd
    } else {
        Motion::LineFirstNonBlank
    };
    app.move_cursors(motion, false, true);

    let (mut edit, config) = app.edit_and_config();
    edit.insert_newline(config);
    if !below {
        edit.window
            .move_cursors(Motion::Up(1), &edit.buffer.document, false, true);
    }
}

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
    let undo_dir = app.undo_dir();
    match app.buffer_mut().save(undo_dir.as_deref()) {
        Ok(()) => {
            let name = app.buffer().document.display_name().to_string();
            app.info(format!("wrote {name}"));
        }
        Err(error) => app.error(error.to_string()),
    }
}

fn quit(app: &mut App) {
    match app.unsaved_message() {
        Some(message) => app.error(message),
        None => app.quit(),
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

    const GUTTER: u16 = 5;

    fn clicked_area(app: &mut App) -> Area {
        let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(60, 12))
            .expect("the test backend always builds");
        terminal
            .draw(|frame| crate::ui::draw(frame, app))
            .expect("drawing into the test backend should not fail");
        app.window().area
    }

    fn other_window(app: &App) -> WindowId {
        app.windows
            .ids()
            .into_iter()
            .find(|id| *id != app.windows.focus())
            .expect("there should be a second window")
    }

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

        assert_eq!(
            app.registers.get().map(|r| r.text).unwrap_or_default(),
            "abc"
        );
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

        let register = app.registers.get().expect("yanked");
        assert_eq!(register.text, "hello");
        assert_eq!(register.kind, RegisterKind::Charwise);
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
        app.registers
            .yank("three".to_string(), RegisterKind::Charwise);
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

        let register = app.registers.get().expect("yanked");
        assert_eq!(register.kind, RegisterKind::Linewise);
        assert_eq!(register.text, "bravo");
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
        press(
            &mut app,
            Action::OperateSelection {
                operator: Operator::Delete,
                count: 1,
            },
        );

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
        assert!(window.cursor().head.line >= window.view.top_line);
        assert!(window.cursor().head.line < window.view.top_line + 10);
    }

    #[test]
    fn reopening_the_tree_picks_up_files_created_outside_the_editor() {
        let root = std::env::temp_dir().join("termi-dispatch-tree-external");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("create fixture");
        std::fs::write(root.join("b.txt"), "").expect("create fixture");
        let mut app = app();
        app.tree = Some(crate::filesystem::tree::Tree::new(root.clone()));
        app.tree_visible = true;
        app.tree_selected = 0;

        // Something else adds a file that sorts above the selected one.
        std::fs::write(root.join("a.txt"), "").expect("create fixture");
        press(&mut app, Action::ToggleTree);
        press(&mut app, Action::ToggleTree);

        let tree = app.tree.as_ref().expect("tree");
        let names: Vec<&str> = tree.entries().iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["a.txt", "b.txt"]);
        // The selection followed its file rather than staying on row zero.
        assert_eq!(
            tree.path_at(app.tree_selected),
            Some(root.join("b.txt").as_path())
        );
    }

    #[test]
    fn refreshing_clamps_a_selection_whose_file_is_gone() {
        let root = std::env::temp_dir().join("termi-dispatch-tree-removed");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("create fixture");
        std::fs::write(root.join("only.txt"), "").expect("create fixture");
        let mut app = app();
        app.tree = Some(crate::filesystem::tree::Tree::new(root.clone()));
        app.tree_selected = 0;

        std::fs::remove_file(root.join("only.txt")).expect("remove fixture");
        app.refresh_tree();

        assert!(app.tree.as_ref().expect("tree").entries().is_empty());
        assert_eq!(app.tree_selected, 0);
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

    #[test]
    fn tree_delete_requires_explicit_confirmation() {
        let root = std::env::temp_dir().join("termi-dispatch-tree-delete-cancel");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("create fixture");
        let path = root.join("note.txt");
        std::fs::write(&path, "hello").expect("write fixture");
        let mut app = app();
        app.tree = Some(crate::filesystem::tree::Tree::new(root));
        app.tree_visible = true;

        press(&mut app, Action::TreeDelete);
        assert_eq!(app.mode, Mode::Confirm);
        assert!(path.exists());

        press(&mut app, Action::ConfirmCancel);
        assert_eq!(app.mode, Mode::Tree);
        assert!(path.exists());
    }

    #[test]
    fn confirmed_tree_delete_closes_a_clean_buffer() {
        let root = std::env::temp_dir().join("termi-dispatch-tree-delete-clean");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("create fixture");
        let path = root.join("note.txt");
        std::fs::write(&path, "hello").expect("write fixture");
        let mut app = app();
        app.open(path.clone()).expect("open fixture");
        app.tree = Some(crate::filesystem::tree::Tree::new(root));
        app.tree_visible = true;

        press(&mut app, Action::TreeDelete);
        press(&mut app, Action::ConfirmDelete);

        assert!(!path.exists());
        assert_eq!(app.buffer().document.path(), None);
        assert_eq!(app.mode, Mode::Tree);
    }

    #[test]
    fn tree_delete_refuses_a_modified_buffer() {
        let root = std::env::temp_dir().join("termi-dispatch-tree-delete-dirty");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("create fixture");
        let path = root.join("note.txt");
        std::fs::write(&path, "hello").expect("write fixture");
        let mut app = app();
        app.open(path.clone()).expect("open fixture");
        type_text(&mut app, "changed");
        app.tree = Some(crate::filesystem::tree::Tree::new(root));
        app.tree_visible = true;
        app.mode = Mode::Tree;

        press(&mut app, Action::TreeDelete);

        assert!(path.exists());
        assert_eq!(app.mode, Mode::Tree);
        assert!(app.status.is_error);
        assert!(app.status.text.contains("save or close"));
    }
}
