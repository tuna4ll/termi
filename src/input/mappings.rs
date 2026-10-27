//! # Key mappings
//!
//! **Purpose:** let users rebind keys from the config file.
//!
//! **Responsibility:** compile the `[keys.<mode>]` tables into key sequences
//! and what they stand for, and name the actions a mapping can point at. The
//! matching itself happens in [`Input`](super::Input), which sees the keys as
//! they arrive.
//!
//! A mapping's value is one of three things:
//!
//! - an action name, such as `"file_picker"` or `"normal_mode"`;
//! - a command line, starting with `:`, such as `":w"`;
//! - anything else, read as keys to type instead, such as `"ddp"` or
//!   `"<Esc>:w<CR>"`. Those keys are not mapped again, so a mapping can never
//!   loop into itself.
//!
//! **Public API:** [`Binding`], [`Mappings`], [`named_action`], [`ACTION_NAMES`].

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::{Action, notation};
use crate::app::mode::Mode;
use crate::config::KeyTables;
use crate::editor::cursor::Motion;
use crate::editor::window::Axis;
use crate::picker::PickerKind;

/// What a mapped key sequence does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Binding {
    /// Carry out an action.
    Action(Action),
    /// Type these keys instead, without mapping them again.
    Keys(Vec<KeyEvent>),
}

/// One compiled mapping.
type Entry = (Vec<KeyEvent>, Binding);

/// The compiled mappings for every mode that has any.
#[derive(Debug, Clone, Default)]
pub struct Mappings {
    normal: Vec<Entry>,
    insert: Vec<Entry>,
    visual: Vec<Entry>,
}

impl Mappings {
    /// Compile the config's tables, collecting a message for each entry that
    /// cannot be understood rather than refusing the lot.
    #[must_use]
    pub fn compile(tables: &KeyTables) -> (Self, Vec<String>) {
        let mut errors = Vec::new();
        let mut table = |name: &str, entries: &std::collections::BTreeMap<String, String>| {
            entries
                .iter()
                .filter_map(|(keys, value)| match compile_entry(keys, value) {
                    Ok(entry) => Some(entry),
                    Err(error) => {
                        errors.push(format!("keys.{name}: {error}"));
                        None
                    }
                })
                .collect::<Vec<_>>()
        };
        let mappings = Self {
            normal: table("normal", &tables.normal),
            insert: table("insert", &tables.insert),
            visual: table("visual", &tables.visual),
        };
        (mappings, errors)
    }

    /// The mappings that apply in `mode`, if it has any.
    #[must_use]
    pub fn for_mode(&self, mode: Mode) -> Option<&[Entry]> {
        let table = match mode {
            Mode::Normal => &self.normal,
            Mode::Insert => &self.insert,
            Mode::Visual | Mode::VisualLine | Mode::VisualBlock => &self.visual,
            _ => return None,
        };
        (!table.is_empty()).then_some(table.as_slice())
    }
}

fn compile_entry(keys: &str, value: &str) -> Result<Entry, String> {
    let sequence = notation::parse(keys)?;
    let binding = if let Some(command) = value.strip_prefix(':') {
        Binding::Action(Action::RunCommand(command.to_string()))
    } else if let Some(action) = named_action(value) {
        Binding::Action(action)
    } else {
        Binding::Keys(notation::parse(value).map_err(|_| format!("{keys:?}: nothing to do"))?)
    };
    Ok((sequence, binding))
}

/// Whether two keys are the same chord, ignoring the Shift that an upper-case
/// letter already implies and the event details terminals disagree on.
#[must_use]
pub fn same_key(a: &KeyEvent, b: &KeyEvent) -> bool {
    let strip = |key: &KeyEvent| match key.code {
        KeyCode::Char(_) => key.modifiers - KeyModifiers::SHIFT,
        _ => key.modifiers,
    };
    a.code == b.code && strip(a) == strip(b)
}

/// Whether `sequence` begins with `prefix`.
#[must_use]
pub fn starts_with(sequence: &[KeyEvent], prefix: &[KeyEvent]) -> bool {
    sequence.len() >= prefix.len() && sequence.iter().zip(prefix).all(|(a, b)| same_key(a, b))
}

/// Every action a mapping can name, for the documentation and completion.
pub const ACTION_NAMES: &[&str] = &[
    "save",
    "quit",
    "undo",
    "redo",
    "file_picker",
    "buffer_picker",
    "command_palette",
    "theme_picker",
    "toggle_tree",
    "next_buffer",
    "prev_buffer",
    "jump_back",
    "jump_forward",
    "normal_mode",
    "insert_mode",
    "visual_mode",
    "visual_line_mode",
    "visual_block_mode",
    "command_mode",
    "search_forward",
    "search_backward",
    "search_next",
    "search_prev",
    "split",
    "vsplit",
    "close_window",
    "only_window",
    "next_window",
    "terminal",
    "select_all",
    "copy",
    "cut",
    "paste",
    "toggle_comment",
    "page_down",
    "page_up",
    "half_page_down",
    "half_page_up",
    "add_cursor_above",
    "add_cursor_below",
    "clear_cursors",
    "move_left",
    "move_right",
    "move_up",
    "move_down",
    "word_forward",
    "word_backward",
    "line_start",
    "line_end",
    "file_start",
    "file_end",
    "repeat_change",
];

