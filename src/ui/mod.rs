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
    let selections = selection_ranges(app);

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
            selections: if focused { &selections } else { &[] },
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

/// The spans to paint as selected, one per cursor that has a selection.
///
/// Visual mode selects the character under each caret as well; every other mode
/// paints whatever Shift or the mouse has dragged out, and nothing when the
/// cursors are collapsed.
fn selection_ranges(app: &App) -> Vec<Range> {
    use crate::app::mode::Mode;

    let document = &app.buffer().document;
    let cursors = app.window().cursors();
    match app.mode {
        Mode::Visual => cursors.iter().map(|c| Range::of(c, document)).collect(),
        Mode::VisualLine => cursors
            .iter()
            .map(|c| Range::of_lines(c, document))
            .collect(),
        _ => cursors
            .iter()
            .filter(|cursor| cursor.has_selection())
            .map(|c| Range::between(c, document))
            .collect(),
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

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer as Surface;

    use crate::config::Config;
    use crate::editor::buffer::Buffer as TextBuffer;
    use crate::editor::document::Document;

    /// An editor holding `text`, with the settings pinned so the frame does not
    /// change with whatever is installed on the machine running the test.
    fn app(text: &str) -> App {
        let mut app = App::with_config(Config {
            system_clipboard: false,
            syntax_highlighting: false,
            line_numbers: false,
            ..Config::default()
        });
        app.buffers[0] = TextBuffer::new(Document::from_text(text, None));
        app.windows.focused_mut().reset(0);
        app
    }

    /// Draw one frame into an off-screen terminal.
    fn render(app: &mut App, width: u16, height: u16) -> Surface {
        let mut terminal =
            Terminal::new(TestBackend::new(width, height)).expect("the test backend always builds");
        terminal
            .draw(|frame| draw(frame, app))
            .expect("drawing into the test backend should not fail");
        terminal.backend().buffer().clone()
    }

    fn row(surface: &Surface, y: u16) -> String {
        (0..surface.area.width)
            .filter_map(|x| surface.cell((x, y)).map(|cell| cell.symbol().to_string()))
            .collect()
    }

    #[test]
    fn one_window_fills_the_text_area_without_a_rule() {
        let mut app = app("alpha");
        let surface = render(&mut app, 40, 10);

        assert!(row(&surface, 0).starts_with("alpha"));
        for y in 0..8 {
            assert!(
                !row(&surface, y).contains('│'),
                "a single window should not be divided"
            );
        }
    }

    #[test]
    fn a_side_by_side_split_draws_both_halves_and_the_rule() {
        let mut app = app("alpha");
        app.windows.split(Axis::Horizontal);
        let surface = render(&mut app, 41, 10);

        let first = row(&surface, 0);
        // Both windows show the same file, so the text appears twice with the
        // rule between them.
        assert_eq!(first.matches("alpha").count(), 2, "row was {first:?}");
        assert!(first.contains('│'), "row was {first:?}");
    }

    #[test]
    fn a_stacked_split_draws_a_horizontal_rule() {
        let mut app = app("alpha");
        app.windows.split(Axis::Vertical);
        let surface = render(&mut app, 40, 11);

        let divided = (0..9).any(|y| row(&surface, y).contains('─'));
        assert!(divided, "a stacked split should be separated by a rule");
    }

    #[test]
    fn two_windows_on_one_file_can_sit_at_different_lines() {
        let text: String = (0..100).map(|n| format!("line {n}\n")).collect();
        let mut app = app(&text);

        app.windows.split(Axis::Horizontal);
        // Send the focused window a long way down the file; the other stays put.
        app.windows.focused_mut().view.top_line = 60;
        app.move_cursors(crate::editor::cursor::Motion::ToLine(60), false, false);

        let surface = render(&mut app, 41, 10);
        let first = row(&surface, 0);
        let (left, right) = first.split_once('│').expect("the rule divides the row");

        // Not the exact line numbers: where the far window lands also depends on
        // `scrolloff`. What matters is that the two are looking at different
        // parts of the same file.
        assert!(left.contains("line 0"), "row was {first:?}");
        assert!(!right.contains("line 0"), "row was {first:?}");
        assert_ne!(left.trim(), right.trim());
    }

    #[test]
    fn a_terminal_too_small_for_the_chrome_still_draws() {
        // The status and command bars alone need two rows; anything less has to
        // shrink the text area rather than panic.
        let mut app = app("alpha");
        app.windows.split(Axis::Horizontal);
        render(&mut app, 4, 2);
        render(&mut app, 1, 1);
    }
}
