//! Editing sessions as a user would type them, end to end: keys in, text out.

mod common;

use common::{editor, text, typed};

#[test]
fn renaming_a_variable_with_a_substitution_and_taking_it_back() {
    let mut app = editor("let count = 0;\ncount += 1;\nprint(count);\n");
    typed(&mut app, ":%s/count/total/g<CR>");
    assert_eq!(text(&app), "let total = 0;\ntotal += 1;\nprint(total);\n");
    typed(&mut app, "u");
    assert_eq!(text(&app), "let count = 0;\ncount += 1;\nprint(count);\n");
    typed(&mut app, "<C-r>");
    assert!(text(&app).contains("total += 1"));
}

#[test]
fn a_macro_turns_lines_into_a_list() {
    let mut app = editor("apple\nbanana\ncherry\n");
    typed(&mut app, "qaI- <Esc>jq2@a");
    assert_eq!(text(&app), "- apple\n- banana\n- cherry\n");
}

#[test]
fn changing_inside_quotes_then_repeating_it() {
    let mut app = editor("a(\"one\")\nb(\"two\")\n");
    typed(&mut app, "f\"ci\"x<Esc>j0f\".");
    assert_eq!(text(&app), "a(\"x\")\nb(\"x\")\n");
}

#[test]
fn a_column_of_text_is_edited_at_once() {
    let mut app = editor("int a;\nint b;\nint c;\n");
    typed(&mut app, "<A-v>jjIconst <Esc>");
    assert_eq!(text(&app), "const int a;\nconst int b;\nconst int c;\n");
}

#[test]
fn registers_keep_two_snippets_apart() {
    let mut app = editor("first\nsecond\n");
    typed(&mut app, "\"ayyj\"byy\"aP");
    assert_eq!(text(&app), "first\nfirst\nsecond\n");
    typed(&mut app, "G\"bp");
    assert!(
        text(&app).ends_with("second\n\nsecond") || text(&app).ends_with("second\nsecond\n"),
        "{}",
        text(&app)
    );
}

#[test]
fn commenting_a_paragraph_in_python() {
    let mut app = editor("");
    app.buffers[0] =
        termi::editor::buffer::Buffer::new(termi::editor::document::Document::from_text(
            "x = 1\ny = 2\n\nz = 3\n",
            Some("script.py".into()),
        ));
    typed(&mut app, "gcip");
    assert_eq!(text(&app), "# x = 1\n# y = 2\n\nz = 3\n");
}

#[test]
fn a_mapping_from_the_config_runs_its_command() {
    let mut config = common::config();
    config.keys.normal.insert("<Space>d".into(), "dd".into());
    config.keys.insert.insert("jk".into(), "normal_mode".into());
    let mut app = common::editor_with("one\ntwo\n", config);
    typed(&mut app, "<Space>d");
    assert_eq!(text(&app), "two\n");
    typed(&mut app, "Ahi<Space>jk");
    assert_eq!(app.mode, termi::app::mode::Mode::Normal);
    assert_eq!(text(&app), "twohi \n");
}

#[test]
fn folding_hides_a_block_until_it_is_opened() {
    let mut app = editor("fn a() {\n    one\n    two\n}\nfn b() {}\n");
    typed(&mut app, "zcj");
    assert_eq!(app.window().cursor().head.line, 3);
    typed(&mut app, "zR");
    assert!(app.window().folds.is_empty());
}
