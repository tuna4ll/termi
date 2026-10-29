//! Working with real files: what reaches the disk, and what comes back.

mod common;

use common::{config, scratch, text, typed};
use termi::app::App;

fn opened(path: &std::path::Path) -> App {
    let mut app = App::headless(config());
    app.open(path.to_path_buf()).expect("open file");
    app
}

#[test]
fn a_reload_after_an_outside_change_cannot_be_undone_into_corruption() {
    let path = scratch("reload").join("notes.txt");
    std::fs::write(&path, "original\n").expect("write file");
    let mut app = opened(&path);
    typed(&mut app, "Aedited<Esc>:w<CR>");

    std::fs::write(&path, "rewritten by another program\n").expect("rewrite file");
    typed(&mut app, ":e!<CR>u");
    assert_eq!(text(&app), "rewritten by another program\n");
}

#[test]
fn writing_keeps_line_endings_and_encoding() {
    let path = scratch("encoding").join("windows.txt");
    std::fs::write(&path, b"caf\xe9\r\nbar\r\n").expect("write file");
    let mut app = opened(&path);
    typed(&mut app, "jdd:w<CR>");
    assert_eq!(std::fs::read(&path).expect("read back"), b"caf\xe9\r\n");
}

#[test]
fn editorconfig_sets_the_indentation_of_a_new_file() {
    let directory = scratch("editorconfig");
    std::fs::write(
        directory.join(".editorconfig"),
        "root = true\n[*.go]\nindent_style = tab\n",
    )
    .expect("write editorconfig");
    let app = opened(&directory.join("main.go"));
    assert!(!app.buffer_config(0).expand_tabs);
}

#[test]
fn write_all_and_quit_all() {
    let directory = scratch("write-all");
    let mut app = App::headless(config());
    for name in ["a.txt", "b.txt"] {
        app.open(directory.join(name)).expect("open");
        typed(&mut app, &format!("i{name}<Esc>"));
    }
    typed(&mut app, ":qa<CR>");
    assert!(!app.should_quit(), "unsaved files keep the editor open");
    typed(&mut app, ":wqa<CR>");
    assert!(app.should_quit());
    assert_eq!(
        std::fs::read_to_string(directory.join("b.txt")).unwrap(),
        "b.txt"
    );
}

#[test]
fn a_session_brings_back_files_and_splits() {
    let directory = scratch("session");
    let (a, b) = (directory.join("a.txt"), directory.join("b.txt"));
    std::fs::write(&a, "a\n").unwrap();
    std::fs::write(&b, "b\n").unwrap();
    let mut app = opened(&a);
    typed(&mut app, ":vs<CR>");
    app.open(b).unwrap();
    let session = directory.join("session.json");
    termi::app::session::save_to(&app, &session).expect("save session");

    let mut restored = App::headless(config());
    assert!(termi::app::session::restore_from(&mut restored, &session));
    assert_eq!(restored.windows.count(), 2);
    assert_eq!(restored.buffers.len(), 2);
}
