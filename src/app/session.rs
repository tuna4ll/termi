//! # Sessions
//!
//! **Purpose:** pick up where the last session in a directory left off.
//!
//! **Responsibility:** on the way out, write down which files were open, how
//! the windows were split, and where each caret and scroll position was; on
//! the way in, when no files were named, open them all again. Sessions are
//! kept per working directory, so each project remembers its own.
//!
//! **Public API:** [`save`], [`restore`], [`save_to`], [`restore_from`].

use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::{Deserialize, Serialize};

use super::state::App;
use crate::editor::cursor::Position;
use crate::editor::window::tree::{Axis, Branch, Node};
use crate::editor::window::{Window, Windows};
use crate::undo::persist::fingerprint;

/// What a session file holds.
#[derive(Debug, Serialize, Deserialize)]
struct Session {
    /// Every named file that was open, in tab order.
    files: Vec<PathBuf>,
    /// How the windows divided the screen.
    layout: Layout,
    /// Which window, in screen order, had the keyboard.
    focus: usize,
}

/// A saved window arrangement.
#[derive(Debug, Serialize, Deserialize)]
enum Layout {
    /// One window on one of the files.
    Window {
        file: usize,
        line: usize,
        col: usize,
        top: usize,
    },
    /// Windows side by side (`horizontal`) or stacked.
    Split {
        horizontal: bool,
        children: Vec<(u16, Layout)>,
    },
}

/// Where the session for the working directory lives.
fn location(app: &App) -> Option<PathBuf> {
    if app.ephemeral {
        return None;
    }
    let cwd = std::env::current_dir().ok()?;
    let key = fingerprint(cwd.to_string_lossy().as_bytes());
    Some(
        crate::config::data_dir()
            .join("sessions")
            .join(format!("{key:016x}.json")),
    )
}

/// Save the session for the working directory.
///
/// # Errors
/// Returns an error if the session file cannot be written.
pub fn save(app: &App) -> Result<()> {
    match location(app) {
        Some(path) => save_to(app, &path),
        None => Ok(()),
    }
}

/// Restore the session for the working directory, returning whether there
/// was one.
pub fn restore(app: &mut App) -> bool {
    location(app).is_some_and(|path| restore_from(app, &path))
}

/// Write `app`'s session to `path`. Nothing is written when no named file is
/// open — an empty session is not worth restoring over the last real one.
///
/// # Errors
/// Returns an error if the file cannot be written.
pub fn save_to(app: &App, path: &Path) -> Result<()> {
    let files: Vec<PathBuf> = app
        .buffers
        .iter()
        .filter_map(|buffer| buffer.document.path())
        .map(|file| file.canonicalize().unwrap_or_else(|_| file.to_path_buf()))
        .collect();
    if files.is_empty() {
        return Ok(());
    }
    // Buffer ids to positions in `files`; unnamed buffers have none, and a
    // window showing one is saved as showing the first file instead.
    let mut index = Vec::with_capacity(app.buffers.len());
    let mut next = 0;
    for buffer in &app.buffers {
        index.push(buffer.document.path().map(|_| {
            next += 1;
            next - 1
        }));
    }
    let layout = layout_of(app, app.windows.root(), &index);
    let focus = app
        .windows
        .ids()
        .iter()
        .position(|id| *id == app.windows.focus())
        .unwrap_or(0);
    let session = Session {
        files,
        layout,
        focus,
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    crate::filesystem::write_file(path, &serde_json::to_string(&session)?)
}

fn layout_of(app: &App, node: &Node, index: &[Option<usize>]) -> Layout {
    match node {
        Node::Leaf(id) => {
            let window = app.windows.get(*id);
            let head = window.cursor().head;
            Layout::Window {
                file: index.get(window.buffer).copied().flatten().unwrap_or(0),
                line: head.line,
                col: head.col,
                top: window.view.top_line,
            }
        }
        Node::Split { axis, children } => Layout::Split {
            horizontal: *axis == Axis::Horizontal,
            children: children
                .iter()
                .map(|branch| (branch.weight, layout_of(app, &branch.node, index)))
                .collect(),
        },
    }
}

/// Restore a session from `path`, returning whether anything was opened.
///
/// Files that have gone missing since are skipped; a window that showed one
/// shows the first file that did open.
pub fn restore_from(app: &mut App, path: &Path) -> bool {
    let Some(session) = std::fs::read_to_string(path)
        .ok()
        .and_then(|json| serde_json::from_str::<Session>(&json).ok())
    else {
        return false;
    };
    let mut buffer_of = Vec::with_capacity(session.files.len());
    for file in &session.files {
        let opened = file.is_file() && app.open(file.clone()).is_ok();
        buffer_of.push(opened.then(|| app.window().buffer));
    }
    let Some(fallback) = buffer_of.iter().flatten().next().copied() else {
        return false;
    };

    let mut windows = Vec::new();
    let root = rebuild(app, &session.layout, &buffer_of, fallback, &mut windows);
    let focus = session.focus.min(windows.len().saturating_sub(1));
    if let Some(restored) = Windows::from_parts(windows, root, focus) {
        app.windows = restored;
    }
    true
}

fn rebuild(
    app: &App,
    layout: &Layout,
    buffer_of: &[Option<usize>],
    fallback: usize,
    windows: &mut Vec<Window>,
) -> Node {
    match layout {
        Layout::Window {
            file,
            line,
            col,
            top,
        } => {
            let buffer = buffer_of.get(*file).copied().flatten().unwrap_or(fallback);
            let mut window = Window::new(buffer);
            let document = &app.buffers[buffer].document;
            let position = document.clamp(Position::new(*line, *col), false);
            window.cursor_mut().move_to(position, false);
            window.view.top_line = (*top).min(document.last_line());
            windows.push(window);
            Node::Leaf(windows.len() - 1)
        }
        Layout::Split {
            horizontal,
            children,
        } => {
            let children: Vec<Branch> = children
                .iter()
                .map(|(weight, child)| Branch {
                    weight: (*weight).max(1),
                    node: rebuild(app, child, buffer_of, fallback, windows),
                })
                .collect();
            if children.len() == 1 {
                return children
                    .into_iter()
                    .next()
                    .map_or(Node::Leaf(0), |b| b.node);
            }
            Node::Split {
                axis: if *horizontal {
                    Axis::Horizontal
                } else {
                    Axis::Vertical
                },
                children,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    #[test]
    fn files_splits_and_carets_come_back() {
        let root = std::env::temp_dir().join("termi-session");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("create fixture");
        let (a, b) = (root.join("a.txt"), root.join("b.txt"));
        std::fs::write(&a, "one\ntwo\nthree\n").expect("write a");
        std::fs::write(&b, "bee\n").expect("write b");

        let mut app = App::with_config(Config {
            system_clipboard: false,
            ..Config::default()
        });
        app.open(a.clone()).expect("open a");
        app.window_mut()
            .cursor_mut()
            .move_to(Position::new(2, 1), false);
        app.split_window(Axis::Horizontal);
        app.open(b).expect("open b");
        let session = root.join("session.json");
        save_to(&app, &session).expect("save session");

        let mut restored = App::with_config(Config {
            system_clipboard: false,
            ..Config::default()
        });
        assert!(restore_from(&mut restored, &session));
        assert_eq!(restored.buffers.len(), 2);
        assert_eq!(restored.windows.count(), 2);
        assert_eq!(restored.buffer().document.display_name(), "b.txt");
        let first = restored.windows.ids()[0];
        assert_eq!(
            restored.windows.get(first).cursor().head,
            Position::new(2, 1)
        );
    }
}
