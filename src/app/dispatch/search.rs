//! Incremental search, its repeats, and `*` and `#`.

use anyhow::Result;

use super::{enter_mode, operator};
use crate::app::mode::Mode;
use crate::app::state::App;
use crate::clipboard::RegisterKind;
use crate::clipboard::registers::Register;

pub(super) const MATCH_COUNT_CAP: usize = 10_000;

pub(super) fn origin_offset(app: &App) -> usize {
    app.buffer()
        .document
        .pos_to_char(app.window().cursor().head)
}

pub(super) fn seek_from_origin(app: &mut App) {
    let origin = app.search.origin();
    let forward = app.search.forward;
    let Some(found) = app.search.find(&app.buffer().document, origin, forward) else {
        return;
    };
    let position = app.buffer().document.char_to_pos(found.start);
    app.window_mut().cursor_mut().move_to(position, false);
}

pub(super) fn cancel_search(app: &mut App) -> Result<()> {
    let origin = app.search.origin();
    let position = app.buffer().document.char_to_pos(origin);
    app.window_mut().cursor_mut().move_to(position, false);
    app.search.set_query(String::new());
    enter_mode(app, Mode::Normal);
    Ok(())
}

pub(super) fn repeat_search(app: &mut App, forward: bool) {
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

/// `*` and `#` — search for the word under the caret.
pub(super) fn search_word(app: &mut App, forward: bool, whole: bool) {
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
