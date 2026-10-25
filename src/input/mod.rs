//! Keys to intentions: the `Action` vocabulary and multi-key sequence state.

pub mod grammar;
pub mod keymap;
pub mod notation;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent};

use crate::app::mode::Mode;
use crate::editor::cursor::Motion;
use crate::editor::textobject::TextObject;
use crate::editor::window::{Axis, Side};
use crate::picker::PickerKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pending {
    Window,
}

/// How an operator changes letter case.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaseChange {
    /// `gu` and visual `u`.
    Lower,
    /// `gU` and visual `U`.
    Upper,
    /// `g~` and `~`.
    Toggle,
}

impl CaseChange {
    /// Apply to a piece of text.
    #[must_use]
    pub fn apply(self, text: &str) -> String {
        match self {
            Self::Lower => text.to_lowercase(),
            Self::Upper => text.to_uppercase(),
            Self::Toggle => text
                .chars()
                .flat_map(|ch| {
                    let flipped: Vec<char> = if ch.is_uppercase() {
                        ch.to_lowercase().collect()
                    } else {
                        ch.to_uppercase().collect()
                    };
                    flipped
                })
                .collect(),
        }
    }
}

/// What a vi operator does to the span it is given.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operator {
    /// `d`.
    Delete,
    /// `c` — delete, then insert.
    Change,
    /// `y`.
    Yank,
    /// `>`.
    Indent,
    /// `<`.
    Dedent,
    /// `gu`, `gU` and `g~`.
    Case(CaseChange),
    /// `gc` — toggle line comments.
    Comment,
}

impl Operator {
    /// The operator a key starts, if it starts one on its own.
    #[must_use]
    pub const fn from_key(key: Option<char>) -> Option<Self> {
        Some(match key {
            Some('d') => Self::Delete,
            Some('c') => Self::Change,
            Some('y') => Self::Yank,
            Some('>') => Self::Indent,
            Some('<') => Self::Dedent,
            _ => return None,
        })
    }

    /// The operator `g` followed by `key` starts.
    #[must_use]
    pub const fn from_g_key(key: char) -> Option<Self> {
        Some(match key {
            'u' => Self::Case(CaseChange::Lower),
            'U' => Self::Case(CaseChange::Upper),
            '~' => Self::Case(CaseChange::Toggle),
            'c' => Self::Comment,
            _ => return None,
        })
    }

    /// The key that, typed after the operator, means "this line": `dd`, `gUU`.
    #[must_use]
    pub const fn doubled(self) -> Option<char> {
        Some(match self {
            Self::Delete => 'd',
            Self::Change | Self::Comment => 'c',
            Self::Yank => 'y',
            Self::Indent => '>',
            Self::Dedent => '<',
            Self::Case(CaseChange::Lower) => 'u',
            Self::Case(CaseChange::Upper) => 'U',
            Self::Case(CaseChange::Toggle) => '~',
        })
    }

    /// Whether the operator changes text, as opposed to only reading it.
    #[must_use]
    pub const fn changes_text(self) -> bool {
        !matches!(self, Self::Yank)
    }
}

/// An `f`, `t`, `F` or `T` search, kept so `;` and `,` can repeat it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FindSpec {
    /// The character looked for.
    pub target: char,
    /// Search to the right.
    pub forward: bool,
    /// Stop one short of the character.
    pub till: bool,
}

impl FindSpec {
    /// The same search in the other direction, for `,`.
    #[must_use]
    pub const fn reversed(self) -> Self {
        Self {
            forward: !self.forward,
            ..self
        }
    }
}

/// A row of the window, for `H`, `M`, `L` and `zt`, `zz`, `zb`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScreenRow {
    /// The first visible line.
    Top,
    /// The middle of the window.
    Middle,
    /// The last visible line.
    Bottom,
}

/// Somewhere the caret can be sent.
///
/// Plain [`Motion`]s are answered by the document alone; the rest need the
/// application — marks live in buffers, screen rows depend on the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Goal {
    /// A motion the cursor can work out by itself.
    Motion(Motion),
    /// `f`, `t`, `F` or `T`.
    Find(FindSpec),
    /// `;` or `,`: a find repeated without becoming the one to repeat.
    Repeat(FindSpec),
    /// `'a` (the line) or `` `a `` (the exact place).
    Mark {
        /// Which mark.
        name: char,
        /// Whether to land on the column too.
        exact: bool,
    },
    /// `H`, `M` or `L`.
    Screen(ScreenRow),
}

