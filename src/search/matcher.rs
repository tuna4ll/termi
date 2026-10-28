//! # Pattern matching
//!
//! **Purpose:** compile a search pattern once and run it over lines.
//!
//! **Responsibility:** hide the difference between a literal search and a regex
//! search behind one type. Literal patterns are escaped and handed to the same
//! engine, so there is exactly one matching code path to reason about.
//!
//! Matching is per line rather than over the whole document. That keeps a search
//! on a large file proportional to what is actually looked at instead of to the
//! file size, at the cost of not supporting patterns that span a newline — a
//! trade every incremental search in a terminal editor makes.
//!
//! **Public API:** [`Matcher`], [`LineMatch`].

use regex::{Regex, RegexBuilder};

/// A match inside one line, in character offsets.
///
/// Character offsets rather than the byte offsets `regex` produces, because
/// every other index in the editor is a character index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineMatch {
    /// First character of the match.
    pub start: usize,
    /// One past the last character.
    pub end: usize,
}

/// A compiled search pattern.
#[derive(Debug, Clone)]
pub struct Matcher {
    regex: Regex,
}

impl Matcher {
    /// Compile `pattern`.
    ///
    /// `smart case` applies when `case_sensitive` is `None`: the search is
    /// case-insensitive until the user types a capital letter, which is what
    /// makes a quick lowercase search find everything without a flag.
    ///
    /// # Errors
    /// Returns a displayable message when a regex pattern does not compile.
    pub fn new(pattern: &str, regex: bool, case_sensitive: Option<bool>) -> Result<Self, String> {
        let source = if regex {
            pattern.to_string()
        } else {
            regex::escape(pattern)
        };
        let sensitive = case_sensitive.unwrap_or_else(|| pattern.chars().any(char::is_uppercase));

        RegexBuilder::new(&source)
            .case_insensitive(!sensitive)
            // A pattern cannot cross a line boundary here, so `.` matching a
            // newline would only ever be surprising.
            .dot_matches_new_line(false)
            .size_limit(1 << 20)
            .build()
            .map(|regex| Self { regex })
            .map_err(|error| {
                error
                    .to_string()
                    .lines()
                    .next()
                    .unwrap_or("bad pattern")
                    .to_string()
            })
    }

    /// Every match in `line`, left to right.
    #[must_use]
    pub fn find_all(&self, line: &str) -> Vec<LineMatch> {
        self.regex
            .find_iter(line)
            .filter(|found| !found.is_empty())
            .map(|found| LineMatch {
                start: line[..found.start()].chars().count(),
                end: line[..found.end()].chars().count(),
            })
            .collect()
    }

    /// The first match in `line` starting at or after character `from`, with
    /// `replacement` expanded against it: its character span and the text to
    /// put there. What `:s///c` asks about one at a time.
    #[must_use]
    pub fn next_match(
        &self,
        line: &str,
        from: usize,
        replacement: &str,
    ) -> Option<(usize, usize, String)> {
        let byte = line
            .char_indices()
            .nth(from)
            .map_or(line.len(), |(index, _)| index);
        let mut at = byte;
        while at <= line.len() {
            let captures = self.regex.captures_at(line, at)?;
            let found = captures.get(0)?;
            if found.is_empty() {
                // An empty match replaces nothing a user could see; step past.
                at = found.end() + line[found.end()..].chars().next().map_or(1, char::len_utf8);
                continue;
            }
            let mut expanded = String::new();
            captures.expand(replacement, &mut expanded);
            return Some((
                line[..found.start()].chars().count(),
                line[..found.end()].chars().count(),
                expanded,
            ));
        }
        None
    }

    /// Replace matches in `line`, returning the new line and how many were
    /// replaced.
    ///
    /// `$1` and `${name}` in `replacement` refer to capture groups, which is why
    /// replacement goes through the regex engine rather than a plain splice.
    #[must_use]
    pub fn replace(&self, line: &str, replacement: &str, all: bool) -> (String, usize) {
        let count = if all {
            self.regex.find_iter(line).count()
        } else {
            usize::from(self.regex.is_match(line))
        };
        let replaced = if all {
            self.regex.replace_all(line, replacement)
        } else {
            self.regex.replace(line, replacement)
        };
        (replaced.into_owned(), count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn matcher(pattern: &str, regex: bool) -> Matcher {
        Matcher::new(pattern, regex, None).expect("valid pattern")
    }

    #[test]
    fn a_literal_pattern_is_not_treated_as_a_regex() {
        let found = matcher("a.c", false).find_all("abc a.c");
        assert_eq!(found, vec![LineMatch { start: 4, end: 7 }]);
    }

    #[test]
    fn a_regex_pattern_matches_as_a_regex() {
        let found = matcher(r"\d+", true).find_all("x12y345");
        assert_eq!(
            found,
            vec![
                LineMatch { start: 1, end: 3 },
                LineMatch { start: 4, end: 7 }
            ]
        );
    }

    #[test]
    fn smart_case_ignores_case_until_a_capital_is_typed() {
        assert_eq!(matcher("foo", false).find_all("FOO foo").len(), 2);
        assert_eq!(matcher("Foo", false).find_all("FOO foo").len(), 0);
    }

    #[test]
    fn an_explicit_case_setting_overrides_smart_case() {
        let sensitive = Matcher::new("foo", false, Some(true)).expect("valid pattern");
        assert_eq!(sensitive.find_all("FOO foo").len(), 1);
    }

    #[test]
    fn offsets_are_characters_not_bytes() {
        let found = matcher("ç", false).find_all("aç ç");
        assert_eq!(found[0], LineMatch { start: 1, end: 2 });
        assert_eq!(found[1], LineMatch { start: 3, end: 4 });
    }

    #[test]
    fn empty_matches_are_skipped() {
        assert!(matcher("x*", true).find_all("abc").is_empty());
    }

    #[test]
    fn an_invalid_regex_reports_an_error() {
        assert!(Matcher::new("(unclosed", true, None).is_err());
    }

    #[test]
    fn replacement_expands_capture_groups() {
        let (line, count) = matcher(r"(\w+)=(\w+)", true).replace("a=1 b=2", "$2=$1", true);
        assert_eq!(line, "1=a 2=b");
        assert_eq!(count, 2);
    }

    #[test]
    fn replacing_without_the_global_flag_only_touches_the_first_match() {
        let (line, count) = matcher("a", false).replace("aaa", "b", false);
        assert_eq!(line, "baa");
        assert_eq!(count, 1);
    }
}
