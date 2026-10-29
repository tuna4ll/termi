//! Properties that must hold for any input, checked against generated cases.

use proptest::prelude::*;
use termi::editor::cursor::Position;
use termi::editor::document::Document;
use termi::input::grammar::{self, Context};
use termi::input::notation;
use termi::undo::{Change, History};

/// One edit to apply: insert text at a fraction of the way through the
/// document, or remove a span starting there.
#[derive(Debug, Clone)]
enum Op {
    Insert(f64, String),
    Remove(f64, usize),
    Checkpoint,
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        (0.0..=1.0f64, "[a-z\\n é𝄞]{0,6}").prop_map(|(at, text)| Op::Insert(at, text)),
        (0.0..=1.0f64, 0usize..8).prop_map(|(at, length)| Op::Remove(at, length)),
        Just(Op::Checkpoint),
    ]
}

fn apply(document: &mut Document, history: &mut History, op: &Op) {
    let length = document.len_chars();
    let at = |fraction: f64| ((length as f64) * fraction) as usize;
    match op {
        Op::Insert(fraction, text) => {
            let at = at(*fraction);
            document.insert(at, text);
            history.record(
                Change::insertion(at, text.as_str()),
                Position::ZERO,
                Position::ZERO,
            );
        }
        Op::Remove(fraction, count) => {
            let start = at(*fraction);
            let removed = document.remove(start, start + count);
            history.record(
                Change::deletion(start, removed),
                Position::ZERO,
                Position::ZERO,
            );
        }
        Op::Checkpoint => history.checkpoint(),
    }
}

proptest! {
    #[test]
    fn undoing_everything_restores_the_original(original in "[a-z\\n]{0,40}", ops in prop::collection::vec(op(), 0..40)) {
        let mut document = Document::from_text(&original, None);
        let mut history = History::default();
        for op in &ops {
            apply(&mut document, &mut history, op);
        }
        while history.undo(&mut document).is_some() {}
        prop_assert_eq!(document.text().to_string(), original);
    }

    #[test]
    fn travelling_back_and_forward_in_time_comes_home(ops in prop::collection::vec(op(), 1..30), back in 0usize..40) {
        let mut document = Document::from_text("start\n", None);
        let mut history = History::default();
        for (index, op) in ops.iter().enumerate() {
            apply(&mut document, &mut history, op);
            // Undo now and then, so the history branches.
            if index % 7 == 3 {
                history.undo(&mut document);
            }
        }
        let now = document.text().to_string();
        let mut steps = 0;
        while steps < back && history.earlier(&mut document, 1).is_some() {
            steps += 1;
        }
        for _ in 0..steps {
            history.later(&mut document, 1);
        }
        prop_assert_eq!(document.text().to_string(), now);
    }

    #[test]
    fn the_command_grammar_never_panics(keys in "[ -~]{0,12}") {
        if let Ok(parsed) = notation::parse(&keys) {
            let _ = grammar::normal(&parsed, Context::default());
            let _ = grammar::visual(&parsed, Context::default());
        }
    }

    #[test]
    fn key_notation_round_trips(text in "[a-zA-Z0-9 :/<>!@#]{1,20}") {
        let keys = notation::parse(&text).expect("any text is keys");
        let again = notation::parse(&notation::encode(&keys)).expect("encoded keys parse");
        prop_assert_eq!(keys, again);
    }

    #[test]
    fn every_language_highlights_any_line_without_panicking(line in "\\PC{0,60}") {
        termi::syntax::builtin_only();
        for name in termi::syntax::names() {
            let highlighter = termi::syntax::by_name(name).expect("known language");
            let (spans, _) = highlighter.highlight_line(&line, termi::syntax::BlockState::Normal);
            let length = line.chars().count();
            for span in spans {
                prop_assert!(span.start <= span.end && span.end <= length);
            }
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// The fuzz target's job in miniature, so every test run does a little
    /// of it: random keys in random modes must never bring the editor down.
    #[test]
    fn random_keys_leave_the_editor_standing(seed in "[a-z(){}\\n ]{0,40}", keys in prop::collection::vec((any::<u8>(), 0u8..4), 0..80)) {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        use termi::app::{App, dispatch};
        use termi::input::{Action, Input};

        termi::syntax::builtin_only();
        let mut app = App::headless(termi::config::Config {
            system_clipboard: false,
            ..termi::config::Config::default()
        });
        app.buffers[0] = termi::editor::buffer::Buffer::new(Document::from_text(&seed, Some("t.rs".into())));
        app.windows.focused_mut().reset(0);
        let mut input = Input::default();
        let special = [KeyCode::Esc, KeyCode::Enter, KeyCode::Backspace, KeyCode::Left, KeyCode::Down, KeyCode::Tab];
        for (byte, modifier) in keys {
            let code = match byte {
                0x20..=0x7e => KeyCode::Char(char::from(byte)),
                other => special[usize::from(other) % special.len()],
            };
            let modifiers = [KeyModifiers::NONE, KeyModifiers::CONTROL, KeyModifiers::ALT, KeyModifiers::SHIFT][usize::from(modifier)];
            let action = input.handle(KeyEvent::new(code, modifiers), app.mode);
            // No shells, no command lines that touch the disk.
            if matches!(action, Action::OpenTerminal | Action::RunCommand(_) | Action::Save | Action::CommandSubmit) {
                continue;
            }
            dispatch::apply(&mut app, action).expect("dispatch never fails");
            app.pending_keys.clear();
            if app.should_quit() {
                break;
            }
        }
        let document = &app.buffer().document;
        for cursor in app.window().cursors() {
            prop_assert!(cursor.head.line <= document.last_line());
        }
    }
}