/// What an operator acts on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// The doubled operator: whole lines, starting at the caret's.
    Lines,
    /// From the caret to wherever a goal lands.
    Goal(Goal),
    /// A text object around the caret.
    Object {
        /// Which object.
        object: TextObject,
        /// `i` rather than `a`.
        inner: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    None,
    EnterMode(Mode),
    Move(Motion),
    Goto {
        goal: Goal,
        count: usize,
    },
    Operate {
        operator: Operator,
        target: Target,
        count: usize,
    },
    OperateSelection {
        operator: Operator,
        count: usize,
    },
    SelectObject {
        object: TextObject,
        inner: bool,
        count: usize,
    },
    Put {
        before: bool,
        count: usize,
    },
    Repeat {
        count: usize,
        action: Box<Action>,
    },
    WithRegister {
        register: char,
        action: Box<Action>,
    },
    RepeatChange(Option<usize>),
    ReplaceChar {
        ch: char,
        count: usize,
    },
    ToggleCaseChars(usize),
    Join {
        count: usize,
        spaces: bool,
    },
    JoinSelection {
        spaces: bool,
    },
    ReplaceSelection(char),
    SwapAnchor,
    BlockInsert {
        append: bool,
    },
    SetMark(char),
    RecordMacro(char),
    StopMacro,
    PlayMacro {
        register: char,
        count: usize,
    },
    SearchWord {
        forward: bool,
        whole: bool,
    },
    RunCommand(String),
    ScrollCaret(ScreenRow),
    Reselect,
    JumpBack,
    JumpForward,
    InsertText(String),
    ToggleComment,
    Extend(Motion),
    Select(Motion),
    SelectAll,
    Scroll(isize),
    Page {
        down: bool,
        half: bool,
    },

    Insert(char),
    InsertNewline,
    InsertIndent,
    DeleteBackward,
    DeleteForward,
    DeleteWordBackward,
    DeleteWordForward,
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

    AddCursor {
        below: bool,
    },
    ClearCursors,

    CycleBuffer {
        forward: bool,
    },
    OpenPicker(PickerKind),
    PickerInput(char),
    PickerBackspace,
    PickerMove(isize),
    PickerSubmit,
    PickerCancel,

    SplitWindow {
        axis: Axis,
    },
    CloseWindow,
    OnlyWindow,
    FocusWindow(Side),
    CycleWindow,
    ResizeWindow {
        axis: Axis,
        delta: i16,
    },
    EqualiseWindows,
    OpenTerminal,
    TerminalInput(KeyEvent),
    ClickAt {
        x: u16,
        y: u16,
    },
    DragTo {
        x: u16,
        y: u16,
    },
    ScrollAt {
        x: u16,
        y: u16,
        delta: isize,
    },

    Save,
    Quit,

    ToggleTree,
    TreeMove(isize),
    TreeActivate,
    TreeCreate {
        directory: bool,
    },
    TreeToggleHidden,
    TreeRename,
    TreeCopy,
    TreeMovePath,
    TreeDelete,

    ConfirmDelete,
    ConfirmCancel,

    SearchStart {
        forward: bool,
    },
    SearchInput(char),
    SearchBackspace,
    SearchSubmit,
    SearchCancel,
    SearchRepeat {
        forward: bool,
    },

    CommandInput(char),
    CommandBackspace,
    CommandSubmit,
    CommandCancel,
}

#[derive(Debug, Default)]
pub struct Input {
    pending: Option<Pending>,
    /// Keys of a normal or visual mode command still being typed.
    keys: Vec<KeyEvent>,
    /// The last `f`, `t`, `F` or `T`, for `;` and `,`.
    last_find: Option<FindSpec>,
    /// Whether a macro is being recorded; set by the event loop.
    pub recording: bool,
}

impl Input {
    pub fn handle(&mut self, key: KeyEvent, mode: Mode) -> Action {
        let key = normalise_alt_gr(key);
        if let Some(prefix) = self.pending.take() {
            return keymap::pending(prefix, key);
        }
        if !matches!(
            mode,
            Mode::Normal | Mode::Visual | Mode::VisualLine | Mode::VisualBlock
        ) {
            self.keys.clear();
        }
        let action = match mode {
            Mode::Normal => self.sequence(key, mode, keymap::normal, grammar::normal),
            Mode::Visual | Mode::VisualLine | Mode::VisualBlock => {
                self.sequence(key, mode, keymap::visual, grammar::visual)
            }
            Mode::Insert => keymap::insert(key),
            Mode::Command => keymap::command(key),
            Mode::Search => keymap::search(key),
            Mode::Tree => keymap::tree(key),
            Mode::Picker => keymap::picker(key),
            Mode::Confirm => keymap::confirm(key),
            Mode::Terminal => keymap::terminal(key, &mut self.pending),
        };
        if matches!(action, Action::EnterMode(_)) {
            self.pending = None;
        }
        action
    }

