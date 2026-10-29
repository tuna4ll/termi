//! The key bindings: one flat `match` per mode, vi where vi is unambiguous.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};

use super::{Action, LspAction, Operator, Pending, is_ctrl, is_plain};
use crate::app::lsp::MenuAction;
use crate::app::mode::Mode;
use crate::editor::cursor::Motion;
use crate::editor::window::{Axis, Side};
use crate::picker::PickerKind;

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
        KeyCode::Char('o') => Action::OpenPicker(PickerKind::Files),
        KeyCode::Char('g') => Action::OpenPicker(PickerKind::Grep),
        // Ctrl+/ arrives as `/` from terminals speaking the kitty protocol and
        // as the legacy 0x1F, which crossterm reports as Ctrl+7, elsewhere.
        KeyCode::Char('/' | '7' | '_') => Action::ToggleComment,
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

/// Normal-mode keys that are complete on their own and never begin a vi
/// command: chords, special keys and Alt combinations.
///
/// `None` hands the key to the command grammar.
pub fn normal(key: KeyEvent, pending: &mut Option<Pending>) -> Option<Action> {
    if window_prefix(key, pending) {
        return Some(Action::None);
    }
    if let Some(action) = word_delete(key) {
        return Some(action);
    }
    if let Some(action) = universal(key) {
        return Some(action);
    }
    if let Some(motion) = shift_motion(key) {
        return Some(Action::Select(motion));
    }
    if let Some(motion) = ctrl_motion(key) {
        return Some(Action::Move(motion));
    }
    if key.modifiers.contains(KeyModifiers::ALT) {
        return Some(alt(key));
    }
    Some(match key.code {
        KeyCode::Esc => Action::ClearCursors,
        KeyCode::F(2) => Action::RenamePrompt,
        KeyCode::Backspace => Action::DeleteBackward,
        KeyCode::Delete => Action::DeleteForward,
        KeyCode::PageDown => Action::Page {
            down: true,
            half: false,
        },
        KeyCode::PageUp => Action::Page {
            down: false,
            half: false,
        },
        _ => return None,
    })
}

/// Alt chords shared by normal and visual mode.
fn alt(key: KeyEvent) -> Action {
    match key.code {
        KeyCode::Down => Action::AddCursor { below: true },
        KeyCode::Up => Action::AddCursor { below: false },
        KeyCode::Left => Action::JumpBack,
        KeyCode::Right => Action::JumpForward,
        KeyCode::Char('v' | 'V') => Action::EnterMode(Mode::VisualBlock),
        _ => Action::None,
    }
}

