//! What every integration test needs: an editor that touches nothing outside
//! the test, and a way to type at it and look at it.

#![allow(dead_code)]

use std::path::PathBuf;

use crossterm::event::KeyEvent;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use termi::app::App;
use termi::app::dispatch;
use termi::config::Config;
use termi::editor::buffer::Buffer;
use termi::editor::document::Document;
use termi::input::{Input, notation};

/// The settings every test starts from: no system clipboard, and nothing
/// that would make a result depend on whose machine runs it.
pub fn config() -> Config {
    termi::syntax::builtin_only();
    Config {
        system_clipboard: false,
        auto_pairs: false,
        ..Config::default()
    }
}

/// An ephemeral editor holding `text` in an unnamed buffer.
pub fn editor(text: &str) -> App {
    editor_with(text, config())
}

/// [`editor`] with other settings.
pub fn editor_with(text: &str, config: Config) -> App {
    let mut app = App::headless(config);
    app.buffers[0] = Buffer::new(Document::from_text(text, None));
    app.windows.focused_mut().reset(0);
    app
}

/// A fresh, empty directory for one test's files.
pub fn scratch(name: &str) -> PathBuf {
    let directory = std::env::temp_dir().join(format!("termi-it-{name}"));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).expect("create scratch directory");
    directory
}

/// Type `keys`, written in key notation, the way the event loop handles them.
pub fn typed(app: &mut App, keys: &str) {
    let mut input = Input::default();
    input.set_mappings(app.mappings.clone());
    for key in notation::parse(keys).expect("valid key notation") {
        if let Some((_, recorded)) = app.macro_recording.as_mut() {
            recorded.push(key);
        }
        feed(app, &mut input, key);
        while let Some(queued) = app.pending_keys.pop_front() {
            feed(app, &mut input, queued);
        }
    }
}

fn feed(app: &mut App, input: &mut Input, key: KeyEvent) {
    input.recording = app.macro_recording.is_some();
    let action = input.handle(key, app.mode);
    dispatch::apply(app, action).expect("dispatch should not fail");
    while let Some((queued, unmapped)) = input.take_replay() {
        let action = input.handle_queued(queued, unmapped, app.mode);
        dispatch::apply(app, action).expect("dispatch should not fail");
    }
}

/// The buffer's text.
pub fn text(app: &App) -> String {
    app.buffer().document.text().to_string()
}

/// Draw one frame and return it as lines of text.
pub fn screen(app: &mut App, width: u16, height: u16) -> Vec<String> {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("test backend");
    terminal
        .draw(|frame| termi::ui::draw(frame, app))
        .expect("drawing should not fail");
    let buffer = terminal.backend().buffer().clone();
    (0..height)
        .map(|y| {
            (0..width)
                .filter_map(|x| buffer.cell((x, y)).map(|cell| cell.symbol().to_string()))
                .collect::<String>()
        })
        .collect()
}
