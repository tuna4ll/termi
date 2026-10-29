//! Windows and the mouse: focus, clicks, drags and cycling.

use super::enter_mode;
use crate::app::mode::Mode;
use crate::app::state::App;
use crate::editor::window::WindowId;
use crate::ui::widgets::editor_view;

pub(super) fn close_window(app: &mut App) {
    let focus = app.windows.focus();
    if !app.close_window(focus) {
        app.info("only one window");
    }
}

pub(super) fn focus_window(app: &mut App, id: WindowId) {
    if id == app.windows.focus() {
        return;
    }
    app.windows.set_focus(id);
    app.sync_mode_to_focus();
}

pub(super) fn click(app: &mut App, x: u16, y: u16) {
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

pub(super) fn drag(app: &mut App, x: u16, y: u16) {
    if app.terminal_focused() {
        return;
    }
    let area = app.window().area;
    let x = x.clamp(area.x, area.right().saturating_sub(1).max(area.x));
    let y = y.clamp(area.y, area.bottom().saturating_sub(1).max(area.y));
    place_caret(app, x, y, true);
}

pub(super) fn place_caret(app: &mut App, x: u16, y: u16, extend: bool) {
    let window = app.window();
    let config = app.buffer_config(window.buffer);
    let position = editor_view::position_at(
        &app.buffers[window.buffer],
        window,
        &config,
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

pub(super) fn cycle_window(app: &mut App) {
    let order = app.windows.ids();
    let current = order
        .iter()
        .position(|id| *id == app.windows.focus())
        .unwrap_or(0);
    let next = order[(current + 1) % order.len()];
    focus_window(app, next);
}
