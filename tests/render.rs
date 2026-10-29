//! What the screen looks like: whole frames, drawn off-screen.

mod common;

use common::{config, editor_with, screen};

#[test]
fn a_frame_has_gutter_text_status_and_command_line() {
    let mut settings = config();
    settings.syntax_highlighting = false;
    let mut app = editor_with("fn main() {}\n", settings);
    let frame = screen(&mut app, 50, 5);
    assert_eq!(
        frame[0],
        "    1 fn main() {}                                "
    );
    assert_eq!(
        frame[1],
        "    2                                             "
    );
    assert_eq!(
        frame[2],
        "~                                                 "
    );
    assert!(frame[3].starts_with(" NORMAL  [No Name]"), "{}", frame[3]);
    assert!(frame[3].trim_end().ends_with("1:1  0%"), "{}", frame[3]);
}

#[test]
fn a_transparent_theme_leaves_the_background_alone() {
    let mut settings = config();
    settings.transparent_background = true;
    let mut app = editor_with("text\n", settings);
    let mut terminal =
        ratatui::Terminal::new(ratatui::backend::TestBackend::new(20, 4)).expect("backend");
    terminal
        .draw(|frame| termi::ui::draw(frame, &mut app))
        .expect("draw");
    let cell = terminal
        .backend()
        .buffer()
        .cell((10, 1))
        .cloned()
        .expect("cell");
    assert_eq!(cell.bg, ratatui::style::Color::Reset);
}

#[test]
fn the_help_buffer_can_be_searched() {
    let mut app = editor_with("", config());
    common::typed(&mut app, ":help<CR>/MACROS<CR>");
    let line = app
        .buffer()
        .document
        .line_string(app.window().cursor().head.line);
    assert!(line.starts_with("MACROS"), "{line}");
}
