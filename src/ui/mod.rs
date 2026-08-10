//! # User interface
//!
//! **Purpose:** turn application state into a frame.
//!
//! **Responsibility:** split the terminal into regions and hand each one to a
//! widget. The UI layer reads the application state and writes pixels; it never
//! mutates editing state, with the single exception of scrolling the viewport,
//! which cannot be decided until the text area's size is known.
//!
//! **Public API:** [`draw`], [`Regions`].

pub mod text;
pub mod widgets;

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};

use crate::app::App;
use crate::editor::buffer::Buffer;
use crate::editor::selection::Range;
use crate::ui::widgets::{
    CommandBar, EditorView, FileTree, Popup, SearchBox, StatusBar, Tab, TabBar, editor_view,
};

/// Where each part of the interface goes this frame.
#[derive(Debug, Clone, Copy)]
pub struct Regions {
    /// Tab strip, present only when more than one buffer is open.
    pub tabs: Option<Rect>,
    /// File tree panel, present only when it is toggled on.
    pub tree: Option<Rect>,
    /// The text area, gutter included.
    pub editor: Rect,
    /// One-line status bar.
    pub status: Rect,
    /// One-line command bar, shared with transient messages.
    pub command: Rect,
}

impl Regions {
    /// Carve `area` into the editor's regions.
    ///
    /// The bars are fixed at one line each and are taken off the bottom first,
    /// so the editor absorbs every remaining row — and on a terminal too small
    /// to fit everything the flexible region shrinks rather than the fixed ones
    /// overlapping.
    #[must_use]
    pub fn split(area: Rect, show_tabs: bool, show_tree: bool) -> Self {
        let [tabs, body, status, command] = Layout::vertical([
            Constraint::Length(u16::from(show_tabs)),
            Constraint::Min(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .areas(area);

        // The tree takes a fixed slice of the width, capped so it never crowds
        // out the text on a narrow terminal.
        let tree_width = if show_tree {
            (body.width / 4)
                .clamp(16, 40)
                .min(body.width.saturating_sub(20))
        } else {
            0
        };
        let [tree, editor] =
            Layout::horizontal([Constraint::Length(tree_width), Constraint::Min(1)]).areas(body);

        Self {
            tabs: show_tabs.then_some(tabs),
            tree: (tree_width > 0).then_some(tree),
            editor,
            status,
            command,
        }
    }
}

/// Render one frame of the editor.
pub fn draw(frame: &mut Frame, app: &mut App) {
    let show_tabs = app.config.show_tabs && app.buffers.len() > 1;
    let regions = Regions::split(frame.area(), show_tabs, app.tree_visible);

    prepare(app, regions.editor);

    let editor = EditorView {
        buffer: app.buffer(),
        window: app.window(),
        focused: true,
        theme: &app.theme,
        config: &app.config,
        selection: selection_range(app),
        search: app.search.is_active().then_some(&app.search),
        active_match: active_match(app),
    };
    let editor_caret = editor.caret_position(regions.editor);
    frame.render_widget(editor, regions.editor);

    if let Some(area) = regions.tabs {
        let tabs: Vec<Tab<'_>> = app
            .buffers
            .iter()
            .map(|buffer| Tab {
                name: buffer.document.display_name(),
                dirty: buffer.document.is_dirty(),
            })
            .collect();
        frame.render_widget(
            TabBar {
                tabs: &tabs,
                active: app.window.buffer,
                theme: &app.theme,
            },
            area,
        );
    }

    if let (Some(area), Some(tree)) = (regions.tree, app.tree.as_ref()) {
        let title = tree
            .root()
            .file_name()
            .and_then(std::ffi::OsStr::to_str)
            .unwrap_or("files");
        frame.render_widget(
            FileTree {
                entries: tree.entries(),
                selected: app.tree_selected,
                focused: app.mode == crate::app::mode::Mode::Tree,
                title,
                theme: &app.theme,
            },
            area,
        );
    }

    frame.render_widget(status_bar(app), regions.status);

    // The bottom line is either the search prompt, the command line, or the
    // message area — never more than one at a time.
    let prompt_caret = if app.mode.is_search() {
        let search_box = SearchBox {
            query: &app.search.query,
            forward: app.search.forward,
            error: app.search.error(),
            theme: &app.theme,
        };
        let caret = search_box.caret_position(regions.command);
        frame.render_widget(search_box, regions.command);
        Some(caret)
    } else {
        let command_bar = CommandBar {
            command: app.mode.is_command().then_some(app.command_line.as_str()),
            status: &app.status,
            theme: &app.theme,
        };
        let caret = command_bar.caret_position(regions.command);
        frame.render_widget(command_bar, regions.command);
        caret
    };

    // The popup is modal, so it is drawn last and hides the caret.
    if let Some((title, body)) = app.popup.as_ref() {
        frame.render_widget(
            Popup {
                title,
                body,
                hint: "press any key",
                theme: &app.theme,
            },
            frame.area(),
        );
        return;
    }

    if let Some(position) = prompt_caret.or(editor_caret) {
        frame.set_cursor_position(position);
    }
}

/// Bring the window up to date with the area it is about to be drawn in.
///
/// This is the only part of rendering that mutates state, and it has to happen
/// before the immutable draw pass: the caret cannot be scrolled into view, and
/// the syntax cache cannot be extended to cover the visible lines, until the
/// size of the text area is known.
fn prepare(app: &mut App, area: Rect) {
    let index = app.window.buffer;
    app.window.height = area.height;

    let App {
        buffers,
        window,
        config,
        ..
    } = app;
    editor_view::scroll_into_view(&buffers[index], window, config, area);

    if config.syntax_highlighting {
        let last = window.view.top_line + usize::from(area.height);
        let Buffer {
            document, syntax, ..
        } = &mut buffers[index];
        syntax.ensure(document, last);
    }
}

/// The match the caret currently sits on, so it can be highlighted differently
/// from the other matches.
fn active_match(app: &App) -> Option<Range> {
    if !app.search.is_active() {
        return None;
    }
    let document = &app.buffer().document;
    let cursor = app.window().cursor();
    let head = document.pos_to_char(cursor.head);
    let line_start = document.line_start(cursor.head.line);

    app.search
        .matches_in_line(&document.line_string(cursor.head.line))
        .into_iter()
        .map(|found| Range {
            start: line_start + found.start,
            end: line_start + found.end,
        })
        .find(|range| range.contains(head))
}

/// The span to paint as selected, which exists only in visual modes.
fn selection_range(app: &App) -> Option<Range> {
    let document = &app.buffer().document;
    let cursor = app.window().cursor();
    match app.mode {
        crate::app::mode::Mode::Visual => Some(Range::of(&cursor, document)),
        crate::app::mode::Mode::VisualLine => Some(Range::of_lines(&cursor, document)),
        _ => None,
    }
}

/// Gather the values the status bar reports.
fn status_bar(app: &App) -> StatusBar<'_> {
    let buffer = app.buffer();
    let window = app.window();
    StatusBar {
        mode: app.mode,
        name: buffer.document.display_name(),
        dirty: buffer.document.is_dirty(),
        language: buffer.syntax.language_name(),
        position: window.cursor().head,
        line_count: buffer.document.len_lines(),
        line_ending: buffer.document.line_ending(),
        cursor_count: window.cursors().len(),
        theme: &app.theme,
    }
}
