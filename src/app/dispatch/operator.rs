//! Operators — `d`, `c`, `y`, `>`, `<`, `gu`, `gU`, `g~`, `gc` — and the
//! other commands that change text by span rather than by keystroke: put,
//! replace, join, and everything visual mode does to a selection.
//!
//! Every function here works out its spans first and changes the text second,
//! inside one undo group, so a command at five cursors is still one `u`.

use super::{enter_mode, motion};
use crate::app::mode::Mode;
use crate::app::state::App;
use crate::clipboard::RegisterKind;
use crate::clipboard::registers::Register;
use crate::editor::cursor::{Cursor, Motion, MotionKind, Position, word};
use crate::editor::document::{Document, comment, indent};
use crate::editor::selection::Range;
use crate::editor::textobject::{self, TextObject};
use crate::input::{CaseChange, Goal, Operator, Target};

/// A span an operator acts on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Span {
    range: Range,
    linewise: bool,
}

/// The whole of lines `first..=last`, newline of the last one included.
fn line_span(document: &Document, first: usize, last: usize) -> Span {
    let start = document.line_start(first);
    let end = if last >= document.last_line() {
        document.len_chars()
    } else {
        document.line_start(last + 1)
    };
    Span {
        range: Range::new(start, end),
        linewise: true,
    }
}

/// The lines a span touches.
fn lines_of(document: &Document, span: Span) -> (usize, usize) {
    let first = document.char_to_pos(span.range.start).line;
    let end = if span.range.end > span.range.start {
        span.range.end - 1
    } else {
        span.range.end
    };
    let last = document.char_to_pos(end).line.max(first);
    (first, last)
}

/// `op` with a target, at every cursor.
pub fn operate(app: &mut App, operator: Operator, target: Target, count: usize) {
    let cursors = app.window().cursors().to_vec();
    let primary = app.window().primary_index();
    let mut spans: Vec<(usize, Span)> = cursors
        .iter()
        .enumerate()
        .filter_map(|(index, cursor)| {
            span_for(app, cursor.head, operator, target, count).map(|span| (index, span))
        })
        .collect();
    if spans.is_empty() {
        return;
    }
    spans.sort_by_key(|(_, span)| span.range.start);
    let main = spans
        .iter()
        .find(|(index, _)| *index == primary)
        .unwrap_or(&spans[0])
        .1;
    let spans: Vec<Span> = spans.into_iter().map(|(_, span)| span).collect();
    apply(app, operator, &spans, main, 1, false);
}

fn span_for(
    app: &App,
    head: Position,
    operator: Operator,
    target: Target,
    count: usize,
) -> Option<Span> {
    let document = &app.buffer().document;
    match target {
        Target::Lines => {
            let last = (head.line + count.max(1) - 1).min(document.last_line());
            Some(line_span(document, head.line, last))
        }
        Target::Object { object, inner } => {
            textobject::select(document, head, object, inner, count).map(|found| Span {
                range: found.range,
                linewise: found.linewise,
            })
        }
        Target::Goal(goal) => goal_span(app, head, operator, goal, count),
    }
}

