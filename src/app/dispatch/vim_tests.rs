//! End-to-end tests of the vi command language: keys go in through
//! [`Input`], exactly as typed, and the text that comes out is checked.

use std::path::PathBuf;

use super::apply;
use crate::app::mode::Mode;
use crate::app::state::App;
use crate::config::Config;
use crate::editor::buffer::Buffer;
use crate::editor::cursor::Position;
use crate::editor::document::Document;
use crate::input::{Input, notation};

/// An editor holding `text`, with the caret at the start.
fn editor(text: &str) -> App {
    editor_for(text, None)
}

fn editor_for(text: &str, path: Option<&str>) -> App {
    let mut app = App::with_config(Config {
        system_clipboard: false,
        auto_pairs: false,
        ..Config::default()
    });
    app.buffers[0] = Buffer::new(Document::from_text(text, path.map(PathBuf::from)));
    app.windows.focused_mut().reset(0);
    app
}

/// Type `keys`, written in key notation, the way the event loop would.
fn typed(app: &mut App, keys: &str) {
    let mut input = Input::default();
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

fn feed(app: &mut App, input: &mut Input, key: crossterm::event::KeyEvent) {
    input.recording = app.macro_recording.is_some();
    let action = input.handle(key, app.mode);
    apply(app, action).expect("dispatch should not fail");
}

fn text(app: &App) -> String {
    app.buffer().document.text().to_string()
}

fn caret(app: &App) -> Position {
    app.window().cursor().head
}

#[test]
fn delete_word_with_counts_either_side() {
    let mut app = editor("one two three four");
    typed(&mut app, "2dw");
    assert_eq!(text(&app), "three four");
    typed(&mut app, "d1w");
    assert_eq!(text(&app), "four");
}

#[test]
fn change_word_stops_at_the_word_and_undoes_in_one_step() {
    let mut app = editor("old name");
    typed(&mut app, "cwnew<Esc>");
    assert_eq!(text(&app), "new name");
    assert_eq!(app.mode, Mode::Normal);
    typed(&mut app, "u");
    assert_eq!(text(&app), "old name");
}

#[test]
fn change_inside_brackets() {
    let mut app = editor("call(a, b) after");
    typed(&mut app, "f(lci(x<Esc>");
    assert_eq!(text(&app), "call(x) after");
}

#[test]
fn delete_till_and_repeat_the_find() {
    let mut app = editor("a,b,c,d");
    typed(&mut app, "dt,");
    assert_eq!(text(&app), ",b,c,d");
    // `;` remembers the `t`, and does not stick against the comma it is on.
    let mut app = editor("a,b,c,d");
    typed(&mut app, "t,;");
    assert_eq!(caret(&app), Position::new(0, 2));
}

#[test]
fn dot_repeats_an_operator_and_an_insert() {
    let mut app = editor("a b c d");
    typed(&mut app, "dw.");
    assert_eq!(text(&app), "c d");

    let mut app = editor("one\ntwo\nthree");
    typed(&mut app, "A;<Esc>j.j.");
    assert_eq!(text(&app), "one;\ntwo;\nthree;");
}

#[test]
fn dot_with_a_count_replaces_the_original_count() {
    let mut app = editor("a b c d e f");
    typed(&mut app, "dw3.");
    assert_eq!(text(&app), "e f");
}

#[test]
fn lines_yank_and_put() {
    let mut app = editor("first\nsecond");
    typed(&mut app, "yyjp");
    assert_eq!(text(&app), "first\nsecond\nfirst");
    typed(&mut app, "ggddp");
    assert_eq!(text(&app), "second\nfirst\nfirst");
}

#[test]
fn delete_the_last_line_takes_its_line_break() {
    let mut app = editor("keep\ngone");
    typed(&mut app, "Gdd");
    assert_eq!(text(&app), "keep");
}

#[test]
fn named_registers_survive_other_yanks() {
    let mut app = editor("alpha\nbeta");
    typed(&mut app, "\"ayyjyy\"ap");
    assert_eq!(text(&app), "alpha\nbeta\nalpha");
}

#[test]
fn a_macro_replays_its_keys() {
    let mut app = editor("a\nb\nc");
    typed(&mut app, "qqA!<Esc>jq2@q");
    assert_eq!(text(&app), "a!\nb!\nc!");
}

#[test]
fn marks_return_to_their_line() {
    let mut app = editor("one\ntwo\nthree");
    typed(&mut app, "jmaG'a");
    assert_eq!(caret(&app).line, 1);
}

#[test]
fn the_jump_list_goes_back_and_forward() {
    let mut app = editor("one\ntwo\nthree\nfour");
    typed(&mut app, "G");
    assert_eq!(caret(&app).line, 3);
    typed(&mut app, "<A-Left>");
    assert_eq!(caret(&app).line, 0);
    typed(&mut app, "<A-Right>");
    assert_eq!(caret(&app).line, 3);
}

#[test]
fn join_replace_toggle_and_shift() {
    let mut app = editor("abc\n    def");
    typed(&mut app, "J");
    assert_eq!(text(&app), "abc def");
    typed(&mut app, "0rx~");
    assert_eq!(text(&app), "Xbc def");
    typed(&mut app, ">>");
    assert_eq!(text(&app), "    Xbc def");
    typed(&mut app, "<<");
    assert_eq!(text(&app), "Xbc def");
}

#[test]
fn case_operators_take_text_objects() {
    let mut app = editor("hello world");
    typed(&mut app, "gUiw");
    assert_eq!(text(&app), "HELLO world");
    typed(&mut app, "wg~~");
    assert_eq!(text(&app), "hello WORLD");
}

#[test]
fn comment_lines_in_the_buffer_language() {
    let mut app = editor_for("let a = 1;\nlet b = 2;", Some("x.rs"));
    typed(&mut app, "gcj");
    assert_eq!(text(&app), "// let a = 1;\n// let b = 2;");
    typed(&mut app, "gcc");
    assert_eq!(text(&app), "let a = 1;\n// let b = 2;");
}

#[test]
fn visual_block_deletes_a_column() {
    let mut app = editor("abc\nabc\nabc");
    typed(&mut app, "<A-v>jjd");
    assert_eq!(text(&app), "bc\nbc\nbc");
}

#[test]
fn visual_block_insert_types_on_every_line() {
    let mut app = editor("a\nb");
    typed(&mut app, "<A-v>jI- <Esc>");
    assert_eq!(text(&app), "- a\n- b");
}

#[test]
fn visual_block_yank_puts_a_rectangle() {
    let mut app = editor("ab\ncd");
    typed(&mut app, "<A-v>jy$p");
    assert_eq!(text(&app), "aba\ncdc");
}

#[test]
fn visual_text_objects_and_operators() {
    let mut app = editor("keep (drop this) keep");
    typed(&mut app, "fdvi(d");
    assert_eq!(text(&app), "keep () keep");

    let mut app = editor("ab cd");
    typed(&mut app, "veU");
    assert_eq!(text(&app), "AB cd");
}

#[test]
fn reselect_brings_back_the_last_selection() {
    let mut app = editor("one two");
    typed(&mut app, "vey");
    typed(&mut app, "gv");
    assert_eq!(app.mode, Mode::Visual);
    typed(&mut app, "d");
    assert_eq!(text(&app), " two");
}

#[test]
fn matching_bracket_and_paragraph_motions() {
    let mut app = editor("(a [b] c)\n\nnext");
    typed(&mut app, "%");
    assert_eq!(caret(&app), Position::new(0, 8));
    typed(&mut app, "}");
    assert_eq!(caret(&app).line, 1);
}

#[test]
fn star_searches_for_the_whole_word() {
    let mut app = editor("foo food foo");
    typed(&mut app, "*");
    assert_eq!(caret(&app), Position::new(0, 9));
}

#[test]
fn change_whole_lines_keeps_indentation() {
    let mut app = editor("    old\nnext");
    typed(&mut app, "ccnew<Esc>");
    assert_eq!(text(&app), "    new\nnext");
}

#[test]
fn delete_a_paragraph_line_wise() {
    let mut app = editor("a\nb\n\nc");
    typed(&mut app, "d}");
    assert_eq!(text(&app), "\nc");
}

#[test]
fn a_count_before_g_names_a_line() {
    let mut app = editor("1\n2\n3\n4");
    typed(&mut app, "3G");
    assert_eq!(caret(&app).line, 2);
    typed(&mut app, "dgg");
    assert_eq!(text(&app), "4");
}

#[test]
fn undoing_back_to_the_saved_text_makes_the_buffer_clean() {
    let mut app = editor("text");
    typed(&mut app, "x");
    assert!(app.buffer().document.is_dirty());
    typed(&mut app, "u");
    assert!(!app.buffer().document.is_dirty());
    typed(&mut app, "<C-r>");
    assert!(app.buffer().document.is_dirty());
}

#[test]
fn g_minus_reaches_an_abandoned_branch() {
    let mut app = editor("");
    typed(&mut app, "iabc<Esc>uixyz<Esc>");
    assert_eq!(text(&app), "xyz");
    typed(&mut app, "g-");
    assert_eq!(text(&app), "abc");
    typed(&mut app, "g+");
    assert_eq!(text(&app), "xyz");
}

#[test]
fn substitute_over_a_range_is_one_undo_step() {
    let mut app = editor("a a\na a\na a\na a");
    typed(&mut app, ":2,3s/a/b/g<CR>");
    assert_eq!(text(&app), "a a\nb b\nb b\na a");
    typed(&mut app, "u");
    assert_eq!(text(&app), "a a\na a\na a\na a");
}

#[test]
fn colon_from_a_selection_works_on_its_lines() {
    let mut app = editor("x\nx\nx");
    typed(&mut app, "jVj:s/x/y/<CR>");
    assert_eq!(text(&app), "x\ny\ny");
}

#[test]
fn confirmed_substitution_asks_about_each_match() {
    let mut app = editor("cat cat cat");
    typed(&mut app, ":s/cat/dog/gc<CR>");
    assert_eq!(app.mode, Mode::Replace);
    typed(&mut app, "yny");
    assert_eq!(text(&app), "dog cat dog");
    assert_eq!(app.mode, Mode::Normal);
    typed(&mut app, "u");
    assert_eq!(text(&app), "cat cat cat");
}

#[test]
fn confirming_all_replaces_the_rest() {
    let mut app = editor("a\na\na");
    typed(&mut app, ":%s/a/b/c<CR>na");
    assert_eq!(text(&app), "a\nb\nb");
}

#[test]
fn help_opens_as_a_searchable_read_only_buffer() {
    let mut app = editor("text");
    typed(&mut app, ":help registers<CR>");
    assert_eq!(app.buffer().document.display_name(), "[help]");
    assert!(app.buffer().document.is_readonly());
    let line = app.buffer().document.line_string(caret(&app).line);
    assert!(line.to_lowercase().contains("registers"), "{line}");
}

#[test]
fn bracket_c_walks_between_changes() {
    use crate::git::Sign;
    let mut app = editor("a\nb\nc\nd\ne");
    app.buffers[0].git.base = Some(String::new());
    app.buffers[0].git.signs = vec![
        None,
        Some(Sign::Added),
        Some(Sign::Added),
        None,
        Some(Sign::Modified),
    ];
    typed(&mut app, "]c");
    assert_eq!(caret(&app).line, 1);
    typed(&mut app, "]c");
    assert_eq!(caret(&app).line, 4);
    typed(&mut app, "[c");
    assert_eq!(caret(&app).line, 1);
}