/// The action a name in a mapping stands for.
#[must_use]
pub fn named_action(name: &str) -> Option<Action> {
    Some(match name {
        "save" => Action::Save,
        "quit" => Action::Quit,
        "undo" => Action::Undo,
        "redo" => Action::Redo,
        "file_picker" => Action::OpenPicker(PickerKind::Files),
        "buffer_picker" => Action::OpenPicker(PickerKind::Buffers),
        "command_palette" => Action::OpenPicker(PickerKind::Commands),
        "theme_picker" => Action::OpenPicker(PickerKind::Themes),
        "toggle_tree" => Action::ToggleTree,
        "next_buffer" => Action::CycleBuffer { forward: true },
        "prev_buffer" => Action::CycleBuffer { forward: false },
        "jump_back" => Action::JumpBack,
        "jump_forward" => Action::JumpForward,
        "normal_mode" => Action::EnterMode(Mode::Normal),
        "insert_mode" => Action::EnterMode(Mode::Insert),
        "visual_mode" => Action::EnterMode(Mode::Visual),
        "visual_line_mode" => Action::EnterMode(Mode::VisualLine),
        "visual_block_mode" => Action::EnterMode(Mode::VisualBlock),
        "command_mode" => Action::EnterMode(Mode::Command),
        "search_forward" => Action::SearchStart { forward: true },
        "search_backward" => Action::SearchStart { forward: false },
        "search_next" => Action::SearchRepeat { forward: true },
        "search_prev" => Action::SearchRepeat { forward: false },
        "split" => Action::SplitWindow {
            axis: Axis::Vertical,
        },
        "vsplit" => Action::SplitWindow {
            axis: Axis::Horizontal,
        },
        "close_window" => Action::CloseWindow,
        "only_window" => Action::OnlyWindow,
        "next_window" => Action::CycleWindow,
        "terminal" => Action::OpenTerminal,
        "select_all" => Action::SelectAll,
        "copy" => Action::Yank,
        "cut" => Action::Cut,
        "paste" => Action::Paste,
        "toggle_comment" => Action::ToggleComment,
        "page_down" => Action::Page {
            down: true,
            half: false,
        },
        "page_up" => Action::Page {
            down: false,
            half: false,
        },
        "half_page_down" => Action::Page {
            down: true,
            half: true,
        },
        "half_page_up" => Action::Page {
            down: false,
            half: true,
        },
        "add_cursor_above" => Action::AddCursor { below: false },
        "add_cursor_below" => Action::AddCursor { below: true },
        "clear_cursors" => Action::ClearCursors,
        "move_left" => Action::Move(Motion::Left),
        "move_right" => Action::Move(Motion::Right),
        "move_up" => Action::Move(Motion::Up(1)),
        "move_down" => Action::Move(Motion::Down(1)),
        "word_forward" => Action::Move(Motion::WordForward),
        "word_backward" => Action::Move(Motion::WordBackward),
        "line_start" => Action::Move(Motion::LineStart),
        "line_end" => Action::Move(Motion::LineEnd),
        "file_start" => Action::Move(Motion::DocStart),
        "file_end" => Action::Move(Motion::DocEnd),
        "repeat_change" => Action::RepeatChange(None),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_listed_name_is_an_action() {
        for name in ACTION_NAMES {
            assert!(named_action(name).is_some(), "{name}");
        }
    }

    #[test]
    fn values_compile_to_actions_commands_or_keys() {
        let mut tables = KeyTables::default();
        tables.normal.insert("<Space>w".into(), ":w".into());
        tables.normal.insert("<C-o>".into(), "jump_back".into());
        tables.normal.insert("Q".into(), "ddp".into());
        tables.insert.insert("jk".into(), "normal_mode".into());
        let (mappings, errors) = Mappings::compile(&tables);
        assert!(errors.is_empty(), "{errors:?}");
        let normal = mappings.for_mode(Mode::Normal).expect("normal mappings");
        assert!(
            normal
                .iter()
                .any(|(_, binding)| *binding == Binding::Action(Action::RunCommand("w".into())))
        );
        assert!(
            normal
                .iter()
                .any(|(_, binding)| matches!(binding, Binding::Keys(keys) if keys.len() == 3))
        );
        assert!(mappings.for_mode(Mode::Command).is_none());
    }
}
