//! # Editing
//!
//! **Purpose:** the operations that change text.
//!
//! **Responsibility:** apply an edit at every cursor, record it in the buffer's
//! [`History`](crate::undo::History), and leave the cursors somewhere sensible.
//!
//! An edit is the one operation that needs both halves of the split between a
//! file and a view of it: it rewrites the [`Buffer`]'s text and history while
//! moving the [`Window`]'s cursors along with it. [`Edit`] borrows the two
//! together for the duration of one operation, which keeps the ownership
//! obvious — the buffer outlives any particular window onto it, so neither can
//! hold the other.
//!
//! Every operation walks the cursors back to front. Editing the last cursor
//! first means the offsets of all the earlier ones are still valid when their
//! turn comes, which removes the whole class of "fix up the other cursors after
//! the edit" bugs.
//!
//! **Public API:** [`Edit`].

use crate::config::Config;
use crate::editor::buffer::Buffer;
use crate::editor::cursor::Position;
use crate::editor::document::indent;
use crate::editor::document::pairs::{self, Insertion};
use crate::editor::selection::Range;
use crate::editor::window::Window;
use crate::undo::Change;

/// A buffer and the window editing it, borrowed together.
#[derive(Debug)]
pub struct Edit<'a> {
    /// The text being changed, and the history recording the change.
    pub buffer: &'a mut Buffer,
    /// The cursors the change happens at.
    pub window: &'a mut Window,
}

impl<'a> Edit<'a> {
    /// Borrow a buffer and a window as one editable unit.
    ///
    /// The caller is responsible for the two matching — `window.buffer` must
    /// identify `buffer` — because only the application knows the buffer list
    /// the index refers to.
    #[must_use]
    pub fn new(buffer: &'a mut Buffer, window: &'a mut Window) -> Self {
        Self { buffer, window }
    }