pub fn pending(prefix: Pending, key: KeyEvent) -> Action {
    match prefix {
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

/// The keys an open completion menu takes for itself; `None` lets the key
/// through to insert mode.
pub fn menu(key: KeyEvent) -> Option<Action> {
    let ctrl = is_ctrl(key.modifiers);
    Some(Action::Menu(match key.code {
        KeyCode::Down | KeyCode::Tab => MenuAction::Next,
        KeyCode::Char('n') if ctrl => MenuAction::Next,
        KeyCode::Up | KeyCode::BackTab => MenuAction::Previous,
        KeyCode::Char('p') if ctrl => MenuAction::Previous,
        KeyCode::Enter => MenuAction::Accept,
        KeyCode::Char('y') if ctrl => MenuAction::Accept,
        KeyCode::Esc => MenuAction::Close,
        _ => return None,
    }))
}

pub fn insert(key: KeyEvent) -> Action {
    if is_ctrl(key.modifiers) && key.code == KeyCode::Char(' ') {
        return Action::Lsp(LspAction::Complete);
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

/// Visual-mode keys that are complete on their own; `None` hands the key to
/// the command grammar.
pub fn visual(key: KeyEvent, pending: &mut Option<Pending>) -> Option<Action> {
    if window_prefix(key, pending) {
        return Some(Action::None);
    }
    if let Some(action) = word_delete(key) {
        return Some(action);
    }
    if let Some(action) = universal(key) {
        return Some(action);
    }
    if let Some(motion) = shift_motion(key) {
        return Some(Action::Select(motion));
    }
    if let Some(motion) = ctrl_motion(key) {
        return Some(Action::Extend(motion));
    }
    if key.modifiers.contains(KeyModifiers::ALT) {
        return Some(alt(key));
    }
    Some(match key.code {
        KeyCode::Esc => Action::EnterMode(Mode::Normal),
        KeyCode::Delete | KeyCode::Backspace => Action::OperateSelection {
            operator: Operator::Delete,
            count: 1,
        },
        KeyCode::PageDown => Action::Page {
            down: true,
            half: false,
        },
        KeyCode::PageUp => Action::Page {
            down: false,
            half: false,
        },
        _ => return None,
    })
}

pub fn command(key: KeyEvent) -> Action {
    if is_ctrl(key.modifiers) {
        return match key.code {
            KeyCode::Char('p') => Action::CommandHistory { back: true },
            KeyCode::Char('n') => Action::CommandHistory { back: false },
            _ => Action::None,
        };
    }
    match key.code {
        KeyCode::Esc => Action::CommandCancel,
        KeyCode::Enter => Action::CommandSubmit,
        KeyCode::Backspace => Action::CommandBackspace,
        KeyCode::Tab => Action::CommandComplete { forward: true },
        KeyCode::BackTab => Action::CommandComplete { forward: false },
        KeyCode::Up => Action::CommandHistory { back: true },
        KeyCode::Down => Action::CommandHistory { back: false },
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
        KeyCode::Char('h') | KeyCode::Left => Action::TreeLeave,
        KeyCode::Backspace | KeyCode::Char('-') => Action::TreeRootUp,
        KeyCode::Char('j') | KeyCode::Down => Action::TreeMove(1),
        KeyCode::Char('k') | KeyCode::Up => Action::TreeMove(-1),
        KeyCode::PageDown => Action::TreeMove(10),
        KeyCode::PageUp => Action::TreeMove(-10),
        KeyCode::Char('g') | KeyCode::Home => Action::TreeMove(isize::MIN),
        KeyCode::Char('G') | KeyCode::End => Action::TreeMove(isize::MAX),
        KeyCode::Char('a') => Action::TreeCreate { directory: false },
        KeyCode::Char('A') => Action::TreeCreate { directory: true },
        KeyCode::Char('.') => Action::TreeToggleHidden,
        KeyCode::Char('r') => Action::TreeRename,
        KeyCode::Char('c') => Action::TreeCopy,
        KeyCode::Char('m') => Action::TreeMovePath,
        KeyCode::Char('d') | KeyCode::Delete => Action::TreeDelete,
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
        KeyCode::Up => Action::SearchHistory { back: true },
        KeyCode::Down => Action::SearchHistory { back: false },
        KeyCode::Char(ch) if is_plain(key.modifiers) => Action::SearchInput(ch),
        _ => Action::None,
    }
}

pub fn picker(key: KeyEvent) -> Action {
    if is_ctrl(key.modifiers) {
        return match key.code {
            KeyCode::Char('n') => Action::PickerMove(1),
            KeyCode::Char('p') => Action::PickerMove(-1),
            _ => Action::None,
        };
    }
    match key.code {
        KeyCode::Esc => Action::PickerCancel,
        KeyCode::Enter => Action::PickerSubmit,
        KeyCode::Backspace => Action::PickerBackspace,
        KeyCode::Down => Action::PickerMove(1),
        KeyCode::Up => Action::PickerMove(-1),
        KeyCode::PageDown => Action::PickerMove(10),
        KeyCode::PageUp => Action::PickerMove(-10),
        KeyCode::Char(ch) if is_plain(key.modifiers) => Action::PickerInput(ch),
        _ => Action::None,
    }
}

/// `:s///c` asks about one match at a time: yes, no, all, last or quit.
pub fn replace(key: KeyEvent) -> Action {
    match key.code {
        KeyCode::Char(answer @ ('y' | 'n' | 'a' | 'q' | 'l')) => Action::ReplaceAnswer(answer),
        KeyCode::Esc => Action::ReplaceAnswer('q'),
        _ => Action::None,
    }
}

pub fn confirm(key: KeyEvent) -> Action {
    match key.code {
        KeyCode::Char('d') if is_plain(key.modifiers) => Action::ConfirmDelete,
        KeyCode::Esc | KeyCode::Char('q') => Action::ConfirmCancel,
        _ => Action::None,
    }
}
