//! # Highlight engine
//!
//! **Purpose:** scan one line and return its spans.
//!
//! **Responsibility:** build a single alternation regex per language and walk a
//! line with it. One combined pattern rather than one regex per token type means
//! the engine makes a single pass and gets precedence for free — the regex
//! crate's leftmost-first alternation is exactly the "a `//` inside a string is
//! not a comment" rule.
//!
//! Block comments are the one thing a per-line regex cannot express, so they are
//! handled around the regex with an explicit [`BlockState`] carried from line to
//! line.
//!
//! **Public API:** [`Highlighter`], [`BlockState`].

use regex::Regex;

use super::{Highlight, HighlightKind, Language};

/// What a line inherits from the lines above it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BlockState {
    /// Ordinary code.
    #[default]
    Normal,
    /// Inside a block comment, `depth` levels deep. Depth is only ever greater
    /// than one for languages with nested block comments.
    InBlockComment(u16),
}

/// A compiled language.
pub struct Highlighter {
    /// The language this highlighter was built from.
    pub language: &'static Language,
    /// The combined alternation; capture group `n + 1` corresponds to `kinds[n]`.
    regex: Regex,
    /// Kind produced by each capture group.
    kinds: Vec<Rule>,
}

/// What a capture group means.
#[derive(Debug, Clone, Copy)]
enum Rule {
    /// A fixed token class.
    Fixed(HighlightKind),
    /// An identifier, classified against the language's word lists.
    Identifier,
    /// An identifier followed by `(`.
    ///
    /// The `regex` crate has no lookahead, so the pattern has to consume the
    /// bracket; this rule highlights only the name and rewinds to just after it
    /// so the bracket is still scanned as punctuation.
    Call,
    /// The opening token of a block comment.
    BlockStart,
}

