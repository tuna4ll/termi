//! # User interface
//!
//! **Purpose:** turn application state into a frame.
//!
//! **Responsibility:** split the terminal into regions, divide the text area
//! between the open windows, and hand each region to a widget. The UI layer
//! reads the application state and writes pixels; it never mutates editing
//! state, with the single exception of [`prepare`], which cannot run until the
//! size of each window is known.
//!
//! **Public API:** [`draw`], [`Regions`].

pub mod layout;
pub mod text;
pub mod widgets;

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};

use crate::app::App;
use crate::editor::buffer::Buffer;
use crate::editor::selection::Range;
use crate::editor::window::Area;
use crate::editor::window::tree::Axis;
use crate::theme::Theme;
use crate::ui::layout::Panes;
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
    /// The area the windows divide between them, gutters included.
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

    let panes = layout::solve(app.windows.root(), regions.editor);
    prepare(app, &panes);

    let editor_caret = draw_windows(frame, app, &panes);
    draw_separators(frame, &panes, &app.theme);

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
                active: app.window().buffer,
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

/// Bring every window up to date with the area it is about to be drawn in.
///
/// This is the only part of rendering that mutates state, and it has to happen
/// before the immutable draw pass: no caret can be scrolled into view, and the
/// syntax cache cannot be extended to cover the visible lines, until the size of
/// each window is known.
fn prepare(app: &mut App, panes: &Panes) {
    let focus = app.windows.focus();

    for (id, rect) in &panes.windows {
        let App {
            buffers,
            windows,
            config,
            ..
        } = &mut *app;

        let window = windows.get_mut(*id);
        window.area = Area::new(rect.x, rect.y, rect.width, rect.height);
        let index = window.buffer;

        // A window that did not make the last edit is never told that the text
        // moved underneath it, so its cursors are checked here instead. The
        // focused window is left alone: dispatch has already put its caret
        // exactly where the current mode wants it.
        if *id != focus {
            window.clamp_cursors(&buffers[index].document, false);
        }
        editor_view::scroll_into_view(&buffers[index], window, config, *rect);

        // Extend the syntax state cache to cover what is about to be drawn.
        if config.syntax_highlighting {
            let last = window.view.top_line + usize::from(rect.height);
            let Buffer {
                document, syntax, ..
            } = &mut buffers[index];
            syntax.ensure(document, last);
        }
    }
}

/// Draw every window, returning where the caret goes in the focused one.
fn draw_windows(frame: &mut Frame, app: &App, panes: &Panes) -> Option<(u16, u16)> {
    let focus = app.windows.focus();
    let mut caret = None;

    for (id, rect) in &panes.windows {
        let focused = *id == focus;
        let window = app.windows.get(*id);
        let view = EditorView {
            buffer: &app.buffers[window.buffer],
            window,
            focused,
            theme: &app.theme,
            config: &app.config,
            // A selection and the search highlight both belong to the window
            // being typed in; painting them everywhere would suggest the other
            // windows were about to act on them too.
            selection: if focused { selection_range(app) } else { None },
            search: (focused && app.search.is_active()).then_some(&app.search),
            active_match: if focused { active_match(app) } else { None },
        };
        if focused {
            caret = view.caret_position(*rect);
        }
        frame.render_widget(view, *rect);
    }
    caret
}

/// Draw the rules between neighbouring windows.
fn draw_separators(frame: &mut Frame, panes: &Panes, theme: &Theme) {
    let surface = frame.buffer_mut();
    for (rect, axis) in &panes.separators {
        // A split that divides the width is separated by a vertical rule.
        let glyph = match axis {
            Axis::Horizontal => '│',
            Axis::Vertical => '─',
        };
        for y in rect.top()..rect.bottom() {
            for x in rect.left()..rect.right() {
                if let Some(cell) = surface.cell_mut((x, y)) {
                    cell.set_char(glyph).set_style(theme.gutter);
                }
            }
        }
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
