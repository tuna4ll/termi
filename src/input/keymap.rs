//! The key bindings: one flat `match` per mode, vi where vi is unambiguous.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};

use super::{Action, Pending, is_ctrl, is_plain};
use crate::app::mode::Mode;
use crate::editor::cursor::Motion;
use crate::editor::window::{Axis, Side};

fn window_prefix(key: KeyEvent, pending: &mut Option<Pending>) -> bool {
    if is_ctrl(key.modifiers) && key.code == KeyCode::Char('w') {
        *pending = Some(Pending::Window);
        return true;
    }
    false
}

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
        KeyCode::Char('c') => Action::Yank,
        KeyCode::Char('x') => Action::Cut,
        KeyCode::Char('v') => Action::Paste,
        KeyCode::Char('a') => Action::SelectAll,
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

fn shift_motion(key: KeyEvent) -> Option<Motion> {
    if !key.modifiers.contains(KeyModifiers::SHIFT) {
        return None;
    }
    if let Some(motion) = ctrl_motion(key) {
        return Some(motion);
    }
    Some(match key.code {
        KeyCode::Left => Motion::Left,
        KeyCode::Right => Motion::Right,
        KeyCode::Up => Motion::Up(1),
        KeyCode::Down => Motion::Down(1),
        KeyCode::Home => Motion::LineStart,
        KeyCode::End => Motion::LineEnd,
        _ => return None,
    })
}

fn ctrl_motion(key: KeyEvent) -> Option<Motion> {
    if !is_ctrl(key.modifiers) {
        return None;
    }
    Some(match key.code {
        KeyCode::Left => Motion::WordBackward,
        KeyCode::Right => Motion::WordForward,
        KeyCode::Home => Motion::DocStart,
        KeyCode::End => Motion::DocEnd,
        _ => return None,
    })
}

/// Terminals that send Ctrl+Backspace as `0x08` reach crossterm as Ctrl+H.
fn word_delete(key: KeyEvent) -> Option<Action> {
    if !is_ctrl(key.modifiers) {
        return None;
    }
    Some(match key.code {
        KeyCode::Backspace | KeyCode::Char('h') => Action::DeleteWordBackward,
        KeyCode::Delete => Action::DeleteWordForward,
        _ => return None,
    })
}

pub fn normal(key: KeyEvent, pending: &mut Option<Pending>) -> Action {
    if window_prefix(key, pending) {
        return Action::None;
    }
    if let Some(action) = word_delete(key) {
        return action;
    }
    if let Some(action) = universal(key) {
        return action;
    }
    if let Some(motion) = shift_motion(key) {
        return Action::Select(motion);
    }
    if let Some(motion) = ctrl_motion(key) {
        return Action::Move(motion);
    }
    if let Some(motion) = motion(key) {
        return Action::Move(motion);
    }
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

pub fn pending(prefix: Pending, key: KeyEvent) -> Action {
    match prefix {
        Pending::Key(prefix) => sequence(prefix, key),
        Pending::Window => window(key),
    }
}

/// Forward terminal keys verbatim while reserving Ctrl+W for pane management.
pub fn terminal(key: KeyEvent, pending: &mut Option<Pending>) -> Action {
    if window_prefix(key, pending) {
        Action::None
    } else {
        Action::TerminalInput(key)
    }
}

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
        KeyCode::Char('t' | 'T') => Action::OpenTerminal,
        _ => Action::None,
    }
}

const WHEEL_STEP: isize = 3;

pub fn mouse(event: MouseEvent) -> Action {
    let (x, y) = (event.column, event.row);
    match event.kind {
        MouseEventKind::Down(MouseButton::Left) => Action::ClickAt { x, y },
        MouseEventKind::Drag(MouseButton::Left) => Action::DragTo { x, y },
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

pub fn insert(key: KeyEvent) -> Action {
    if let Some(action) = word_delete(key) {
        return action;
    }
    if let Some(action) = universal(key) {
        return action;
    }
    if let Some(motion) = shift_motion(key) {
        return Action::Select(motion);
    }
    if let Some(motion) = ctrl_motion(key) {
        return Action::Move(motion);
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

        KeyCode::Char(ch) if is_plain(key.modifiers) => Action::Insert(ch),
        _ => Action::None,
    }
}

pub fn visual(key: KeyEvent, pending: &mut Option<Pending>) -> Action {
    if window_prefix(key, pending) {
        return Action::None;
    }
    if let Some(action) = word_delete(key) {
        return action;
    }
    if let Some(action) = universal(key) {
        return action;
    }
    if let Some(motion) = shift_motion(key) {
        return Action::Select(motion);
    }
    if let Some(motion) = ctrl_motion(key) {
        return Action::Extend(motion);
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

pub fn command(key: KeyEvent) -> Action {
    match key.code {
        KeyCode::Esc => Action::CommandCancel,
        KeyCode::Enter => Action::CommandSubmit,
        KeyCode::Backspace => Action::CommandBackspace,
        KeyCode::Char(ch) if is_plain(key.modifiers) => Action::CommandInput(ch),
        _ => Action::None,
    }
}

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

pub fn search(key: KeyEvent) -> Action {
    if is_ctrl(key.modifiers) {
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