    /// Insert `text` at every cursor.
    pub fn insert_text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        for index in self.window.edit_order() {
            self.insert_at(index, text, 0);
        }
        self.invalidate_from_first_cursor();
        self.window.resort();
    }

    /// Insert one typed character at every cursor, closing a bracket or a quote
    /// when this is a place one should be closed.
    ///
    /// The decision is taken per cursor rather than once: two cursors are rarely
    /// in the same surroundings, and typing `(` should close at the one at the
    /// end of a line without doing so at the one in the middle of a word.
    pub fn insert_char(&mut self, ch: char, config: &Config) {
        if !config.auto_pairs {
            self.insert_text(&ch.to_string());
            return;
        }
        let mut text = String::with_capacity(8);
        for index in self.window.edit_order() {
            let before = self.window.cursors()[index].head;
            match pairs::resolve(&self.buffer.document, before, ch) {
                Insertion::Plain => {
                    text.clear();
                    text.push(ch);
                    self.insert_at(index, &text, 0);
                }
                Insertion::Pair(close) => {
                    text.clear();
                    text.push(ch);
                    text.push(close);
                    self.insert_at(index, &text, 1);
                }
                // Nothing is inserted, so nothing needs recording: stepping over
                // a bracket is a movement, and undo should not stop on it.
                Insertion::Skip => {
                    let after = Position::new(before.line, before.col + 1);
                    self.window.cursors_mut()[index].move_to(after, false);
                }
            }
        }
        self.invalidate_from_first_cursor();
        self.window.resort();
    }

    /// Insert `text` at one cursor, leaving the caret `back` characters short of
    /// the end of it.
    ///
    /// `back` is what an auto-closed pair needs: the caret belongs between the
    /// two halves, not after both.
    fn insert_at(&mut self, index: usize, text: &str, back: usize) {
        let before = self.window.cursors()[index].head;
        let at = self.buffer.document.pos_to_char(before);

        self.buffer.document.insert(at, text);
        let landed = at + text.chars().count() - back;
        let after = self.buffer.document.char_to_pos(landed);
        self.buffer
            .history
            .record(Change::insertion(at, text), before, after);
        self.window.cursors_mut()[index].move_to(after, false);
    }

    /// Break the line at every cursor, carrying the indentation over.
    ///
    /// Pressing Enter between the halves of a pair opens the block out over
    /// three lines, leaving the caret on the blank one in the middle — the
    /// shape everybody wants after typing `{`.
    pub fn insert_newline(&mut self, config: &Config) {
        for index in self.window.edit_order() {
            let before = self.window.cursors()[index].head;

            let mut text = String::from("\n");
            if config.auto_indent {
                text.push_str(&indent::auto_indent_for_new_line(
                    &self.buffer.document,
                    before.line,
                    before.col,
                    config.tab_width,
                    config.expand_tabs,
                ));
            }
            let mut back = 0;
            if config.auto_pairs && pairs::surrounds(&self.buffer.document, before) {
                // The closing half lines up with the line that opened it, not
                // with the indented line the caret is left on.
                let trailer = format!(
                    "\n{}",
                    indent::indent_of(&self.buffer.document, before.line)
                );
                back = trailer.chars().count();
                text.push_str(&trailer);
            }

            self.insert_at(index, &text, back);
        }
        self.invalidate_from_first_cursor();
        self.window.resort();
    }

    /// Insert one indentation step at every cursor.
    ///
    /// With `expand_tabs` the step is only as wide as it needs to be to reach
    /// the next tab stop, so pressing Tab mid-line lines up instead of always
    /// jumping a full `tab_width`.
    pub fn insert_indent(&mut self, config: &Config) {
        if !config.expand_tabs {
            self.insert_text("\t");
            return;
        }
        for index in self.window.edit_order() {
            let before = self.window.cursors()[index].head;
            let width = config.tab_width - (before.col % config.tab_width);
            let text = " ".repeat(width);
            let at = self.buffer.document.pos_to_char(before);

            self.buffer.document.insert(at, &text);
            let after = self.buffer.document.char_to_pos(at + width);
            self.buffer
                .history
                .record(Change::insertion(at, text.as_str()), before, after);
            self.window.cursors_mut()[index].move_to(after, false);
        }
        self.invalidate_from_first_cursor();
        self.window.resort();
    }

    /// Delete the character before every cursor.
    ///
    /// Inside leading whitespace this removes a whole indentation step, which is
    /// what makes backspace feel symmetric with Tab.
    pub fn delete_backward(&mut self, config: &Config) {
        for index in self.window.edit_order() {
            let before = self.window.cursors()[index].head;
            let at = self.buffer.document.pos_to_char(before);
            if at == 0 {
                continue;
            }
            let start = at - self.backspace_width(before, config);
            // Backspacing between the halves of a pair takes both, so a bracket
            // typed by mistake goes away in one keystroke instead of leaving its
            // closing half stranded.
            let end = if config.auto_pairs && pairs::surrounds(&self.buffer.document, before) {
                at + 1
            } else {
                at
            };

            let removed = self.buffer.document.remove(start, end);
            let after = self.buffer.document.char_to_pos(start);
            self.buffer
                .history
                .record(Change::deletion(start, removed), before, after);
            self.window.cursors_mut()[index].move_to(after, false);
        }
        self.invalidate_from_first_cursor();
        self.window.resort();
    }

    /// Delete the character under every cursor.
    pub fn delete_forward(&mut self) {
        for index in self.window.edit_order() {
            let before = self.window.cursors()[index].head;
            let at = self.buffer.document.pos_to_char(before);
            if at >= self.buffer.document.len_chars() {
                continue;
            }

            let removed = self.buffer.document.remove(at, at + 1);
            let after = self.buffer.document.char_to_pos(at);
            self.buffer
                .history
                .record(Change::deletion(at, removed), before, after);
            self.window.cursors_mut()[index].move_to(after, false);
        }
        self.invalidate_from_first_cursor();
        self.window.resort();
    }

    /// Delete the selection at every cursor that has one.
    ///
    /// Returns whether anything was removed, which is what lets the caller fall
    /// back to the plain behaviour of the key that asked: Backspace with no
    /// selection still deletes one character.
    ///
    /// This is the per-cursor sibling of [`Edit::delete_range`]: it keeps every
    /// cursor, because typing over three selections at three cursors has to
    /// leave three carets behind.
    pub fn delete_selections(&mut self) -> bool {
        let mut removed_any = false;
        for index in self.window.edit_order() {
            let cursor = self.window.cursors()[index];
            if !cursor.has_selection() {
                continue;
            }
            let range = Range::between(&cursor, &self.buffer.document);
            let before = cursor.head;

            let removed = self.buffer.document.remove(range.start, range.end);
            let after = self.buffer.document.char_to_pos(range.start);
            self.buffer
                .history
                .record(Change::deletion(range.start, removed), before, after);
            self.window.cursors_mut()[index].move_to(after, false);
            removed_any = true;
        }
        if removed_any {
            self.invalidate_from_first_cursor();
            self.window.resort();
        }
        removed_any
    }

    /// Delete a span and collapse the primary cursor onto its start.
    ///
    /// Returns the removed text so callers can put it on the clipboard.
    pub fn delete_range(&mut self, range: Range) -> String {
        if range.is_empty() {
            return String::new();
        }
        let before = self.window.cursor().head;
        let removed = self.buffer.document.remove(range.start, range.end);
        let after = self.buffer.document.char_to_pos(range.start);

        self.buffer.history.record(
            Change::deletion(range.start, removed.clone()),
            before,
            after,
        );
        self.window.clear_secondary_cursors();
        self.window.cursor_mut().move_to(after, false);
        self.buffer.invalidate_syntax_from(after.line);
        self.buffer.history.checkpoint();
        removed
    }

    /// Insert clipboard content at the primary cursor.
    ///
    /// Line-wise content goes on its own line *below* the caret, the way `p`
    /// behaves in vi; a fragment is spliced in at the caret. Getting this wrong
    /// is the difference between pasting a function after the current one and
    /// pasting it into the middle of a line.
    pub fn paste(&mut self, text: &str, line_wise: bool) {
        if text.is_empty() {
            return;
        }
        if !line_wise {
            self.insert_text(text);
            self.checkpoint();
            return;
        }

        let before = self.window.cursor().head;
        let next_line = before.line + 1;
        let (at, payload) = if next_line <= self.buffer.document.last_line() {
            let mut payload = text.to_string();
            if !payload.ends_with('\n') {
                payload.push('\n');
            }
            (self.buffer.document.line_start(next_line), payload)
        } else {
            // Nothing follows this line, so the payload has to bring its own
            // leading newline instead of a trailing one.
            let payload = format!("\n{}", text.trim_end_matches('\n'));
            (self.buffer.document.len_chars(), payload)
        };

        self.buffer.document.insert(at, &payload);
        let landed = self
            .buffer
            .document
            .char_to_pos(at + usize::from(payload.starts_with('\n')));
        self.buffer
            .history
            .record(Change::insertion(at, payload.as_str()), before, landed);

        self.window.clear_secondary_cursors();
        self.window.cursor_mut().move_to(landed, false);
        self.buffer.invalidate_syntax_from(before.line);
        self.checkpoint();
    }

    /// Strip trailing spaces and tabs from every line.
    ///
    /// Returns how many lines changed. Runs on save, back to front so earlier
    /// offsets stay valid, and as a single undo step so one `u` puts it all back.
    pub fn trim_trailing_whitespace(&mut self) -> usize {
        let before = self.window.cursor().head;
        let mut trimmed = 0;

        self.buffer.history.checkpoint();
        for line in (0..self.buffer.document.len_lines()).rev() {
            let text = self.buffer.document.line_string(line);
            let kept = text.trim_end_matches([' ', '\t']).chars().count();
            let length = text.chars().count();
            if kept == length {
                continue;
            }
            let start = self.buffer.document.line_start(line) + kept;
            let removed = self.buffer.document.remove(start, start + (length - kept));
            let after = self.buffer.document.char_to_pos(start);
            self.buffer
                .history
                .record(Change::deletion(start, removed), before, after);
            trimmed += 1;
        }

        if trimmed > 0 {
            self.buffer.invalidate_syntax_from(0);
            self.window.clamp_cursors(&self.buffer.document, true);
            self.buffer.history.checkpoint();
        }
        trimmed
    }

    /// Undo one step, moving the caret to where the edit started.
    ///
    /// Returns `false` when there is nothing left to undo.
    pub fn undo(&mut self) -> bool {
        let Some(position) = self.buffer.history.undo(&mut self.buffer.document) else {
            return false;
        };
        self.restore_caret(position);
        true
    }

    /// Redo one step.
    ///
    /// Returns `false` when there is nothing to redo.
    pub fn redo(&mut self) -> bool {
        let Some(position) = self.buffer.history.redo(&mut self.buffer.document) else {
            return false;
        };
        self.restore_caret(position);
        true
    }

    /// End the current undo step, so the next edit starts a new one.
    pub fn checkpoint(&mut self) {
        self.buffer.history.checkpoint();
    }

    /// Invalidate the syntax cache from the topmost cursor downwards.
    ///
    /// Cursors are in document order, so the first one is the earliest line any
    /// of them can have touched.
    fn invalidate_from_first_cursor(&mut self) {
        let line = self.window.cursors()[0].head.line;
        self.buffer.invalidate_syntax_from(line);
    }

    /// How many characters backspace should remove at `position`.
    fn backspace_width(&self, position: Position, config: &Config) -> usize {
        if !config.expand_tabs || position.col == 0 {
            return 1;
        }
        // Only collapse a full indent step when everything to the left is
        // spaces; otherwise backspace inside a word would eat several letters.
        let leading_spaces = self
            .buffer
            .document
            .line(position.line)
            .chars()
            .take(position.col)
            .all(|ch| ch == ' ');
        if !leading_spaces {
            return 1;
        }
        let step = (position.col - 1) % config.tab_width + 1;
        step.min(position.col)
    }

    /// Put a single cursor at `position` after a history operation.
    fn restore_caret(&mut self, position: Position) {
        self.window.clear_secondary_cursors();
        let clamped = self.buffer.document.clamp(position, true);
        self.window.cursor_mut().move_to(clamped, false);
        self.buffer.invalidate_syntax_from(clamped.line);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::cursor::Cursor;
    use crate::editor::document::Document;

    /// A buffer and a window on it, the pair every edit needs.
    struct Fixture {
        buffer: Buffer,
        window: Window,
    }

    impl Fixture {
        fn new(text: &str) -> Self {
            Self {
                buffer: Buffer::new(Document::from_text(text, None)),
                window: Window::new(0),
            }
        }

        fn edit(&mut self) -> Edit<'_> {
            Edit::new(&mut self.buffer, &mut self.window)
        }

        fn text(&self) -> String {
            self.buffer.document.text().to_string()
        }

        fn head(&self) -> Position {
            self.window.cursor().head
        }

        fn caret_to(&mut self, line: usize, col: usize) {
            self.window
                .cursor_mut()
                .move_to(Position::new(line, col), false);
        }
    }

    fn config() -> Config {
        Config::default()
    }

    #[test]
    fn typing_moves_the_caret_along() {
        let mut fixture = Fixture::new("");
        fixture.edit().insert_text("hi");
        assert_eq!(fixture.text(), "hi");
        assert_eq!(fixture.head(), Position::new(0, 2));
    }

    #[test]
    fn newline_carries_the_indentation_over() {
        let mut fixture = Fixture::new("    let x = 1;");
        fixture.caret_to(0, 14);
        fixture.edit().insert_newline(&config());
        assert_eq!(fixture.text(), "    let x = 1;\n    ");
        assert_eq!(fixture.head(), Position::new(1, 4));
    }

    #[test]
    fn newline_after_an_opening_brace_indents_one_more_level() {
        let mut fixture = Fixture::new("fn main() {");
        fixture.caret_to(0, 11);
        fixture.edit().insert_newline(&config());
        assert_eq!(fixture.text(), "fn main() {\n    ");
    }

    #[test]
    fn tab_aligns_to_the_next_tab_stop() {
        let mut fixture = Fixture::new("ab");
        fixture.caret_to(0, 2);
        fixture.edit().insert_indent(&config());
        assert_eq!(fixture.text(), "ab  ");
    }

    #[test]
    fn backspace_removes_a_whole_indent_step() {
        let mut fixture = Fixture::new("        x");
        fixture.caret_to(0, 8);
        fixture.edit().delete_backward(&config());
        assert_eq!(fixture.text(), "    x");
    }

    #[test]
    fn backspace_inside_a_word_removes_one_character() {
        let mut fixture = Fixture::new("word");
        fixture.caret_to(0, 4);
        fixture.edit().delete_backward(&config());
        assert_eq!(fixture.text(), "wor");
    }

    #[test]
    fn backspace_at_the_start_of_a_line_joins_it_to_the_previous_one() {
        let mut fixture = Fixture::new("ab\ncd");
        fixture.caret_to(1, 0);
        fixture.edit().delete_backward(&config());
        assert_eq!(fixture.text(), "abcd");
        assert_eq!(fixture.head(), Position::new(0, 2));
    }

    #[test]
    fn multiple_cursors_all_receive_the_edit() {
        let mut fixture = Fixture::new("a\na\na");
        fixture.window.add_cursor(Cursor::at(Position::new(1, 0)));
        fixture.window.add_cursor(Cursor::at(Position::new(2, 0)));
        fixture.edit().insert_text("x");
        assert_eq!(fixture.text(), "xa\nxa\nxa");
        assert_eq!(fixture.window.cursors().len(), 3);
    }

    #[test]
    fn deleting_a_range_returns_the_removed_text() {
        let mut fixture = Fixture::new("hello world");
        let removed = fixture.edit().delete_range(Range { start: 0, end: 6 });
        assert_eq!(removed, "hello ");
        assert_eq!(fixture.text(), "world");
        assert_eq!(fixture.head(), Position::ZERO);
    }

    #[test]
    fn a_line_wise_paste_lands_on_its_own_line_below() {
        let mut fixture = Fixture::new("first\nsecond");
        fixture.edit().paste("copied\n", true);
        assert_eq!(fixture.text(), "first\ncopied\nsecond");
        assert_eq!(fixture.head(), Position::new(1, 0));
    }

    #[test]
    fn a_line_wise_paste_on_the_last_line_appends_a_new_line() {
        let mut fixture = Fixture::new("only");
        fixture.edit().paste("copied\n", true);
        assert_eq!(fixture.text(), "only\ncopied");
        assert_eq!(fixture.head(), Position::new(1, 0));
    }

    #[test]
    fn a_fragment_paste_splices_into_the_current_line() {
        let mut fixture = Fixture::new("ac");
        fixture.caret_to(0, 1);
        fixture.edit().paste("b", false);
        assert_eq!(fixture.text(), "abc");
    }

    /// Type `text` one character at a time, the way the keyboard delivers it.
    fn type_chars(fixture: &mut Fixture, text: &str) {
        for ch in text.chars() {
            fixture.edit().insert_char(ch, &config());
        }
    }

    #[test]
    fn an_opening_bracket_brings_its_closing_half() {
        let mut fixture = Fixture::new("");
        type_chars(&mut fixture, "(");
        assert_eq!(fixture.text(), "()");
        assert_eq!(fixture.head(), Position::new(0, 1), "caret goes between");
    }

    #[test]
    fn typing_through_a_pair_does_not_double_the_bracket() {
        let mut fixture = Fixture::new("");
        type_chars(&mut fixture, "(a)");
        assert_eq!(fixture.text(), "(a)");
        assert_eq!(fixture.head(), Position::new(0, 3));
    }

    #[test]
    fn a_quote_pairs_and_then_closes_itself() {
        let mut fixture = Fixture::new("");
        type_chars(&mut fixture, "\"hi\"");
        assert_eq!(fixture.text(), "\"hi\"");
    }

    #[test]
    fn stepping_over_a_bracket_is_not_its_own_undo_step() {
        let mut fixture = Fixture::new("");
        type_chars(&mut fixture, "()");
        // One insertion happened, so one undo takes the whole pair away.
        assert!(fixture.edit().undo());
        assert_eq!(fixture.text(), "");
    }

    #[test]
    fn a_bracket_typed_in_front_of_a_word_is_not_closed() {
        let mut fixture = Fixture::new("word");
        type_chars(&mut fixture, "(");
        assert_eq!(fixture.text(), "(word");
    }

    #[test]
    fn backspace_between_a_pair_removes_both_halves() {
        let mut fixture = Fixture::new("");
        type_chars(&mut fixture, "{");
        fixture.edit().delete_backward(&config());
        assert_eq!(fixture.text(), "");
        assert_eq!(fixture.head(), Position::ZERO);
    }

    #[test]
    fn backspace_beside_an_unmatched_bracket_removes_only_it() {
        let mut fixture = Fixture::new("(]");
        fixture.caret_to(0, 1);
        fixture.edit().delete_backward(&config());
        assert_eq!(fixture.text(), "]");
    }

    #[test]
    fn enter_between_a_pair_opens_the_block_out() {
        let mut fixture = Fixture::new("fn main() ");
        fixture.caret_to(0, 10);
        type_chars(&mut fixture, "{");
        fixture.edit().insert_newline(&config());

        assert_eq!(fixture.text(), "fn main() {\n    \n}");
        assert_eq!(fixture.head(), Position::new(1, 4));
    }

    #[test]
    fn an_opened_block_keeps_the_indentation_of_the_line_that_opened_it() {
        let mut fixture = Fixture::new("    if x {}");
        fixture.caret_to(0, 10);
        fixture.edit().insert_newline(&config());

        assert_eq!(fixture.text(), "    if x {\n        \n    }");
        assert_eq!(fixture.head(), Position::new(1, 8));
    }

    #[test]
    fn pairs_can_be_switched_off() {
        let plain = Config {
            auto_pairs: false,
            ..config()
        };
        let mut fixture = Fixture::new("");
        fixture.edit().insert_char('(', &plain);
        assert_eq!(fixture.text(), "(");
    }

    #[test]
    fn every_cursor_decides_for_itself_whether_to_close() {
        // One cursor at the end of a line, one in front of a word.
        let mut fixture = Fixture::new("\nword");
        fixture.window.add_cursor(Cursor::at(Position::new(1, 0)));
        fixture.edit().insert_char('(', &config());
        assert_eq!(fixture.text(), "()\n(word");
    }

    #[test]
    fn undo_and_redo_round_trip_an_edit() {
        let mut fixture = Fixture::new("start");
        fixture.caret_to(0, 5);
        fixture.edit().insert_text("!");
        assert!(fixture.edit().undo());
        assert_eq!(fixture.text(), "start");
        assert!(fixture.edit().redo());
        assert_eq!(fixture.text(), "start!");
        assert!(!fixture.edit().redo());
    }

    #[test]
    fn two_windows_on_one_buffer_edit_the_same_text() {
        let mut buffer = Buffer::new(Document::from_text("shared", None));
        let mut first = Window::new(0);
        let mut second = Window::new(0);

        Edit::new(&mut buffer, &mut first).insert_text(">");
        Edit::new(&mut buffer, &mut second).insert_text("<");

        assert_eq!(buffer.document.text().to_string(), "<>shared");
        // Each window moved its own caret and only its own.
        assert_eq!(first.cursor().head, Position::new(0, 1));
        assert_eq!(second.cursor().head, Position::new(0, 1));
    }

    #[test]
    fn one_history_serves_every_window_on_a_buffer() {
        let mut buffer = Buffer::new(Document::from_text("", None));
        let mut first = Window::new(0);
        let mut second = Window::new(0);

        Edit::new(&mut buffer, &mut first).insert_text("typed");
        // The second window undoes an edit it did not make, because the history
        // belongs to the file rather than to the view of it.
        assert!(Edit::new(&mut buffer, &mut second).undo());
        assert_eq!(buffer.document.text().to_string(), "");
    }
}