impl Highlighter {
    /// Compile the alternation for `language`.
    ///
    /// # Panics
    /// Panics if a built-in language definition contains an invalid pattern,
    /// which is a programming error rather than a runtime condition.
    #[must_use]
    pub fn new(language: &&'static Language) -> Self {
        let language: &'static Language = language;
        let mut alternatives = Vec::new();
        let mut kinds = Vec::new();

        // Order only decides ties: the regex crate takes the leftmost match, and
        // only picks between alternatives that start at the same offset.
        //
        // The block-comment opener has to come first because it can be a prefix
        // of another rule — Python's `"""` also matches the string rule, and the
        // docstring reading is the right one.
        if let Some((open, _)) = language.block_comment {
            alternatives.push(regex::escape(open));
            kinds.push(Rule::BlockStart);
        }
        // Language-specific rules then win over everything generic.
        for (pattern, kind) in language.extra_rules {
            alternatives.push((*pattern).to_string());
            kinds.push(Rule::Fixed(*kind));
        }
        if let Some(prefix) = language.line_comment {
            alternatives.push(format!("{}.*", regex::escape(prefix)));
            kinds.push(Rule::Fixed(HighlightKind::Comment));
        }
        if !language.prose {
            // An unterminated string still highlights to end of line, which is
            // what the user sees while typing one.
            alternatives.push(r#""(?:[^"\\]|\\.)*"?"#.to_string());
            kinds.push(Rule::Fixed(HighlightKind::String));
            alternatives.push(r"'(?:[^'\\]|\\.)'".to_string());
            kinds.push(Rule::Fixed(HighlightKind::String));
            alternatives.push(NUMBER.to_string());
            kinds.push(Rule::Fixed(HighlightKind::Number));

            if language.macro_suffix {
                alternatives.push(r"\b[A-Za-z_]\w*!".to_string());
                kinds.push(Rule::Fixed(HighlightKind::Macro));
            }
            alternatives.push(r"\b[A-Za-z_]\w*\s*\(".to_string());
            kinds.push(Rule::Call);
            alternatives.push(r"\b[A-Za-z_]\w*".to_string());
            kinds.push(Rule::Identifier);
            alternatives.push(r"[-+*/%=<>!&|^~?:@]+".to_string());
            kinds.push(Rule::Fixed(HighlightKind::Operator));
            alternatives.push(r"[{}()\[\];,.]".to_string());
            kinds.push(Rule::Fixed(HighlightKind::Punctuation));
        }

        let source = alternatives
            .iter()
            .map(|alternative| format!("({alternative})"))
            .collect::<Vec<_>>()
            .join("|");

        Self {
            language,
            regex: Regex::new(&source).expect("built-in language patterns are valid"),
            kinds,
        }
    }

    /// Highlight one line, given the state left behind by the previous one.
    ///
    /// Returns the spans in left-to-right order and the state the next line
    /// starts in.
    #[must_use]
    pub fn highlight_line(&self, line: &str, state: BlockState) -> (Vec<Highlight>, BlockState) {
        let mut spans: Vec<(usize, usize, HighlightKind)> = Vec::new();
        let mut state = state;
        let mut position = 0;

        // Finish any comment that ran over from the previous line before the
        // regex gets a chance to misread its contents.
        if let BlockState::InBlockComment(depth) = state {
            let (end, next) = self.scan_block_comment(line, 0, depth);
            spans.push((0, end, HighlightKind::Comment));
            state = next;
            position = end;
            if matches!(state, BlockState::InBlockComment(_)) {
                return (self.to_char_spans(line, spans), state);
            }
        }

        while position <= line.len() {
            let Some(captures) = self.regex.captures_at(line, position) else {
                break;
            };
            let matched = captures.get(0).expect("group 0 always participates");
            let rule = captures
                .iter()
                .enumerate()
                .skip(1)
                .find_map(|(index, group)| group.map(|_| self.kinds[index - 1]));

            match rule {
                Some(Rule::BlockStart) => {
                    let (end, next) = self.scan_block_comment(line, matched.end(), 1);
                    spans.push((matched.start(), end, HighlightKind::Comment));
                    state = next;
                    position = end;
                    if matches!(state, BlockState::InBlockComment(_)) {
                        break;
                    }
                }
                Some(Rule::Identifier) => {
                    if let Some(kind) = self.classify(matched.as_str()) {
                        spans.push((matched.start(), matched.end(), kind));
                    }
                    position = matched.end();
                }
                Some(Rule::Call) => {
                    let name = matched.as_str().trim_end_matches('(').trim_end();
                    let end = matched.start() + name.len();
                    // A keyword before a bracket — `if (`, `while (` — is still
                    // a keyword, not a call.
                    let kind = self.classify(name).unwrap_or(HighlightKind::Function);
                    spans.push((matched.start(), end, kind));
                    position = end;
                }
                Some(Rule::Fixed(kind)) => {
                    spans.push((matched.start(), matched.end(), kind));
                    position = matched.end();
                }
                None => break,
            }
            // A zero-width match would loop forever.
            if matched.start() == matched.end() {
                position = matched.end() + 1;
            }
        }

        (self.to_char_spans(line, spans), state)
    }

    /// Classify an identifier against the language's word lists.
    ///
    /// Returns `None` for plain identifiers, which are left unstyled so the
    /// bulk of the text keeps the theme's default colour.
    fn classify(&self, word: &str) -> Option<HighlightKind> {
        if self.language.keywords.contains(&word) {
            return Some(HighlightKind::Keyword);
        }
        if self.language.types.contains(&word) {
            return Some(HighlightKind::Type);
        }
        if self.language.constants.contains(&word) {
            return Some(HighlightKind::Constant);
        }
        // `SCREAMING_CASE` is a constant by convention in every language here,
        // but a single capital letter is not enough to call it one.
        if word.len() > 1
            && word
                .chars()
                .all(|ch| ch.is_ascii_uppercase() || ch == '_' || ch.is_ascii_digit())
            && word.chars().any(|ch| ch.is_ascii_uppercase())
        {
            return Some(HighlightKind::Constant);
        }
        if self.language.capitalised_types && word.starts_with(|ch: char| ch.is_uppercase()) {
            return Some(HighlightKind::Type);
        }
        None
    }

    /// Walk a block comment from `from`, returning where it ends and the state
    /// the next line inherits.
    fn scan_block_comment(&self, line: &str, from: usize, depth: u16) -> (usize, BlockState) {
        let Some((open, close)) = self.language.block_comment else {
            return (line.len(), BlockState::Normal);
        };
        let mut depth = depth;
        let mut position = from;

        while position < line.len() {
            let rest = &line[position..];
            let next_close = rest.find(close);
            let next_open = self
                .language
                .nested_block_comments
                .then(|| rest.find(open))
                .flatten();

            match (next_open, next_close) {
                // A nested opener before the next closer deepens the comment.
                (Some(open_at), Some(close_at)) if open_at < close_at => {
                    depth += 1;
                    position += open_at + open.len();
                }
                (Some(open_at), None) => {
                    depth += 1;
                    position += open_at + open.len();
                }
                (_, Some(close_at)) => {
                    position += close_at + close.len();
                    depth -= 1;
                    if depth == 0 {
                        return (position, BlockState::Normal);
                    }
                }
                (None, None) => break,
            }
        }
        (line.len(), BlockState::InBlockComment(depth))
    }

    /// Convert byte spans to character spans.
    ///
    /// ASCII lines — the overwhelming majority of source code — skip the
    /// conversion entirely, so the common case costs nothing.
    fn to_char_spans(
        &self,
        line: &str,
        spans: Vec<(usize, usize, HighlightKind)>,
    ) -> Vec<Highlight> {
        if line.is_ascii() {
            return spans
                .into_iter()
                .map(|(start, end, kind)| Highlight { start, end, kind })
                .collect();
        }
        // One pass over the line builds every boundary we need.
        let mut byte_to_char = vec![0usize; line.len() + 1];
        for (char_index, (byte_index, _)) in line.char_indices().enumerate() {
            byte_to_char[byte_index] = char_index;
        }
        // Fill continuation bytes so an interior offset still maps sensibly.
        let mut last = 0;
        for entry in &mut byte_to_char {
            if *entry == 0 && last != 0 {
                *entry = last;
            } else {
                last = *entry;
            }
        }
        byte_to_char[line.len()] = line.chars().count();

        spans
            .into_iter()
            .map(|(start, end, kind)| Highlight {
                start: byte_to_char[start],
                end: byte_to_char[end],
                kind,
            })
            .collect()
    }
}

/// Integer and float literals, including the usual radix prefixes and suffixes.
const NUMBER: &str = r"\b(?:0[xX][0-9a-fA-F_]+|0[bB][01_]+|0[oO][0-7_]+|\d[\d_]*(?:\.\d[\d_]*)?(?:[eE][+-]?\d+)?)[A-Za-z_0-9]*";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::syntax::by_name;

