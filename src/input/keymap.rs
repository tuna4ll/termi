//! # Keymaps
//!
//! **Purpose:** the actual key bindings.
//!
//! **Responsibility:** one function per mode, each a flat `match` from key to
//! [`Action`]. A table rather than nested conditionals means the whole binding
//! set for a mode can be read — and audited for conflicts — in one screen.
//!
//! The bindings follow vi where vi is unambiguous, and Helix where vi is
//! awkward; the editing keys (`Ctrl+S`, `Ctrl+Q`, arrows, Home/End) also work
//! the way a modeless editor's do, so the editor is usable before the modal
//! bindings are learned.
//!
//! **Public API:** [`normal`], [`insert`], [`visual`], [`command`],
//! [`pending`].

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};

use super::{Action, Pending, is_ctrl, is_plain};
use crate::app::mode::Mode;
use crate::editor::cursor::Motion;
use crate::editor::window::{Axis, Side};

/// Start the `Ctrl+W` window sequence, if that is what this key is.
///
/// Checked in the modes where a chord cannot be text. Insert mode is left out
/// on purpose: nothing there would survive swallowing a keystroke to wait for a
/// second one.
fn window_prefix(key: KeyEvent, pending: &mut Option<Pending>) -> bool {
    if is_ctrl(key.modifiers) && key.code == KeyCode::Char('w') {
        *pending = Some(Pending::Window);
        return true;
    }
    false
}

/// Bindings shared by every mode: window-level keys that must always work.
fn universal(key: KeyEvent) -> Option<Action> {
    if !is_ctrl(key.modifiers) {
        return None;
    }
    Some(match key.code {
        KeyCode::Char('q') => Action::Quit,
        KeyCode::Char('s') => Action::Save,
        KeyCode::Char('n') => Action::CycleBuffer { forward: true },
        KeyCode::Char('p') => Action::CycleBuffer { forward: false },
        KeyCode::Char('z') => Action::Undo,
        KeyCode::Char('y' | 'r') => Action::Redo,
        KeyCode::Char('d') => Action::Page {
            down: true,
            half: true,
        },
        KeyCode::Char('u') => Action::Page {
            down: false,
            half: true,
        },
        KeyCode::Char('e') => Action::Scroll(1),
        KeyCode::Char('b') => Action::ToggleTree,
        _ => return None,
    })
}

/// Motions that mean the same thing in normal and visual mode.
fn motion(key: KeyEvent) -> Option<Motion> {
    if !is_plain(key.modifiers) {
        return None;
    }
    Some(match key.code {
        KeyCode::Char('h') | KeyCode::Left => Motion::Left,
        KeyCode::Char('l') | KeyCode::Right => Motion::Right,
        KeyCode::Char('k') | KeyCode::Up => Motion::Up(1),
        KeyCode::Char('j') | KeyCode::Down => Motion::Down(1),
        KeyCode::Char('w') => Motion::WordForward,
        KeyCode::Char('b') => Motion::WordBackward,
        KeyCode::Char('e') => Motion::WordEnd,
        KeyCode::Char('0') | KeyCode::Home => Motion::LineStart,
        KeyCode::Char('^') => Motion::LineFirstNonBlank,
        KeyCode::Char('$') | KeyCode::End => Motion::LineEnd,
        KeyCode::Char('G') => Motion::DocEnd,
        _ => return None,
    })
}

/// The motion a shifted arrow, Home or End asks for.
///
/// Only the keys a modeless editor also selects with are listed. The letter
/// motions are deliberately left out: `G` and `^` reach us with Shift set too,
/// and they are commands rather than selections.
fn shift_motion(key: KeyEvent) -> Option<Motion> {
    if !key.modifiers.contains(KeyModifiers::SHIFT) {
        return None;
    }
    // Ctrl widens the step to a word, as it does in every modeless editor.
    let by_word = is_ctrl(key.modifiers);
    Some(match key.code {
        KeyCode::Left if by_word => Motion::WordBackward,
        KeyCode::Right if by_word => Motion::WordForward,
        KeyCode::Left => Motion::Left,
        KeyCode::Right => Motion::Right,
        KeyCode::Up => Motion::Up(1),
        KeyCode::Down => Motion::Down(1),
        KeyCode::Home => Motion::LineStart,
        KeyCode::End => Motion::LineEnd,
        _ => return None,
    })
}

