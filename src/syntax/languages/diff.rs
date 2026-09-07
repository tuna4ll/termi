//! Diff and patch definition.
//!
//! A diff is line oriented rather than token oriented: what a line means is
//! decided entirely by its first character, so every rule is anchored at the
//! start of the line and paints the whole of it. `prose` is on for the same
//! reason — a removed line is red because it was removed, not because of the
//! keywords inside it.
//!
//! Order matters more here than in any other language. `--- a/file` and
//! `+++ b/file` open a unified diff and have to be read as headers before the
//! generic `-` and `+` rules get to call them a removed and an added line.
//!
//! Both diff dialects are covered: unified and git output, where `-` and `+`
//! mark the lines, and the older `diff` default, where `<` and `>` do.

use crate::syntax::{HighlightKind, Language};

/// Diff and patch files.
pub static DIFF: Language = Language {
    name: "diff",
    extensions: &["diff", "patch", "rej"],
    filenames: &[],
    keywords: &[],
    types: &[],
    constants: &[],
    line_comment: None,
    block_comment: None,
    nested_block_comments: false,
    macro_suffix: false,
    capitalised_types: false,
    extra_rules: &[
        // Headers first, so the three that begin with `-` or `+` are not read as
        // removed or added lines.
        (r"^(?:---|\+\+\+)(?:\s.*)?$", HighlightKind::DiffMeta),
        (r"^diff .*", HighlightKind::DiffMeta),
        (
            r"^(?:index|old mode|new mode|new file mode|deleted file mode|similarity index|dissimilarity index|copy from|copy to|rename from|rename to|Binary files|GIT binary patch) .*",
            HighlightKind::DiffMeta,
        ),
        // The mail headers `git format-patch` writes above the diff itself.
        (
            r"^(?:From|Date|Subject)(?::| [0-9a-f]{7,}).*",
            HighlightKind::DiffMeta,
        ),
        // Two `@` for a unified hunk, three for a combined one from a merge.
        (r"^@{2,3} .*", HighlightKind::DiffHunk),
        (r"^\+.*", HighlightKind::DiffAdded),
        (r"^-.*", HighlightKind::DiffRemoved),
        // The older `diff` output: a `3,4c3,4` range, then `<` and `>` lines.
        (r"^\d+(?:,\d+)?[acd]\d+(?:,\d+)?$", HighlightKind::DiffHunk),
        (r"^>.*", HighlightKind::DiffAdded),
        (r"^<.*", HighlightKind::DiffRemoved),
        // `\ No newline at end of file`.
        (r"^\\ .*", HighlightKind::Comment),
    ],
    prose: true,
};