    /// Render a line as a string of one letter per character, so an expected
    /// highlighting can be written down and compared at a glance.
    fn sketch(language: &str, line: &str) -> String {
        let highlighter = by_name(language).expect("known language");
        let (spans, _) = highlighter.highlight_line(line, BlockState::Normal);
        paint(line, &spans)
    }

    fn paint(line: &str, spans: &[Highlight]) -> String {
        let mut out = vec![b'.'; line.chars().count()];
        for span in spans {
            let letter = match span.kind {
                HighlightKind::Keyword => b'k',
                HighlightKind::Type => b't',
                HighlightKind::Function => b'f',
                HighlightKind::String => b's',
                HighlightKind::Number => b'n',
                HighlightKind::Comment => b'c',
                HighlightKind::Constant => b'C',
                HighlightKind::Operator => b'o',
                HighlightKind::Punctuation => b'p',
                HighlightKind::Attribute => b'a',
                HighlightKind::Macro => b'm',
                HighlightKind::Heading => b'H',
                HighlightKind::Emphasis => b'E',
                HighlightKind::Link => b'L',
                HighlightKind::DiffAdded => b'+',
                HighlightKind::DiffRemoved => b'-',
                HighlightKind::DiffHunk => b'@',
                HighlightKind::DiffMeta => b'M',
            };
            for cell in out.iter_mut().take(span.end).skip(span.start) {
                *cell = letter;
            }
        }
        String::from_utf8(out).expect("ascii sketch")
    }

    #[test]
    fn rust_keywords_types_and_literals() {
        assert_eq!(sketch("rust", "let x: u32 = 1;"), "kkk..o.ttt.o.np");
    }

