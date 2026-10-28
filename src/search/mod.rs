//! # Search
//!
//! **Purpose:** find things, incrementally.
//!
//! **Responsibility:** hold the query being typed, compile it, and answer two
//! questions: "where is the next match from here?" and "what should be
//! highlighted on this line?". It never moves the cursor itself — the caller
//! decides what to do with a match.
//!
//! Matches are found on demand rather than indexed up front. Scanning a whole
//! document on every keystroke would make incremental search unusable on a large
//! file; scanning outward from the cursor until a match is found does the least
//! work that answers the question.
//!
//! **Public API:** [`Search`], [`matcher::Matcher`].

pub mod matcher;
pub mod project;

use crate::editor::document::Document;
use crate::editor::selection::Range;

pub use matcher::{LineMatch, Matcher};

/// Incremental search state for the editor.
#[derive(Debug, Default)]
pub struct Search {
    /// What the user has typed so far.
    pub query: String,
    /// Interpret the query as a regular expression.
    pub regex: bool,
    /// `None` means smart case.
    pub case_sensitive: Option<bool>,
    /// Direction the last search ran in, reused by "find next".
    pub forward: bool,
    /// Compiled query, or `None` when the query is empty or invalid.
    matcher: Option<Matcher>,
    /// Message from the last failed compile, shown while typing.
    error: Option<String>,
    /// Where the search started, so cancelling can go back.
    origin: usize,
    /// Treat this query as a regex whatever the setting says — `*` builds one.
    force_regex: bool,
}

impl Search {
    /// Begin a search from the caret at `origin`.
    pub fn begin(&mut self, origin: usize, forward: bool) {
        self.query.clear();
        self.matcher = None;
        self.error = None;
        self.force_regex = false;
        self.origin = origin;
        self.forward = forward;
    }

    /// Where the caret was when the search started.
    #[must_use]
    pub const fn origin(&self) -> usize {
        self.origin
    }

    /// Replace the query and recompile.
    pub fn set_query(&mut self, query: String) {
        self.query = query;
        self.compile();
    }

    /// Search for `word`, as `*` and `#` do; `whole` keeps it from matching
    /// inside a longer word.
    pub fn set_word(&mut self, word: &str, whole: bool, forward: bool) {
        self.forward = forward;
        self.force_regex = true;
        let escaped = regex::escape(word);
        self.query = if whole {
            format!(r"\b{escaped}\b")
        } else {
            escaped
        };
        self.compile();
    }

    /// Append a character to the query.
    pub fn push(&mut self, ch: char) {
        self.query.push(ch);
        self.compile();
    }

    /// Remove the last character of the query.
    pub fn pop(&mut self) {
        self.query.pop();
        self.compile();
    }

    /// The compile error for the current query, if any.
    #[must_use]
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// Whether there is a usable query.
    #[must_use]
    pub const fn is_active(&self) -> bool {
        self.matcher.is_some()
    }

    /// Matches on one line, for highlighting.
    ///
    /// Called once per visible line per frame, so it does no allocation beyond
    /// the result vector.
    #[must_use]
    pub fn matches_in_line(&self, line: &str) -> Vec<LineMatch> {
        self.matcher
            .as_ref()
            .map(|matcher| matcher.find_all(line))
            .unwrap_or_default()
    }

    /// The next match at or after `from`, wrapping around the end of the
    /// document.
    ///
    /// Returns `None` only when the document contains no match at all.
    #[must_use]
    pub fn find(&self, document: &Document, from: usize, forward: bool) -> Option<Range> {
        let matcher = self.matcher.as_ref()?;
        let lines = document.len_lines();
        let start_line = document.char_to_pos(from).line;

        // Walk every line exactly once, starting at the caret's, so a wrap-around
        // costs no more than a straight scan.
        for offset in 0..=lines {
            let line = if forward {
                (start_line + offset) % lines
            } else {
                (start_line + lines - offset % lines) % lines
            };
            let text = document.line_string(line);
            let line_start = document.line_start(line);
            let found = matcher.find_all(&text);

            let hit = if forward {
                found
                    .iter()
                    .find(|m| offset > 0 || line_start + m.start > from)
            } else {
                found
                    .iter()
                    .rev()
                    .find(|m| offset > 0 || line_start + m.start < from)
            };
            if let Some(hit) = hit {
                return Some(Range {
                    start: line_start + hit.start,
                    end: line_start + hit.end,
                });
            }
        }
        None
    }

    /// Number of matches in the whole document, capped so counting cannot stall
    /// the editor on a huge file.
    #[must_use]
    pub fn count(&self, document: &Document, cap: usize) -> usize {
        let Some(matcher) = self.matcher.as_ref() else {
            return 0;
        };
        let mut total = 0;
        for line in 0..document.len_lines() {
            total += matcher.find_all(&document.line_string(line)).len();
            if total >= cap {
                return cap;
            }
        }
        total
    }

    /// Recompile after the query changed.
    fn compile(&mut self) {
        if self.query.is_empty() {
            self.matcher = None;
            self.error = None;
            return;
        }
        match Matcher::new(
            &self.query,
            self.regex || self.force_regex,
            self.case_sensitive,
        ) {
            Ok(matcher) => {
                self.matcher = Some(matcher);
                self.error = None;
            }
            Err(message) => {
                // Keep the previous matcher out of the way: highlighting a stale
                // pattern while the user fixes a typo is worse than nothing.
                self.matcher = None;
                self.error = Some(message);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn search(query: &str) -> Search {
        let mut search = Search::default();
        search.set_query(query.to_string());
        search
    }

    fn doc(text: &str) -> Document {
        Document::from_text(text, None)
    }

    #[test]
    fn finds_the_next_match_after_the_caret() {
        let document = doc("foo bar\nfoo baz");
        let found = search("foo").find(&document, 0, true).expect("a match");
        assert_eq!(found, Range { start: 8, end: 11 });
    }

    #[test]
    fn searching_forward_wraps_around_the_end() {
        let document = doc("foo bar\nbaz");
        let found = search("foo").find(&document, 9, true).expect("a match");
        assert_eq!(found, Range { start: 0, end: 3 });
    }

    #[test]
    fn searching_backward_finds_the_previous_match() {
        let document = doc("foo\nbar\nfoo");
        let found = search("foo").find(&document, 8, false).expect("a match");
        assert_eq!(found, Range { start: 0, end: 3 });
    }

    #[test]
    fn a_missing_pattern_finds_nothing() {
        let document = doc("abc");
        assert!(search("zzz").find(&document, 0, true).is_none());
    }

    #[test]
    fn an_invalid_regex_reports_an_error_and_matches_nothing() {
        let mut search = Search {
            regex: true,
            ..Search::default()
        };
        search.set_query("(unclosed".to_string());
        assert!(search.error().is_some());
        assert!(!search.is_active());
    }

    #[test]
    fn counting_stops_at_the_cap() {
        let document = doc(&"x\n".repeat(100));
        assert_eq!(search("x").count(&document, 10), 10);
    }
}
