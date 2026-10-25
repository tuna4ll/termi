//! Moving the caret to places that need more than the document to find:
//! counts, marks, screen rows and the jump list.

use super::{enter_mode, move_cursors};
use crate::app::mode::Mode;
use crate::app::state::App;
use crate::editor::cursor::{Cursor, Motion, MotionKind, Position, scan};
use crate::input::{Goal, ScreenRow};

/// How an operator should treat the span up to `goal`.
#[must_use]
pub const fn kind_of(goal: Goal) -> MotionKind {
    match goal {
        Goal::Motion(motion) => motion.kind(),
        Goal::Find(_) | Goal::Repeat(_) => MotionKind::Inclusive,
        Goal::Mark { exact: true, .. } => MotionKind::Exclusive,
        Goal::Mark { exact: false, .. } | Goal::Screen(_) => MotionKind::Linewise,
    }
}

/// Where `goal` lands, starting from `from`, `count` times over.
///
/// `allow_eol` lets the caret rest past the last character, which an
/// exclusive operator span needs to reach the end of a line.
#[must_use]
pub fn resolve(
    app: &App,
    from: Position,
    goal: Goal,
    count: usize,
    allow_eol: bool,
) -> Option<Position> {
    let document = &app.buffer().document;
    let count = count.max(1);
    match goal {
        Goal::Motion(Motion::LineEnd) if count > 1 => {
            let line = (from.line + count - 1).min(document.last_line());
            Some(document.clamp(Position::new(line, usize::MAX), allow_eol))
        }
        Goal::Motion(motion) => {
            let mut cursor = Cursor::at(from);
            match motion.scaled(count) {
                Some(scaled) if count > 1 => cursor.apply(scaled, document, false, allow_eol),
                _ => {
                    for _ in 0..count {
                        cursor.apply(motion, document, false, allow_eol);
                    }
                }
            }
            Some(cursor.head)
        }
        Goal::Find(spec) => {
            scan::find_in_line(document, from, spec.target, spec.forward, spec.till, count)
        }
        Goal::Repeat(spec) => {
            // `;` after `t` must not stick against the character it stopped
            // short of, so the search starts one step further on.
            let start = if spec.till {
                let col = if spec.forward {
                    from.col + 1
                } else {
                    from.col.checked_sub(1)?
                };
                Position::new(from.line, col)
            } else {
                from
            };
            scan::find_in_line(document, start, spec.target, spec.forward, spec.till, count)
        }
        Goal::Mark { name, exact } => {
            let position = local_mark(app, name)?;
            Some(if exact {
                document.clamp(position, allow_eol)
            } else {
                let line = position.line.min(document.last_line());
                Position::new(
                    line,
                    crate::editor::document::indent::first_non_blank(document, line),
                )
            })
        }
        Goal::Screen(row) => {
            let line = screen_line(app, row);
            Some(Position::new(
                line,
                crate::editor::document::indent::first_non_blank(document, line),
            ))
        }
    }
}

/// A mark in the focused buffer, including the ones the editor keeps.
fn local_mark(app: &App, name: char) -> Option<Position> {
    match name {
        // `''` and ``` `` ``` return to where the last jump started.
        '\'' | '`' => app.window().last_jump().map(|jump| jump.position),
        _ => app.buffer().marks.get(&name).copied(),
    }
}

/// The document line at a row of the focused window.
fn screen_line(app: &App, row: ScreenRow) -> usize {
    let window = app.window();
    let last = app.buffer().document.last_line();
    let height = usize::from(window.area.height).max(1);
    let top = window.view.top_line;
    let bottom = (top + height - 1).min(last);
    let margin = app.config.scrolloff.min(height.saturating_sub(1) / 2);
    match row {
        ScreenRow::Top if top == 0 => 0,
        ScreenRow::Top => (top + margin).min(bottom),
        ScreenRow::Middle => top + (bottom - top) / 2,
        ScreenRow::Bottom if bottom == last => last,
        ScreenRow::Bottom => bottom.saturating_sub(margin).max(top),
    }
}