    #[test]
    fn a_comment_marker_inside_a_string_is_not_a_comment() {
        assert_eq!(sketch("rust", r#"let s = "// not";"#), "kkk...o.ssssssssp");
    }

    #[test]
    fn a_string_marker_inside_a_comment_is_not_a_string() {
        assert_eq!(sketch("rust", r#"// a "quote""#), "cccccccccccc");
    }

    #[test]
    fn rust_lifetimes_are_not_character_literals() {
        // `'a` is a lifetime (type-coloured), `char` a type, `(`/`)`/`{`/`}`
        // punctuation.
        assert_eq!(
            sketch("rust", "fn f<'a>(c: char) {}"),
            "kk..ottop.o.ttttp.pp"
        );
    }

    #[test]
    fn a_character_literal_still_wins_over_a_lifetime() {
        assert_eq!(sketch("rust", "let c = 'x';"), "kkk...o.sssp");
    }

    #[test]
    fn function_calls_are_highlighted_but_keywords_before_a_bracket_are_not() {
        assert_eq!(sketch("rust", "if foo() {}"), "kk.fffpp.pp");
    }

    #[test]
    fn block_comments_carry_over_to_the_next_line() {
        let highlighter = by_name("c").expect("known language");
        let (spans, state) = highlighter.highlight_line("int a; /* start", BlockState::Normal);
        assert_eq!(paint("int a; /* start", &spans), "ttt..p.cccccccc");
        assert_eq!(state, BlockState::InBlockComment(1));

        let (spans, state) = highlighter.highlight_line("still */ int b;", state);
        assert_eq!(paint("still */ int b;", &spans), "cccccccc.ttt..p");
        assert_eq!(state, BlockState::Normal);
    }

    #[test]
    fn rust_block_comments_nest() {
        let highlighter = by_name("rust").expect("known language");
        let (_, state) = highlighter.highlight_line("/* a /* b", BlockState::Normal);
        assert_eq!(state, BlockState::InBlockComment(2));
        let (_, state) = highlighter.highlight_line("*/", state);
        assert_eq!(state, BlockState::InBlockComment(1));
        let (_, state) = highlighter.highlight_line("*/", state);
        assert_eq!(state, BlockState::Normal);
    }

    #[test]
    fn c_preprocessor_includes_swallow_their_angle_brackets() {
        assert_eq!(sketch("c", "#include <stdio.h>"), "aaaaaaaaaaaaaaaaaa");
    }

    #[test]
    fn cpp_template_arguments_are_not_mistaken_for_an_include() {
        assert_eq!(sketch("cpp", "vector<int> v;"), "ttttttottto..p");
    }

    #[test]
    fn zig_builtins_are_macros() {
        assert_eq!(
            sketch("zig", "const s = @import(\"std\");"),
            "kkkkk...o.mmmmmmmpssssspp"
        );
    }

    #[test]
    fn python_decorators_and_docstrings() {
        assert_eq!(sketch("python", "@cache"), "aaaaaa");
        let highlighter = by_name("python").expect("known language");
        let (_, state) = highlighter.highlight_line(r#"""" doc"#, BlockState::Normal);
        assert_eq!(state, BlockState::InBlockComment(1));
    }

    #[test]
    fn markdown_headings_and_links() {
        assert_eq!(sketch("markdown", "# Title"), "HHHHHHH");
        assert_eq!(sketch("markdown", "see [x](y)"), "....LLLLLL");
        assert_eq!(sketch("markdown", "a **bold** b"), "..EEEEEEEE..");
    }

    #[test]
    fn diff_marks_whole_added_and_removed_lines() {
        assert_eq!(sketch("diff", "+let x = 1;"), "+++++++++++");
        assert_eq!(sketch("diff", "-let x = 1;"), "-----------");
        // A context line keeps the default colour.
        assert_eq!(sketch("diff", " let x = 1;"), "...........");
    }

    #[test]
    fn diff_file_headers_are_not_added_or_removed_lines() {
        assert_eq!(sketch("diff", "--- a/main.rs"), "MMMMMMMMMMMMM");
        assert_eq!(sketch("diff", "+++ b/main.rs"), "MMMMMMMMMMMMM");
        assert_eq!(sketch("diff", "-- still removed"), "----------------");
    }

    #[test]
    fn diff_hunk_headers_and_metadata() {
        assert_eq!(
            sketch("diff", "@@ -1,4 +1,6 @@ fn f"),
            "@@@@@@@@@@@@@@@@@@@@"
        );
        assert_eq!(sketch("diff", "diff --git a/x b/x"), "MMMMMMMMMMMMMMMMMM");
        assert_eq!(
            sketch("diff", "index 83db48f..bf269f4 100644"),
            "MMMMMMMMMMMMMMMMMMMMMMMMMMMMM"
        );
    }

    #[test]
    fn diff_reads_the_older_output_format_too() {
        assert_eq!(sketch("diff", "3,4c3,4"), "@@@@@@@");
        assert_eq!(sketch("diff", "< was"), "-----");
        assert_eq!(sketch("diff", "> now"), "+++++");
    }

    #[test]
    fn non_ascii_lines_report_character_offsets() {
        let highlighter = by_name("rust").expect("known language");
        let (spans, _) = highlighter.highlight_line("// ağaç", BlockState::Normal);
        assert_eq!(spans[0].start, 0);
        assert_eq!(spans[0].end, 7);
    }
}
