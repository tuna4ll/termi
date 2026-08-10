//! # Input
//!
//! **Purpose:** turn key presses into intentions.
//!
//! **Responsibility:** own the [`Action`] vocabulary and the pending-key state
//! needed for multi-key sequences like `gg`. Nothing here touches a buffer —
//! translation and execution are separated so that keys can be remapped, or
//! actions replayed from a macro or a command, without duplicating any editing
//! logic.
//!
//! **Public API:** [`Action`], [`Input`].

pub mod keymap;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent};

use crate::app::mode::Mode;
use crate::editor::cursor::Motion;
use crate::editor::window::{Axis, Side};

/// The first key of a multi-key sequence, held while the second is awaited.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pending {
    /// A plain prefix, such as the `g` of `gg` or the `d` of `dd`.
    Key(char),
    /// `Ctrl+W`, which prefixes every window command.
    Window,
}

/// Something the user asked the editor to do.
///
/// Actions are deliberately coarse — one per user-visible operation — so the
/// dispatcher reads like a list of features rather than a state machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// The key means nothing in this mode.
    None,
    /// Switch modes.
    EnterMode(Mode),
    /// Move every cursor.
    Move(Motion),
    /// Move every cursor, extending the selection.
    Extend(Motion),
    /// Scroll without moving the caret; negative is upwards.
    Scroll(isize),
    /// Move a page up or down, sized from the current window.
    Page { down: bool, half: bool },

    /// Insert a character at every cursor.
    Insert(char),
    /// Break the line.
    InsertNewline,
    /// Insert one indentation step.
    InsertIndent,
    /// Delete backwards.
    DeleteBackward,
    /// Delete forwards.
    DeleteForward,
    /// Delete the current selection or line.
    Delete,
    /// Open a new line below the current one and start inserting.
    OpenLineBelow,
    /// Open a new line above the current one and start inserting.
    OpenLineAbove,
    /// Enter insert mode after the caret.
    AppendAfter,
    /// Enter insert mode at the end of the line.
    AppendAtLineEnd,
    /// Enter insert mode at the first non-blank character.
    InsertAtLineStart,

    /// Undo one step.
    Undo,
    /// Redo one step.
    Redo,
    /// Copy the selection or line.
    Yank,
    /// Cut the selection or line.
    Cut,
    /// Paste the clipboard.
    Paste,

    /// Add a cursor on the line above or below.
    AddCursor { below: bool },
    /// Collapse back to a single cursor.
    ClearCursors,

    /// Focus the next or previous buffer.
    CycleBuffer { forward: bool },

    /// Divide the focused window in two.
    SplitWindow { axis: Axis },
    /// Close the focused window, leaving its buffer open.
    CloseWindow,
    /// Close every window except the focused one.
    OnlyWindow,
    /// Move the focus to the neighbouring window on one side.
    FocusWindow(Side),
    /// Move the focus to the next window in screen order.
    CycleWindow,
    /// Grow or shrink the focused window along one axis.
    ResizeWindow { axis: Axis, delta: i16 },
    /// Give every window an equal share of the screen again.
    EqualiseWindows,
    /// Aim the keyboard at whichever window covers this cell.
    ///
    /// Carried as coordinates rather than a window id because the input layer
    /// holds no state: only the application knows where the windows are.
    FocusAt { x: u16, y: u16 },
    /// Scroll the window covering this cell, focused or not.
    ScrollAt { x: u16, y: u16, delta: isize },

    /// Write the active buffer.
    Save,
    /// Leave the editor.
    Quit,

    /// Show or hide the file tree panel.
    ToggleTree,
    /// Move the file tree selection by `delta` rows.
    TreeMove(isize),
    /// Open the selected file, or expand the selected directory.
    TreeActivate,

    /// Open the search prompt, searching forwards or backwards.
    SearchStart { forward: bool },
    /// Append a character to the search query.
    SearchInput(char),
    /// Remove the last character of the search query.
    SearchBackspace,
    /// Accept the search and keep the caret on the match.
    SearchSubmit,
    /// Abandon the search and go back to where it started.
    SearchCancel,
    /// Jump to the next or previous match of the last search.
    SearchRepeat { forward: bool },

    /// Append a character to the command line.
    CommandInput(char),
    /// Remove the last character of the command line.
    CommandBackspace,
    /// Run the command line.
    CommandSubmit,
    /// Abandon the command line.
    CommandCancel,
}