fn goal_span(
    app: &App,
    head: Position,
    operator: Operator,
    goal: Goal,
    count: usize,
) -> Option<Span> {
    let document = &app.buffer().document;
    let start = document.pos_to_char(head);

    // `cw` is `ce` in disguise: changing a word should not eat the blank after
    // it, or every rename would glue the new name to the next word.
    if operator == Operator::Change
        && let Goal::Motion(motion @ (Motion::WordForward | Motion::BigWordForward)) = goal
        && !document
            .text()
            .get_char(start)
            .is_none_or(char::is_whitespace)
    {
        let big = motion == Motion::BigWordForward;
        let (_, mut end) = word::run_at(document, start, big)?;
        for _ in 1..count.max(1) {
            let next = if big {
                word::big_word_end(document, document.char_to_pos(end))
            } else {
                word::word_end(document, document.char_to_pos(end))
            };
            end = document.pos_to_char(next) + 1;
        }
        return Some(Span {
            range: Range::new(start, end),
            linewise: false,
        });
    }

    let kind = motion::kind_of(goal);
    let mut to = motion::resolve(app, head, goal, count, kind == MotionKind::Exclusive)?;
    match kind {
        MotionKind::Linewise => {
            let (first, last) = if to.line < head.line {
                (to.line, head.line)
            } else {
                (head.line, to.line)
            };
            Some(line_span(document, first, last))
        }
        MotionKind::Inclusive => {
            let end = document.pos_to_char(to);
            let (low, high) = if end < start {
                (end, start)
            } else {
                (start, end)
            };
            Some(Span {
                range: Range::new(low, (high + 1).min(document.len_chars())),
                linewise: false,
            })
        }
        MotionKind::Exclusive => {
            // An exclusive motion that ends at the start of a later line stops
            // at the end of the line before — `dw` on a line's last word keeps
            // the line break — and becomes line-wise when it also started at
            // the indentation, which is what makes `d}` delete a paragraph.
            if to.line > head.line && to.col == 0 {
                if head.col <= indent::first_non_blank(document, head.line) {
                    return Some(line_span(document, head.line, to.line - 1));
                }
                to = Position::new(to.line - 1, document.line_len(to.line - 1));
            }
            if matches!(
                goal,
                Goal::Motion(Motion::WordForward | Motion::BigWordForward)
            ) && to.line > head.line
                && count <= 1
            {
                to = Position::new(head.line, document.line_len(head.line));
            }
            let end = document.pos_to_char(to);
            let range = Range::new(start, end);
            (!range.is_empty()).then_some(Span {
                range,
                linewise: false,
            })
        }
    }
}

/// The spans the current visual selection covers.
fn selection_spans(app: &App) -> Vec<Span> {
    let document = &app.buffer().document;
    let cursors = app.window().cursors();
    match app.mode {
        Mode::VisualLine => cursors
            .iter()
            .map(|cursor| Span {
                range: Range::of_lines(cursor, document),
                linewise: true,
            })
            .collect(),
        Mode::VisualBlock => Range::block(&app.window().cursor(), document)
            .into_iter()
            .map(|range| Span {
                range,
                linewise: false,
            })
            .collect(),
        _ => cursors
            .iter()
            .map(|cursor| Span {
                range: Range::of(cursor, document),
                linewise: false,
            })
            .collect(),
    }
}

/// Apply `op` to the visual selection and leave visual mode.
pub fn operate_selection(app: &mut App, operator: Operator, count: usize) {
    let block = app.mode == Mode::VisualBlock;
    let mut spans = selection_spans(app);
    spans.sort_by_key(|span| span.range.start);
    let Some(main) = spans.first().copied() else {
        return;
    };
    // Shifting by a count is the one place visual mode reads one: `3>`.
    let times = if matches!(operator, Operator::Indent | Operator::Dedent) {
        count.max(1)
    } else {
        1
    };
    enter_mode(app, Mode::Normal);
    apply(app, operator, &spans, main, times, block);
}

/// The text a yank or delete of `spans` puts in a register.
fn register_text(
    document: &Document,
    spans: &[Span],
    main: Span,
    block: bool,
) -> (String, RegisterKind) {
    if block {
        let lines: Vec<String> = spans
            .iter()
            .map(|span| document.slice_string(span.range.start, span.range.end))
            .collect();
        return (lines.join("\n"), RegisterKind::Blockwise);
    }
    let mut text = document.slice_string(main.range.start, main.range.end);
    if main.linewise {
        if !text.ends_with('\n') {
            text.push('\n');
        }
        (text, RegisterKind::Linewise)
    } else {
        (text, RegisterKind::Charwise)
    }
}

/// The range a line-wise delete removes: at the end of the file the line
/// break *before* the lines goes too, or a blank line would be left behind.
fn deletion_range(document: &Document, span: Span) -> Range {
    let mut range = span.range;
    if span.linewise
        && range.end == document.len_chars()
        && range.start > 0
        && !document
            .slice_string(range.start, range.end)
            .ends_with('\n')
    {
        range.start -= 1;
    }
    range
}