/// Move to `goal`, `count` times over.
pub fn goto(app: &mut App, goal: Goal, count: usize) {
    // A plain motion moves every cursor; the rest only make sense for one.
    if let Goal::Motion(motion) = goal {
        if motion.is_jump() {
            app.window_mut().record_jump();
        }
        if motion == Motion::LineEnd && count > 1 {
            move_cursors(app, Motion::Down(count - 1), false);
            move_cursors(app, Motion::LineEnd, false);
            return;
        }
        for _ in 0..count.max(1) {
            move_cursors(app, motion, false);
        }
        return;
    }

    if let Goal::Mark { name, exact } = goal
        && name.is_ascii_uppercase()
    {
        return goto_global_mark(app, name, exact);
    }
    let allow_eol = app.mode.is_insert();
    let from = app.window().cursor().head;
    let Some(target) = resolve(app, from, goal, count, allow_eol) else {
        if let Goal::Mark { name, .. } = goal {
            app.error(format!("mark {name} is not set"));
        }
        return;
    };
    if matches!(goal, Goal::Mark { .. } | Goal::Screen(_)) {
        app.window_mut().record_jump();
    }
    let extend = app.mode.is_visual();
    app.edit().checkpoint();
    app.window_mut().clear_secondary_cursors();
    app.window_mut().cursor_mut().move_to(target, extend);
}

fn goto_global_mark(app: &mut App, name: char, exact: bool) {
    let Some((path, position)) = app.global_marks.get(&name).cloned() else {
        app.error(format!("mark {name} is not set"));
        return;
    };
    app.window_mut().record_jump();
    if app.buffer().document.path() != Some(path.as_path())
        && let Err(error) = app.open(path)
    {
        app.error(error.to_string());
        return;
    }
    let document = &app.buffer().document;
    let target = if exact {
        document.clamp(position, false)
    } else {
        let line = position.line.min(document.last_line());
        Position::new(
            line,
            crate::editor::document::indent::first_non_blank(document, line),
        )
    };
    if app.mode.is_visual() {
        enter_mode(app, Mode::Normal);
    }
    app.window_mut().clear_secondary_cursors();
    app.window_mut().cursor_mut().move_to(target, false);
}

/// `m` — remember where the caret is under `name`.
pub fn set_mark(app: &mut App, name: char) {
    let position = app.window().cursor().head;
    if name.is_ascii_uppercase() {
        let Some(path) = app
            .buffer()
            .document
            .path()
            .map(std::path::Path::to_path_buf)
        else {
            app.error("a file mark needs a file — save the buffer first");
            return;
        };
        app.global_marks.insert(name, (path, position));
    } else if name.is_ascii_lowercase() || matches!(name, '<' | '>' | '[' | ']') {
        app.buffer_mut().marks.insert(name, position);
    } else {
        app.error(format!("{name} is not a mark name"));
        return;
    }
    app.info(format!("mark {name} set"));
}

/// `Alt+←` and `Alt+→` — travel through the jump list.
pub fn jump(app: &mut App, back: bool) {
    let jump = if back {
        app.window_mut().jump_back()
    } else {
        app.window_mut().jump_forward()
    };
    let Some(jump) = jump else {
        app.info(if back {
            "already at the oldest jump"
        } else {
            "already at the newest jump"
        });
        return;
    };
    if jump.buffer >= app.buffers.len() {
        return;
    }
    if app.mode.is_visual() {
        enter_mode(app, Mode::Normal);
    }
    app.window_mut().show(jump.buffer);
    let position = app.buffer().document.clamp(jump.position, false);
    app.window_mut().clear_secondary_cursors();
    app.window_mut().cursor_mut().move_to(position, false);
}

/// `zt`, `zz` and `zb` — scroll so the caret's line sits at a row of the
/// window, without moving the caret.
pub fn scroll_caret(app: &mut App, row: ScreenRow) {
    let line = app.window().cursor().head.line;
    let height = usize::from(app.window().area.height).max(1);
    let window = app.window_mut();
    window.view.top_line = match row {
        ScreenRow::Top => line,
        ScreenRow::Middle => line.saturating_sub(height / 2),
        ScreenRow::Bottom => (line + 1).saturating_sub(height),
    };
}