/// Normal mode: keys are commands.
///
/// `pending` is set when a key only makes sense as the first half of a sequence.
pub fn normal(key: KeyEvent, pending: &mut Option<Pending>) -> Action {
    if window_prefix(key, pending) {
        return Action::None;
    }
    if let Some(action) = universal(key) {
        return action;
    }
    if let Some(motion) = shift_motion(key) {
        return Action::Select(motion);
    }
    if let Some(motion) = motion(key) {
        return Action::Move(motion);
    }
    // Alt with an arrow drops a second cursor, the way most modeless editors do
    // it — the modal alternative would need a prefix nobody would guess.
    if key.modifiers.contains(KeyModifiers::ALT) {
        return match key.code {
            KeyCode::Down => Action::AddCursor { below: true },
            KeyCode::Up => Action::AddCursor { below: false },
            _ => Action::None,
        };
    }
    if !is_plain(key.modifiers) {
        return Action::None;
    }
    match key.code {
        KeyCode::Char('i') => Action::EnterMode(Mode::Insert),
        KeyCode::Char('I') => Action::InsertAtLineStart,
        KeyCode::Char('a') => Action::AppendAfter,
        KeyCode::Char('A') => Action::AppendAtLineEnd,
        KeyCode::Char('o') => Action::OpenLineBelow,
        KeyCode::Char('O') => Action::OpenLineAbove,
        KeyCode::Char('v') => Action::EnterMode(Mode::Visual),
        KeyCode::Char('V') => Action::EnterMode(Mode::VisualLine),
        KeyCode::Char(':') => Action::EnterMode(Mode::Command),

        KeyCode::Char('x') | KeyCode::Delete => Action::DeleteForward,
        KeyCode::Backspace => Action::DeleteBackward,
        KeyCode::Char('u') => Action::Undo,
        KeyCode::Char('p') => Action::Paste,

        KeyCode::Char('/') => Action::SearchStart { forward: true },
        KeyCode::Char('?') => Action::SearchStart { forward: false },
        KeyCode::Char('n') => Action::SearchRepeat { forward: true },
        KeyCode::Char('N') => Action::SearchRepeat { forward: false },

        // Sequences: the second key decides what happens.
        KeyCode::Char(prefix @ ('g' | 'd' | 'y')) => {
            *pending = Some(Pending::Key(prefix));
            Action::None
        }

        KeyCode::PageDown => Action::Page {
            down: true,
            half: false,
        },
        KeyCode::PageUp => Action::Page {
            down: false,
            half: false,
        },
        KeyCode::Esc => Action::ClearCursors,
        _ => Action::None,
    }
}

/// The second key of a sequence started in normal or visual mode.
pub fn pending(prefix: Pending, key: KeyEvent) -> Action {
    match prefix {
        Pending::Key(prefix) => sequence(prefix, key),
        Pending::Window => window(key),
    }
}

/// The second key of a plain two-key sequence.
fn sequence(prefix: char, key: KeyEvent) -> Action {
    match (prefix, key.code) {
        ('g', KeyCode::Char('g')) => Action::Move(Motion::DocStart),
        ('g', KeyCode::Char('e')) => Action::Move(Motion::DocEnd),
        ('g', KeyCode::Char('h')) => Action::Move(Motion::LineStart),
        ('g', KeyCode::Char('l')) => Action::Move(Motion::LineEnd),
        ('g', KeyCode::Char('s')) => Action::Move(Motion::LineFirstNonBlank),
        ('d', KeyCode::Char('d')) => Action::Delete,
        ('y', KeyCode::Char('y')) => Action::Yank,
        _ => Action::None,
    }
}