fn apply(app: &mut App, operator: Operator, spans: &[Span], main: Span, times: usize, block: bool) {
    let (text, kind) = register_text(&app.buffer().document, spans, main, block);
    match operator {
        Operator::Yank => {
            let lines = text.lines().count();
            app.registers.yank(text, kind);
            let document = &app.buffer().document;
            let head = app.window().cursor().head;
            let start = document.char_to_pos(main.range.start);
            let position = if main.linewise {
                document.clamp(Position::new(start.line, head.col), false)
            } else {
                start
            };
            place(app, vec![position]);
            if lines > 1 {
                app.info(format!("{lines} lines yanked"));
            }
        }
        Operator::Delete => {
            app.registers.delete(text, kind);
            let replacements: Vec<(Range, String)> = spans
                .iter()
                .map(|span| (deletion_range(&app.buffer().document, *span), String::new()))
                .collect();
            let starts = edit_group(app, &replacements);
            let document = &app.buffer().document;
            let positions = starts
                .into_iter()
                .map(|start| {
                    let position = document.char_to_pos(start);
                    if main.linewise {
                        let line = position.line.min(document.last_line());
                        // The removed line's successor moves up into place; a
                        // delete at the end of the file lands on the line above.
                        let line = if start >= document.len_chars()
                            && line > 0
                            && document.line_len(line) == 0
                            && start > 0
                        {
                            document.char_to_pos(start.saturating_sub(1)).line
                        } else {
                            line
                        };
                        Position::new(line, indent::first_non_blank(document, line))
                    } else {
                        position
                    }
                })
                .collect();
            place(app, positions);
            app.clamp_cursors(false);
        }
        Operator::Change => change(app, spans, text, kind),
        Operator::Indent | Operator::Dedent => {
            shift_lines(app, spans, operator == Operator::Indent, times);
        }
        Operator::Case(change) => {
            let document = &app.buffer().document;
            let replacements: Vec<(Range, String)> = spans
                .iter()
                .map(|span| {
                    let text = document.slice_string(span.range.start, span.range.end);
                    (span.range, change.apply(&text))
                })
                .collect();
            let starts = edit_group(app, &replacements);
            let document = &app.buffer().document;
            let positions = starts
                .into_iter()
                .map(|start| document.char_to_pos(start))
                .collect();
            place(app, positions);
            app.clamp_cursors(false);
        }
        Operator::Comment => {
            let lines: Vec<(usize, usize)> = spans
                .iter()
                .map(|span| lines_of(&app.buffer().document, *span))
                .collect();
            comment_lines(app, &lines);
        }
    }
}

/// Run replacements as one undo step.
fn edit_group(app: &mut App, replacements: &[(Range, String)]) -> Vec<usize> {
    let mut edit = app.edit();
    edit.begin_group();
    let starts = edit.replace_many(replacements);
    edit.end_group();
    starts
}

/// Put one caret at each position, the first being primary.
fn place(app: &mut App, positions: Vec<Position>) {
    if positions.is_empty() {
        return;
    }
    let cursors = positions.into_iter().map(Cursor::at).collect();
    app.window_mut().set_cursors(cursors, 0);
}

fn change(app: &mut App, spans: &[Span], text: String, kind: RegisterKind) {
    app.registers.delete(text, kind);
    let auto_indent = app.config.auto_indent;
    let document = &app.buffer().document;
    let mut indents = Vec::new();
    let replacements: Vec<(Range, String)> = spans
        .iter()
        .map(|span| {
            if span.linewise {
                // `cc` keeps the line and its indentation, and empties the rest.
                let (first, last) = lines_of(document, *span);
                let start = document.line_start(first);
                let end = document.line_start(last) + document.line_len(last);
                let indent = if auto_indent {
                    indent::indent_of(document, first)
                } else {
                    String::new()
                };
                indents.push(indent.chars().count());
                (Range::new(start, end), indent)
            } else {
                indents.push(0);
                (span.range, String::new())
            }
        })
        .collect();

    // The group stays open until insert mode ends, so `cw` and the word typed
    // after it come back with a single `u`.
    let mut edit = app.edit();
    edit.begin_group();
    let starts = edit.replace_many(&replacements);
    app.insert_group_open = true;

    let document = &app.buffer().document;
    let positions: Vec<Position> = starts
        .iter()
        .zip(indents)
        .map(|(start, indent)| document.char_to_pos(start + indent))
        .collect();
    enter_mode(app, Mode::Insert);
    place(app, positions);
}