/// Key translation, including the state needed for multi-key sequences.
#[derive(Debug, Default)]
pub struct Input {
    /// First key of a pending sequence, such as the `g` of `gg`.
    ///
    /// Held here rather than in the application state because it is purely an
    /// input-layer concern and must be discarded whenever the mode changes.
    pending: Option<Pending>,
}

impl Input {
    /// Translate one key press in the context of `mode`.
    pub fn handle(&mut self, key: KeyEvent, mode: Mode) -> Action {
        let key = normalise_alt_gr(key);
        if let Some(prefix) = self.pending.take() {
            return keymap::pending(prefix, key);
        }
        let action = match mode {
            Mode::Normal => keymap::normal(key, &mut self.pending),
            Mode::Insert => keymap::insert(key),
            Mode::Visual | Mode::VisualLine => keymap::visual(key, &mut self.pending),
            Mode::Command => keymap::command(key),
            Mode::Search => keymap::search(key),
            Mode::Tree => keymap::tree(key),
        };
        if matches!(action, Action::EnterMode(_)) {
            self.pending = None;
        }
        action
    }

    /// Translate one mouse event.
    ///
    /// Reaching for the mouse abandons a half-typed key sequence: the second
    /// key of `Ctrl+W` is not going to arrive.
    pub fn handle_mouse(&mut self, event: MouseEvent) -> Action {
        self.pending = None;
        keymap::mouse(event)
    }
}

/// Strip the phantom Ctrl+Alt that Windows puts on an AltGr character.
///
/// Windows implements AltGr as right-Alt plus left-Ctrl, and the console hands
/// crossterm both of those flags alongside the character the layout actually
/// produced. So `#` on a Turkish (or German, French, Polish, …) layout arrives
/// as `Char('#')` with `CONTROL | ALT`, every keymap rejects it as a modified
/// key, and the character is impossible to type — while the same character
/// loaded from a file displays fine.
///
/// The character has already been resolved by the layout at this point, so the
/// two modifiers carry no further meaning and are dropped. Nothing is bound to
/// Ctrl+Alt, so no chord is lost; Shift is left alone because [`is_plain`]
/// already tolerates it.
#[must_use]
fn normalise_alt_gr(mut key: KeyEvent) -> KeyEvent {
    let alt_gr = KeyModifiers::CONTROL | KeyModifiers::ALT;
    if key.modifiers.contains(alt_gr)
        && let KeyCode::Char(ch) = key.code
        && !ch.is_control()
    {
        key.modifiers.remove(alt_gr);
    }
    key
}

/// Whether a key carries no modifier that changes its meaning.
///
/// Shift is ignored on purpose: crossterm already reports the shifted character,
/// so `Char('G')` arrives with `SHIFT` set and must still count as plain.
#[must_use]
pub(crate) fn is_plain(modifiers: KeyModifiers) -> bool {
    modifiers.difference(KeyModifiers::SHIFT).is_empty()
}

/// Whether a key is pressed with Control and nothing else.
#[must_use]
pub(crate) fn is_ctrl(modifiers: KeyModifiers) -> bool {
    modifiers.contains(KeyModifiers::CONTROL)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::cursor::Motion;

    /// AltGr as Windows reports it: the layout's character plus Ctrl and Alt.
    fn alt_gr(ch: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(ch), KeyModifiers::CONTROL | KeyModifiers::ALT)
    }

    fn plain(ch: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE)
    }

    #[test]
    fn alt_gr_characters_are_inserted_as_text() {
        let mut input = Input::default();
        for ch in ['#', '$', '{', '}', '[', ']', '\\', '@', '€'] {
            assert_eq!(input.handle(alt_gr(ch), Mode::Insert), Action::Insert(ch));
        }
    }

    #[test]
    fn alt_gr_characters_reach_the_command_and_search_prompts() {
        let mut input = Input::default();
        assert_eq!(
            input.handle(alt_gr('#'), Mode::Command),
            Action::CommandInput('#')
        );
        assert_eq!(
            input.handle(alt_gr('$'), Mode::Search),
            Action::SearchInput('$')
        );
    }

    #[test]
    fn an_alt_gr_character_keeps_its_normal_mode_binding() {
        let mut input = Input::default();
        assert_eq!(
            input.handle(alt_gr('$'), Mode::Normal),
            Action::Move(Motion::LineEnd)
        );
    }

    #[test]
    fn control_chords_still_win_over_text() {
        let mut input = Input::default();
        let ctrl_s = KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL);
        assert_eq!(input.handle(ctrl_s, Mode::Insert), Action::Save);
        assert_eq!(input.handle(plain('a'), Mode::Insert), Action::Insert('a'));
    }
}