/// The key after `Ctrl+W`.
///
/// The bindings follow vi, including its naming: `s` splits along a horizontal
/// line and so stacks the windows, while `v` splits along a vertical one and
/// puts them side by side. Ctrl is ignored, so holding it down through the
/// whole sequence — `Ctrl+W Ctrl+V` — works as well as letting go.
fn window(key: KeyEvent) -> Action {
    match key.code {
        KeyCode::Char('s' | 'S') => Action::SplitWindow {
            axis: Axis::Vertical,
        },
        KeyCode::Char('v' | 'V') => Action::SplitWindow {
            axis: Axis::Horizontal,
        },
        KeyCode::Char('c' | 'q') => Action::CloseWindow,
        KeyCode::Char('o') => Action::OnlyWindow,
        KeyCode::Char('w') => Action::CycleWindow,

        KeyCode::Char('h') | KeyCode::Left => Action::FocusWindow(Side::Left),
        KeyCode::Char('j') | KeyCode::Down => Action::FocusWindow(Side::Down),
        KeyCode::Char('k') | KeyCode::Up => Action::FocusWindow(Side::Up),
        KeyCode::Char('l') | KeyCode::Right => Action::FocusWindow(Side::Right),

        KeyCode::Char('+') => Action::ResizeWindow {
            axis: Axis::Vertical,
            delta: 1,
        },
        KeyCode::Char('-') => Action::ResizeWindow {
            axis: Axis::Vertical,
            delta: -1,
        },
        KeyCode::Char('>') => Action::ResizeWindow {
            axis: Axis::Horizontal,
            delta: 1,
        },
        KeyCode::Char('<') => Action::ResizeWindow {
            axis: Axis::Horizontal,
            delta: -1,
        },
        KeyCode::Char('=') => Action::EqualiseWindows,
        _ => Action::None,
    }
}

/// How many lines one notch of the wheel moves.
const WHEEL_STEP: isize = 3;

/// Mouse events, which are about windows rather than about text.
///
/// A click moves the focus and the wheel scrolls; neither places the caret.
/// Pointing at a window is an unambiguous instruction about *which* window,
/// while pointing at a character is not — a click inside a wrapped line, a tab
/// or a double-width glyph lands between positions rather than on one.
pub fn mouse(event: MouseEvent) -> Action {
    let (x, y) = (event.column, event.row);
    match event.kind {
        MouseEventKind::Down(MouseButton::Left) => Action::FocusAt { x, y },
        MouseEventKind::ScrollDown => Action::ScrollAt {
            x,
            y,
            delta: WHEEL_STEP,
        },
        MouseEventKind::ScrollUp => Action::ScrollAt {
            x,
            y,
            delta: -WHEEL_STEP,
        },
        _ => Action::None,
    }
}

/// Insert mode: printable keys become text.
pub fn insert(key: KeyEvent) -> Action {
    if let Some(action) = universal(key) {
        return action;
    }
    // A shifted arrow selects instead of moving, which drops out of insert
    // mode: the selection has to be visible, and only visual mode paints one.
    if let Some(motion) = shift_motion(key) {
        return Action::Select(motion);
    }
    match key.code {
        KeyCode::Esc => Action::EnterMode(Mode::Normal),
        KeyCode::Enter => Action::InsertNewline,
        KeyCode::Tab => Action::InsertIndent,
        KeyCode::Backspace => Action::DeleteBackward,
        KeyCode::Delete => Action::DeleteForward,

        KeyCode::Left => Action::Move(Motion::Left),
        KeyCode::Right => Action::Move(Motion::Right),
        KeyCode::Up => Action::Move(Motion::Up(1)),
        KeyCode::Down => Action::Move(Motion::Down(1)),
        KeyCode::Home => Action::Move(Motion::LineStart),
        KeyCode::End => Action::Move(Motion::LineEnd),
        KeyCode::PageUp => Action::Page {
            down: false,
            half: false,
        },
        KeyCode::PageDown => Action::Page {
            down: true,
            half: false,
        },

        // Anything else printable is text. Control combinations were already
        // handled above, so reaching here with a modifier means Shift or AltGr,
        // both of which are part of the character crossterm reports.
        KeyCode::Char(ch) if is_plain(key.modifiers) => Action::Insert(ch),
        _ => Action::None,
    }
}