fn shift_lines(app: &mut App, spans: &[Span], indent_right: bool, times: usize) {
    let tab_width = app.config.tab_width;
    let unit = indent::indent_unit(tab_width, app.config.expand_tabs).repeat(times);
    let document = &app.buffer().document;
    let mut lines: Vec<usize> = spans
        .iter()
        .flat_map(|span| {
            let (first, last) = lines_of(document, *span);
            first..=last
        })
        .collect();
    lines.sort_unstable();
    lines.dedup();

    let replacements: Vec<(Range, String)> = lines
        .iter()
        .filter_map(|&line| {
            let start = document.line_start(line);
            if indent_right {
                (document.line_len(line) > 0).then(|| (Range::new(start, start), unit.clone()))
            } else {
                let mut width = 0;
                let mut remove = 0;
                for ch in document.line_string(line).chars() {
                    let step = match ch {
                        ' ' => 1,
                        '\t' => tab_width,
                        _ => break,
                    };
                    if width + step > tab_width * times {
                        break;
                    }
                    width += step;
                    remove += 1;
                }
                (remove > 0).then(|| (Range::new(start, start + remove), String::new()))
            }
        })
        .collect();
    edit_group(app, &replacements);
    if let Some(&first) = lines.first() {
        let document = &app.buffer().document;
        place(
            app,
            vec![Position::new(
                first,
                indent::first_non_blank(document, first),
            )],
        );
        app.clamp_cursors(false);
    }
}

/// Toggle comments over runs of lines.
fn comment_lines(app: &mut App, runs: &[(usize, usize)]) {
    let Some(language) = app.buffer().syntax.language() else {
        app.error("no comment syntax for plain text");
        return;
    };
    let tokens = comment::Tokens {
        line: language.line_comment,
        block: language.block_comment,
    };
    let document = &app.buffer().document;
    let mut replacements = Vec::new();
    let mut seen = Vec::new();
    for &(first, last) in runs {
        if seen.iter().any(|&(a, b)| first <= b && last >= a) {
            continue;
        }
        seen.push((first, last));
        replacements.extend(comment::toggle(document, first, last, tokens));
    }
    if replacements.is_empty() {
        if tokens.line.is_none() && tokens.block.is_none() {
            app.error(format!("no comment syntax for {}", language.name));
        }
        return;
    }
    replacements.sort_by_key(|(range, _)| range.start);
    let caret = app.window().cursor().head;
    edit_group(app, &replacements);
    let position = app.buffer().document.clamp(caret, false);
    place(app, vec![position]);
}

/// Ctrl+/ — comment or uncomment the lines under every cursor, or the
/// selection's lines.
pub fn toggle_comment(app: &mut App) {
    if app.mode.is_visual() {
        return operate_selection(app, Operator::Comment, 1);
    }
    let document = &app.buffer().document;
    let runs: Vec<(usize, usize)> = app
        .window()
        .cursors()
        .iter()
        .map(|cursor| {
            let (a, b) = (cursor.anchor.line, cursor.head.line);
            // A selection ending at the very start of a line does not reach
            // into it.
            let (first, mut last) = if a <= b { (a, b) } else { (b, a) };
            let end = cursor.anchor.max(cursor.head);
            if last > first && end.col == 0 && cursor.has_selection() {
                last -= 1;
            }
            (
                first.min(document.last_line()),
                last.min(document.last_line()),
            )
        })
        .collect();
    let allow_eol = app.mode.is_insert();
    let carets: Vec<Cursor> = app.window().cursors().to_vec();
    comment_lines(app, &runs);
    // Keep the carets on their lines; the marker shifted the text under them.
    let document = &app.buffer().document;
    let cursors = carets
        .into_iter()
        .map(|cursor| Cursor::at(document.clamp(cursor.head, allow_eol)))
        .collect();
    app.window_mut().set_cursors(cursors, 0);
}

/// `%`, `:`, `/` and the rest of the read-only registers, then the normal ones.
fn fetch_register(app: &mut App) -> Option<Register> {
    match app.registers.selected() {
        Some('%') => {
            app.registers.clear_selection();
            app.buffer()
                .document
                .path()
                .map(|path| Register::new(path.display().to_string(), RegisterKind::Charwise))
        }
        Some(name @ (':' | '/' | '.')) => {
            app.registers.clear_selection();
            app.registers.named(name).cloned()
        }
        _ => app.registers.get(),
    }
}

