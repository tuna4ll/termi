//! Any sequence of keys, in any mode, must leave the editor standing.
//!
//! The first bytes become the starting text; the rest are read two at a time
//! as a key and its modifiers.

#![no_main]

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use libfuzzer_sys::fuzz_target;
use termi::app::{App, dispatch};
use termi::config::Config;
use termi::editor::buffer::Buffer;
use termi::editor::document::Document;
use termi::input::Input;

const SPECIAL: [KeyCode; 12] = [
    KeyCode::Esc,
    KeyCode::Enter,
    KeyCode::Tab,
    KeyCode::Backspace,
    KeyCode::Delete,
    KeyCode::Left,
    KeyCode::Right,
    KeyCode::Up,
    KeyCode::Down,
    KeyCode::Home,
    KeyCode::End,
    KeyCode::PageDown,
];

fuzz_target!(|data: &[u8]| {
    let split = data.len().min(64);
    let (seed, keys) = data.split_at(split);
    termi::syntax::builtin_only();
    let mut app = App::headless(Config {
        system_clipboard: false,
        ..Config::default()
    });
    let text = String::from_utf8_lossy(seed);
    app.buffers[0] = Buffer::new(Document::from_text(&text, Some("fuzz.rs".into())));
    app.windows.focused_mut().reset(0);

    let mut input = Input::default();
    for pair in keys.chunks(2) {
        let code = match pair[0] {
            byte @ 0x20..=0x7e => KeyCode::Char(char::from(byte)),
            byte => SPECIAL[usize::from(byte) % SPECIAL.len()],
        };
        let modifiers = match pair.get(1).copied().unwrap_or(0) % 8 {
            1 => KeyModifiers::CONTROL,
            2 => KeyModifiers::ALT,
            3 => KeyModifiers::SHIFT,
            _ => KeyModifiers::NONE,
        };
        // Terminal panes and the shell they start are out of scope here.
        if matches!(code, KeyCode::Char('t' | 'T')) && modifiers.is_empty() && app.mode == termi::app::mode::Mode::Normal {
            continue;
        }
        let action = input.handle(KeyEvent::new(code, modifiers), app.mode);
        if matches!(action, termi::input::Action::OpenTerminal | termi::input::Action::RunCommand(_)) {
            continue;
        }
        let _ = dispatch::apply(&mut app, action);
        app.pending_keys.clear();
        if app.should_quit() {
            break;
        }
    }
});