/// Visual mode: motions extend the selection.
pub fn visual(key: KeyEvent, pending: &mut Option<Pending>) -> Action {
    if window_prefix(key, pending) {
        return Action::None;
    }
    if let Some(action) = universal(key) {
        return action;
    }
    if let Some(motion) = shift_motion(key) {
        return Action::Select(motion);
    }
    if let Some(motion) = motion(key) {
        return Action::Extend(motion);
    }
    if !is_plain(key.modifiers) {
        return Action::None;
    }
    match key.code {
        KeyCode::Esc | KeyCode::Char('v') => Action::EnterMode(Mode::Normal),
        KeyCode::Char('V') => Action::EnterMode(Mode::VisualLine),
        KeyCode::Char(':') => Action::EnterMode(Mode::Command),
        KeyCode::Char('d' | 'x') | KeyCode::Delete => Action::Delete,
        KeyCode::Char('y') => Action::Yank,
        KeyCode::Char('c') => Action::Cut,
        KeyCode::Char('p') => Action::Paste,
        KeyCode::Char('i') => Action::EnterMode(Mode::Insert),
        KeyCode::Char('g') => {
            *pending = Some(Pending::Key('g'));
            Action::None
        }
        KeyCode::PageDown => Action::Page {
            down: true,
            half: false,
        },
        KeyCode::PageUp => Action::Page {
            down: false,
            half: false,
        },
        _ => Action::None,
    }
}

/// Command mode: the command bar owns the keyboard.
pub fn command(key: KeyEvent) -> Action {
    match key.code {
        KeyCode::Esc => Action::CommandCancel,
        KeyCode::Enter => Action::CommandSubmit,
        KeyCode::Backspace => Action::CommandBackspace,
        KeyCode::Char(ch) if is_plain(key.modifiers) => Action::CommandInput(ch),
        _ => Action::None,
    }
}

/// Tree mode: the file browser has the keyboard.
pub fn tree(key: KeyEvent) -> Action {
    if is_ctrl(key.modifiers) && key.code == KeyCode::Char('b') {
        return Action::ToggleTree;
    }
    match key.code {
        KeyCode::Esc | KeyCode::Char('q') => Action::EnterMode(Mode::Normal),
        KeyCode::Enter | KeyCode::Char('l' | ' ') | KeyCode::Right => Action::TreeActivate,
        KeyCode::Char('j') | KeyCode::Down => Action::TreeMove(1),
        KeyCode::Char('k') | KeyCode::Up => Action::TreeMove(-1),
        KeyCode::PageDown => Action::TreeMove(10),
        KeyCode::PageUp => Action::TreeMove(-10),
        KeyCode::Char('g') | KeyCode::Home => Action::TreeMove(isize::MIN),
        KeyCode::Char('G') | KeyCode::End => Action::TreeMove(isize::MAX),
        KeyCode::Char('a') => Action::TreeCreate { directory: false },
        KeyCode::Char('A') => Action::TreeCreate { directory: true },
        _ => Action::None,
    }
}

/// Search mode: the query is edited while the document follows along.
pub fn search(key: KeyEvent) -> Action {
    if is_ctrl(key.modifiers) {
        // Stepping between matches without leaving the prompt.
        return match key.code {
            KeyCode::Char('n') => Action::SearchRepeat { forward: true },
            KeyCode::Char('p') => Action::SearchRepeat { forward: false },
            _ => Action::None,
        };
    }
    match key.code {
        KeyCode::Esc => Action::SearchCancel,
        KeyCode::Enter => Action::SearchSubmit,
        KeyCode::Backspace => Action::SearchBackspace,
        KeyCode::Down => Action::SearchRepeat { forward: true },
        KeyCode::Up => Action::SearchRepeat { forward: false },
        KeyCode::Char(ch) if is_plain(key.modifiers) => Action::SearchInput(ch),
        _ => Action::None,
    }
}