/// `p` and `P`.
pub fn put(app: &mut App, before: bool, count: usize) {
    let Some(register) = fetch_register(app) else {
        app.info("nothing to put");
        return;
    };
    put_register(app, &register, before, count);
}

fn put_register(app: &mut App, register: &Register, before: bool, count: usize) {
    let count = count.max(1);
    match register.kind {
        RegisterKind::Charwise => put_chars(app, &register.text.repeat(count), before),
        RegisterKind::Linewise => put_lines(app, &register.text, before, count),
        RegisterKind::Blockwise => put_block(app, &register.text, before, count),
    }
}

fn put_chars(app: &mut App, text: &str, before: bool) {
    let document = &app.buffer().document;
    let mut replacements: Vec<(Range, String)> = app
        .window()
        .cursors()
        .iter()
        .map(|cursor| {
            let at = document.pos_to_char(cursor.head)
                + usize::from(!before && document.line_len(cursor.head.line) > 0);
            (Range::new(at, at), text.to_string())
        })
        .collect();
    replacements.sort_by_key(|(range, _)| range.start);
    let starts = edit_group(app, &replacements);
    let length = text.chars().count();
    let multiline = text.contains('\n');
    let document = &app.buffer().document;
    let positions = starts
        .into_iter()
        .map(|start| {
            if multiline {
                document.char_to_pos(start)
            } else {
                document.char_to_pos(start + length.saturating_sub(1))
            }
        })
        .collect();
    place(app, positions);
    app.clamp_cursors(false);
}

fn put_lines(app: &mut App, text: &str, before: bool, count: usize) {
    let mut body = text.to_string();
    if !body.ends_with('\n') {
        body.push('\n');
    }
    let body = body.repeat(count);
    let document = &app.buffer().document;
    let mut lines: Vec<usize> = app.window().cursors().iter().map(|c| c.head.line).collect();
    lines.dedup();
    let mut replacements: Vec<(Range, String)> = lines
        .into_iter()
        .map(|line| {
            if before {
                let at = document.line_start(line);
                (Range::new(at, at), body.clone())
            } else if line < document.last_line() {
                let at = document.line_start(line + 1);
                (Range::new(at, at), body.clone())
            } else {
                let at = document.len_chars();
                (
                    Range::new(at, at),
                    format!("\n{}", body.trim_end_matches('\n')),
                )
            }
        })
        .collect();
    replacements.sort_by_key(|(range, _)| range.start);
    let starts = edit_group(app, &replacements);
    let document = &app.buffer().document;
    let positions = starts
        .into_iter()
        .zip(&replacements)
        .map(|(start, (_, inserted))| {
            let start = start + usize::from(inserted.starts_with('\n'));
            let line = document.char_to_pos(start).line;
            Position::new(line, indent::first_non_blank(document, line))
        })
        .collect();
    place(app, positions);
    app.clamp_cursors(false);
}

fn put_block(app: &mut App, text: &str, before: bool, count: usize) {
    let head = app.window().cursor().head;
    let document = &app.buffer().document;
    let column = head.col + usize::from(!before && document.line_len(head.line) > 0);
    let rows: Vec<&str> = text.split('\n').collect();
    let width = rows
        .iter()
        .map(|row| row.chars().count())
        .max()
        .unwrap_or(0);

    let mut replacements = Vec::new();
    let mut appended = String::new();
    for (offset, row) in rows.iter().enumerate() {
        // Every row but the last is padded to the block's width, so the text to
        // its right stays in its column.
        let padded = format!("{row:<width$}").repeat(count);
        let line = head.line + offset;
        if line > document.last_line() {
            appended.push('\n');
            appended.push_str(&" ".repeat(column));
            appended.push_str(padded.trim_end());
            continue;
        }
        let len = document.line_len(line);
        let start = document.line_start(line);
        let piece = if column > len {
            format!("{}{}", " ".repeat(column - len), padded.trim_end())
        } else if column == len {
            padded.trim_end().to_string()
        } else {
            padded
        };
        let at = start + column.min(len);
        replacements.push((Range::new(at, at), piece));
    }
    if !appended.is_empty() {
        let end = document.len_chars();
        replacements.push((Range::new(end, end), appended));
    }
    edit_group(app, &replacements);
    let position = app
        .buffer()
        .document
        .clamp(Position::new(head.line, column), false);
    place(app, vec![position]);
}