    /// Feed one key to a mode that understands vi commands.
    ///
    /// Keys that never begin a command — Ctrl chords, Shift+arrows, Escape —
    /// are answered straight away by `direct`; everything else accumulates
    /// until `parse` recognises a whole command or rejects the lot.
    fn sequence(
        &mut self,
        key: KeyEvent,
        mode: Mode,
        direct: fn(KeyEvent, &mut Option<Pending>) -> Option<Action>,
        parse: fn(&[KeyEvent], grammar::Context) -> grammar::Parse,
    ) -> Action {
        if self.keys.is_empty() {
            if let Some(action) = direct(key, &mut self.pending) {
                return action;
            }
        } else if key.code == KeyCode::Esc {
            self.keys.clear();
            return Action::None;
        }
        self.keys.push(key);
        let context = grammar::Context {
            last_find: self.last_find,
            recording: self.recording,
            mode,
        };
        match parse(&self.keys, context) {
            grammar::Parse::Incomplete => Action::None,
            grammar::Parse::Invalid => {
                self.keys.clear();
                Action::None
            }
            grammar::Parse::Done(action) => {
                self.keys.clear();
                if let Some(spec) = find_of(&action) {
                    self.last_find = Some(spec);
                }
                action
            }
        }
    }

    /// The keys of a command still being typed, for the status bar.
    #[must_use]
    pub fn pending_keys(&self) -> String {
        self.keys
            .iter()
            .filter_map(|key| match key.code {
                KeyCode::Char(ch) => Some(ch),
                _ => None,
            })
            .collect()
    }

    pub fn handle_mouse(&mut self, event: MouseEvent) -> Action {
        self.pending = None;
        self.keys.clear();
        keymap::mouse(event)
    }
}

/// The `f`/`t` search inside an action, so `;` can repeat it.
fn find_of(action: &Action) -> Option<FindSpec> {
    match action {
        Action::Goto {
            goal: Goal::Find(spec),
            ..
        }
        | Action::Operate {
            target: Target::Goal(Goal::Find(spec)),
            ..
        } => Some(*spec),
        Action::WithRegister { action, .. } | Action::Repeat { action, .. } => find_of(action),
        _ => None,
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
    fn control_o_opens_the_file_picker() {
        let mut input = Input::default();
        let key = KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL);
        assert_eq!(
            input.handle(key, Mode::Normal),
            Action::OpenPicker(PickerKind::Files)
        );
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
    fn ctrl_shift_up_and_down_still_select_a_line() {
        let mut input = Input::default();
        let up = KeyEvent::new(KeyCode::Up, KeyModifiers::CONTROL | KeyModifiers::SHIFT);
        assert_eq!(
            input.handle(up, Mode::Normal),
            Action::Select(Motion::Up(1))
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

    #[test]
    fn terminal_keys_are_forwarded_except_for_the_window_prefix() {
        let mut input = Input::default();
        let letter = plain('x');
        assert_eq!(
            input.handle(letter, Mode::Terminal),
            Action::TerminalInput(letter)
        );

        let ctrl_w = KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL);
        assert_eq!(input.handle(ctrl_w, Mode::Terminal), Action::None);
        assert_eq!(
            input.handle(plain('c'), Mode::Terminal),
            Action::CloseWindow
        );
    }

    #[test]
    fn ctrl_w_t_opens_a_terminal_from_editor_and_terminal_modes() {
        let ctrl_w = KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL);

        for mode in [Mode::Normal, Mode::Terminal] {
            let mut input = Input::default();
            assert_eq!(input.handle(ctrl_w, mode), Action::None);
            assert_eq!(input.handle(plain('t'), mode), Action::OpenTerminal);

            assert_eq!(input.handle(ctrl_w, mode), Action::None);
            assert_eq!(input.handle(plain('T'), mode), Action::OpenTerminal);
        }
    }
}
