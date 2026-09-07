//! Keys to intentions: the `Action` vocabulary and multi-key sequence state.

pub mod keymap;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent};

use crate::app::mode::Mode;
use crate::editor::cursor::Motion;
use crate::editor::window::{Axis, Side};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pending {
    Key(char),
    Window,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    None,
    EnterMode(Mode),
    Move(Motion),
    Extend(Motion),
    Select(Motion),
    SelectAll,
    Scroll(isize),
    Page { down: bool, half: bool },

    Insert(char),
    InsertNewline,
    InsertIndent,
    DeleteBackward,
    DeleteForward,
    DeleteWordBackward,
    DeleteWordForward,
    Delete,
    OpenLineBelow,
    OpenLineAbove,
    AppendAfter,
    AppendAtLineEnd,
    InsertAtLineStart,

    Undo,
    Redo,
    Yank,
    Cut,
    Paste,

    AddCursor { below: bool },
    ClearCursors,

    CycleBuffer { forward: bool },

    SplitWindow { axis: Axis },
    CloseWindow,
    OnlyWindow,
    FocusWindow(Side),
    CycleWindow,
    ResizeWindow { axis: Axis, delta: i16 },
    EqualiseWindows,
    ClickAt { x: u16, y: u16 },
    DragTo { x: u16, y: u16 },
    ScrollAt { x: u16, y: u16, delta: isize },

    Save,
    Quit,

    ToggleTree,
    TreeMove(isize),
    TreeActivate,
    TreeCreate { directory: bool },

    SearchStart { forward: bool },
    SearchInput(char),
    SearchBackspace,
    SearchSubmit,
    SearchCancel,
    SearchRepeat { forward: bool },

    CommandInput(char),
    CommandBackspace,
    CommandSubmit,
    CommandCancel,
}

#[derive(Debug, Default)]
pub struct Input {
    pending: Option<Pending>,
}

impl Input {
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

    pub fn handle_mouse(&mut self, event: MouseEvent) -> Action {
        self.pending = None;
        keymap::mouse(event)
    }
}

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

#[must_use]
pub(crate) fn is_plain(modifiers: KeyModifiers) -> bool {
    modifiers.difference(KeyModifiers::SHIFT).is_empty()
}

#[must_use]
pub(crate) fn is_ctrl(modifiers: KeyModifiers) -> bool {
    modifiers.contains(KeyModifiers::CONTROL)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::cursor::Motion;

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

    #[test]
    fn a_shifted_arrow_selects_in_every_editing_mode() {
        let mut input = Input::default();
        let shift_right = KeyEvent::new(KeyCode::Right, KeyModifiers::SHIFT);
        for mode in [Mode::Normal, Mode::Insert, Mode::Visual] {
            assert_eq!(
                input.handle(shift_right, mode),
                Action::Select(Motion::Right),
                "shift+right should select in {}",
                mode.name()
            );
        }
    }

    #[test]
    fn ctrl_widens_a_shifted_arrow_to_a_word() {
        let mut input = Input::default();
        let key = KeyEvent::new(KeyCode::Left, KeyModifiers::SHIFT | KeyModifiers::CONTROL);
        assert_eq!(
            input.handle(key, Mode::Normal),
            Action::Select(Motion::WordBackward)
        );
    }

    #[test]
    fn an_unshifted_arrow_still_only_moves() {
        let mut input = Input::default();
        let right = KeyEvent::new(KeyCode::Right, KeyModifiers::NONE);
        assert_eq!(
            input.handle(right, Mode::Normal),
            Action::Move(Motion::Right)
        );
    }

    #[test]
    fn a_shifted_letter_keeps_its_command_meaning() {
        let mut input = Input::default();
        let g = KeyEvent::new(KeyCode::Char('G'), KeyModifiers::SHIFT);
        assert_eq!(input.handle(g, Mode::Normal), Action::Move(Motion::DocEnd));
    }

    #[test]
    fn a_ctrl_arrow_steps_a_whole_word() {
        let mut input = Input::default();
        for mode in [Mode::Normal, Mode::Insert] {
            let left = KeyEvent::new(KeyCode::Left, KeyModifiers::CONTROL);
            let right = KeyEvent::new(KeyCode::Right, KeyModifiers::CONTROL);
            assert_eq!(input.handle(left, mode), Action::Move(Motion::WordBackward));
            assert_eq!(input.handle(right, mode), Action::Move(Motion::WordForward));
        }
    }

    #[test]
    fn a_ctrl_arrow_extends_the_selection_in_visual_mode() {
        let mut input = Input::default();
        let right = KeyEvent::new(KeyCode::Right, KeyModifiers::CONTROL);
        assert_eq!(
            input.handle(right, Mode::Visual),
            Action::Extend(Motion::WordForward)
        );
    }

    #[test]
    fn ctrl_home_and_end_reach_the_ends_of_the_file() {
        let mut input = Input::default();
        let home = KeyEvent::new(KeyCode::Home, KeyModifiers::CONTROL);
        let end = KeyEvent::new(KeyCode::End, KeyModifiers::CONTROL);
        assert_eq!(
            input.handle(home, Mode::Insert),
            Action::Move(Motion::DocStart)
        );
        assert_eq!(
            input.handle(end, Mode::Insert),
            Action::Move(Motion::DocEnd)
        );

        let shifted = KeyEvent::new(KeyCode::End, KeyModifiers::CONTROL | KeyModifiers::SHIFT);
        assert_eq!(
            input.handle(shifted, Mode::Insert),
            Action::Select(Motion::DocEnd)
        );
    }

    #[test]
    fn ctrl_backspace_and_ctrl_delete_remove_a_word() {
        let mut input = Input::default();
        let backspace = KeyEvent::new(KeyCode::Backspace, KeyModifiers::CONTROL);
        let delete = KeyEvent::new(KeyCode::Delete, KeyModifiers::CONTROL);
        for mode in [Mode::Normal, Mode::Insert, Mode::Visual] {
            assert_eq!(input.handle(backspace, mode), Action::DeleteWordBackward);
            assert_eq!(input.handle(delete, mode), Action::DeleteWordForward);
        }
    }

    #[test]
    fn a_terminal_spelling_ctrl_backspace_as_ctrl_h_still_works() {
        let mut input = Input::default();
        let ctrl_h = KeyEvent::new(KeyCode::Char('h'), KeyModifiers::CONTROL);
        assert_eq!(
            input.handle(ctrl_h, Mode::Insert),
            Action::DeleteWordBackward
        );
        assert_eq!(
            input.handle(ctrl_h, Mode::Normal),
            Action::DeleteWordBackward
        );
    }

    #[test]
    fn a_plain_backspace_still_removes_one_character() {
        let mut input = Input::default();
        let backspace = KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE);
        assert_eq!(
            input.handle(backspace, Mode::Insert),
            Action::DeleteBackward
        );
    }

    #[test]
    fn tree_creation_has_file_and_directory_shortcuts() {
        let mut input = Input::default();

        assert_eq!(
            input.handle(plain('a'), Mode::Tree),
            Action::TreeCreate { directory: false }
        );
        assert_eq!(
            input.handle(plain('A'), Mode::Tree),
            Action::TreeCreate { directory: true }
        );
    }
}