/// Paste over a selection, or at the caret: Ctrl+V, and `p` in visual mode.
pub fn paste(app: &mut App) {
    let Some(register) = fetch_register(app) else {
        app.info("clipboard is empty");
        return;
    };
    if app.mode.is_visual() {
        let linewise_selection = app.mode == Mode::VisualLine;
        operate_selection(app, Operator::Delete, 1);
        // Lines pasted over a character selection, or text over lines, still
        // land where the selection was.
        let before = !(linewise_selection
            && register.kind == RegisterKind::Linewise
            && app.window().cursor().head.line == app.buffer().document.last_line()
            && app
                .buffer()
                .document
                .line_len(app.window().cursor().head.line)
                == 0);
        if linewise_selection && register.kind != RegisterKind::Linewise {
            let mut lines = register.clone();
            lines.kind = RegisterKind::Linewise;
            put_register(app, &lines, true, 1);
        } else {
            put_register(app, &register, before, 1);
        }
        return;
    }
    // The modeless convention: paste at the caret, replacing any selection.
    let mut edit = app.edit();
    edit.delete_selections();
    edit.paste(&register.text, register.kind == RegisterKind::Linewise);
}

/// `r` — replace `count` characters under the caret.
pub fn replace_char(app: &mut App, ch: char, count: usize) {
    let count = count.max(1);
    let document = &app.buffer().document;
    let mut replacements: Vec<(Range, String)> = app
        .window()
        .cursors()
        .iter()
        .filter(|cursor| cursor.head.col + count <= document.line_len(cursor.head.line))
        .map(|cursor| {
            let at = document.pos_to_char(cursor.head);
            (Range::new(at, at + count), ch.to_string().repeat(count))
        })
        .collect();
    if replacements.is_empty() {
        return;
    }
    replacements.sort_by_key(|(range, _)| range.start);
    let starts = edit_group(app, &replacements);
    let document = &app.buffer().document;
    let positions = starts
        .into_iter()
        .map(|start| document.char_to_pos(start + count - 1))
        .collect();
    place(app, positions);
}

/// `r` in visual mode — every selected character becomes `ch`.
pub fn replace_selection(app: &mut App, ch: char) {
    let mut spans = selection_spans(app);
    spans.sort_by_key(|span| span.range.start);
    let document = &app.buffer().document;
    let replacements: Vec<(Range, String)> = spans
        .iter()
        .map(|span| {
            let text = document.slice_string(span.range.start, span.range.end);
            let replaced: String = text
                .chars()
                .map(|current| if current == '\n' { '\n' } else { ch })
                .collect();
            (span.range, replaced)
        })
        .collect();
    let start = spans.first().map(|span| span.range.start);
    enter_mode(app, Mode::Normal);
    edit_group(app, &replacements);
    if let Some(start) = start {
        let position = app.buffer().document.char_to_pos(start);
        place(app, vec![position]);
    }
}

/// `~` — flip the case of `count` characters and step past them.
pub fn toggle_case_chars(app: &mut App, count: usize) {
    let document = &app.buffer().document;
    let mut replacements: Vec<(Range, String)> = app
        .window()
        .cursors()
        .iter()
        .filter(|cursor| document.line_len(cursor.head.line) > 0)
        .map(|cursor| {
            let at = document.pos_to_char(cursor.head);
            let end = (cursor.head.col + count.max(1)).min(document.line_len(cursor.head.line));
            let end = document.line_start(cursor.head.line) + end;
            let text = document.slice_string(at, end);
            (Range::new(at, end), CaseChange::Toggle.apply(&text))
        })
        .collect();
    if replacements.is_empty() {
        return;
    }
    replacements.sort_by_key(|(range, _)| range.start);
    let ends: Vec<usize> = replacements.iter().map(|(range, _)| range.end).collect();
    edit_group(app, &replacements);
    let document = &app.buffer().document;
    let positions = ends
        .into_iter()
        .map(|end| document.clamp(document.char_to_pos(end), false))
        .collect();
    place(app, positions);
}

