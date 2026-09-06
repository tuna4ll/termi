//! # Application layer
//!
//! **Purpose:** own everything that is "the running editor" as opposed to "a
//! piece of text".
//!
//! **Responsibility:** holds the global state (open buffers, current mode,
//! pending command line, transient status message) and drives the event loop
//! that ties input, editing and rendering together.
//!
//! **Public API:** [`App`], [`run`].

pub mod commands;
pub mod dispatch;
pub mod mode;
pub mod state;

use std::path::PathBuf;
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{self, Event, KeyEventKind};

pub use state::App;

use crate::filesystem::watcher::Watcher;
use crate::input::Input;
use crate::renderer::Tui;
use crate::ui;

/// How long to block on input before waking up anyway.
///
/// The loop is otherwise fully event driven — this timeout only exists so that
/// background sources (file-system events, resize follow-ups) get serviced
/// promptly without spinning the CPU.
const POLL_INTERVAL: Duration = Duration::from_millis(100);

/// Run the editor until the user asks to quit.
///
/// # Errors
/// Returns an error if drawing a frame or reading an event fails.
pub fn run(app: &mut App, tui: &mut Tui) -> Result<()> {
    let mut input = Input::default();
    let mut watcher = app.config.watch_files.then(Watcher::new).flatten();

    while !app.should_quit() {
        if let Some(watcher) = watcher.as_mut() {
            watch_open_files(app, watcher);
            handle_external_changes(app, watcher.drain());
        }
        // A modeless selection has its head *between* characters, the way a bar
        // cursor does, so the shape follows the selection as well as the mode.
        tui.set_cursor_shape(app.mode.uses_bar_cursor() || app.has_selection())?;
        tui.draw(|frame| ui::draw(frame, app))?;

        if !event::poll(POLL_INTERVAL)? {
            continue;
        }

        match event::read()? {
            // Windows reports both press and release; acting on both would
            // double every keystroke.
            Event::Key(key) if key.kind == KeyEventKind::Press => {
                let action = input.handle(key, app.mode);
                dispatch::apply(app, action)?;
            }
            Event::Mouse(mouse) => {
                let action = input.handle_mouse(mouse);
                dispatch::apply(app, action)?;
            }
            Event::Paste(text) => {
                let mut edit = app.edit();
                edit.insert_text(&text);
                edit.checkpoint();
            }
            Event::Resize(_, _) => tui.clear()?,
            _ => {}
        }
    }
    Ok(())
}

/// Make sure every open file is being watched.
///
/// Called each iteration because buffers are opened and closed while running;
/// [`Watcher::watch`] ignores paths it already covers.
fn watch_open_files(app: &App, watcher: &mut Watcher) {
    for buffer in &app.buffers {
        if let Some(path) = buffer.document.path() {
            watcher.watch(path);
        }
    }
}

/// React to files that changed on disk.
///
/// A clean buffer is reloaded silently — that is what the user wants when a
/// formatter or a branch switch rewrote the file. A modified buffer is never
/// touched; overwriting unsaved work is unforgivable, so it only gets a warning.
fn handle_external_changes(app: &mut App, changed: Vec<PathBuf>) {
    for path in changed {
        let Some(index) = app
            .buffers
            .iter()
            .position(|buffer| buffer.document.path() == Some(path.as_path()))
        else {
            continue;
        };

        // Our own saves fire events too. Comparing content rather than
        // timestamps tells a real external edit from the write we just did.
        let Ok(on_disk) = crate::filesystem::read_file(&path) else {
            continue;
        };
        if app.buffers[index].document.matches_disk_text(&on_disk) {
            continue;
        }

        if app.buffers[index].document.is_dirty() {
            let name = app.buffers[index].document.display_name().to_string();
            app.show_popup(
                "file changed on disk",
                format!(
                    "{name} was modified outside the editor, and this buffer has \
                     unsaved changes.\n\nUse :e! to discard yours and reload, or \
                     :w to overwrite the file on disk."
                ),
            );
        } else if app.buffers[index].document.reload().is_ok() {
            app.buffers[index].detect_language();
            app.clamp_windows_on(index);
            let name = app.buffers[index].document.display_name().to_string();
            app.info(format!("{name} reloaded from disk"));
        }
    }
}