/// `J` and `gJ` — join `count` lines (at least two) into one.
pub fn join(app: &mut App, count: usize, spaces: bool) {
    let mut lines: Vec<usize> = app.window().cursors().iter().map(|c| c.head.line).collect();
    lines.dedup();
    join_lines(app, &lines, count.max(2) - 1, spaces);
}

/// `J` in visual mode — join every selected line.
pub fn join_selection(app: &mut App, spaces: bool) {
    let cursor = app.window().cursor();
    let first = cursor.anchor.line.min(cursor.head.line);
    let last = cursor.anchor.line.max(cursor.head.line);
    enter_mode(app, Mode::Normal);
    join_lines(app, &[first], (last - first).max(1), spaces);
}

fn join_lines(app: &mut App, lines: &[usize], joins: usize, spaces: bool) {
    let mut positions = Vec::new();
    let mut edit = app.edit();
    edit.begin_group();
    for &line in lines.iter().rev() {
        let mut caret = None;
        for _ in 0..joins {
            let document = &edit.buffer.document;
            if line >= document.last_line() {
                break;
            }
            let current = document.line_string(line);
            let next = document.line_string(line + 1);
            let end = document.line_start(line) + document.line_len(line);
            let leading = if spaces {
                next.chars().take_while(|ch| ch.is_whitespace()).count()
            } else {
                0
            };
            let next_start = document.line_start(line + 1);
            let rest = next.trim_start();
            let separator = if !spaces
                || rest.is_empty()
                || current.is_empty()
                || current.ends_with([' ', '\t'])
                || rest.starts_with(')')
            {
                ""
            } else {
                " "
            };
            edit.replace(end, next_start + leading, separator);
            caret = Some(end);
        }
        if let Some(caret) = caret {
            positions.push(caret);
        }
    }
    edit.end_group();
    let document = &app.buffer().document;
    let mut positions: Vec<Position> = positions
        .into_iter()
        .map(|at| document.char_to_pos(at))
        .collect();
    positions.reverse();
    place(app, positions);
    app.clamp_cursors(false);
}

/// `iw`, `a(` and the rest in visual mode: select the object.
pub fn select_object(app: &mut App, object: TextObject, inner: bool, count: usize) {
    let head = app.window().cursor().head;
    let document = &app.buffer().document;
    let Some(found) = textobject::select(document, head, object, inner, count) else {
        return;
    };
    if found.range.is_empty() {
        return;
    }
    let start = document.char_to_pos(found.range.start);
    let end = document.char_to_pos(found.range.end - 1);
    if found.linewise && app.mode == Mode::Visual {
        app.mode = Mode::VisualLine;
    }
    app.window_mut().clear_secondary_cursors();
    let cursor = app.window_mut().cursor_mut();
    cursor.move_to(start, false);
    cursor.move_to(end, true);
}

/// `I` and `A` in visual mode: insert at the start or end of the selection,
/// on every line of a block.
pub fn block_insert(app: &mut App, append: bool) {
    let cursor = app.window().cursor();
    let document = &app.buffer().document;
    let positions: Vec<Position> = if app.mode == Mode::VisualBlock {
        let first = cursor.anchor.line.min(cursor.head.line);
        let last = cursor.anchor.line.max(cursor.head.line);
        let column = if append {
            cursor.anchor.col.max(cursor.head.col) + 1
        } else {
            cursor.anchor.col.min(cursor.head.col)
        };
        (first..=last)
            .map(|line| document.clamp(Position::new(line, column), true))
            .collect()
    } else {
        let range = match app.mode {
            Mode::VisualLine => Range::of_lines(&cursor, document),
            _ => Range::of(&cursor, document),
        };
        let at = if append { range.end } else { range.start };
        vec![document.char_to_pos(at)]
    };
    enter_mode(app, Mode::Insert);
    place(app, positions);
}

/// The goal of a word motion from the caret, for `*`.
#[must_use]
pub fn word_under_caret(app: &App) -> Option<String> {
    let document = &app.buffer().document;
    let head = app.window().cursor().head;
    let line_start = document.line_start(head.line);
    let line_end = line_start + document.line_len(head.line);
    let mut index = document.pos_to_char(head);
    while index < line_end && !word::is_word_at(document, index) {
        index += 1;
    }
    if index >= line_end {
        return None;
    }
    let (start, end) = word::run_at(document, index, false)?;
    Some(document.slice_string(start, end))
}
