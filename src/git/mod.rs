//! # Git
//!
//! **Purpose:** show what changed since the last commit, and where it came
//! from.
//!
//! **Responsibility:** run the `git` binary — there is no library to keep in
//! step with the user's repository format, and every developer has git
//! installed — and turn its answers into things the editor draws: a sign per
//! changed line, a status per file in the tree, a branch name, a blame line.
//! Every function returns `None` outside a repository or without git, so the
//! rest of the editor never has to ask.
//!
//! **Public API:** [`Sign`], [`signs`], [`head_text`], [`branch`],
//! [`statuses`], [`blame`], [`hunk_starts`].

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use similar::{DiffOp, TextDiff};

/// How a line differs from the last commit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sign {
    /// A line that is new.
    Added,
    /// A line that replaced another.
    Modified,
    /// One or more lines were removed just above this one.
    Removed,
}

/// Run git in `directory`, returning what it printed when it succeeded.
fn git(directory: &Path, arguments: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(directory)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

/// The directory a file is in, `.` for a bare file name.
fn parent(path: &Path) -> Option<&Path> {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .or(Some(Path::new(".")))
}

/// The top of the repository `path` is in.
#[must_use]
pub fn repository(path: &Path) -> Option<PathBuf> {
    let directory = if path.is_dir() { path } else { parent(path)? };
    git(directory, &["rev-parse", "--show-toplevel"]).map(|top| PathBuf::from(top.trim()))
}

/// The text `path` had in the last commit: `Some("")` for a file git does not
/// know yet, so every line of it shows as added, and `None` outside a
/// repository.
#[must_use]
pub fn head_text(path: &Path) -> Option<String> {
    let directory = parent(path)?;
    let name = path.file_name()?.to_str()?;
    repository(path)?;
    Some(git(directory, &["show", &format!("HEAD:./{name}")]).unwrap_or_default())
}

/// The branch checked out in the repository holding `path`, or the short
/// commit when the head is detached.
#[must_use]
pub fn branch(path: &Path) -> Option<String> {
    let directory = if path.is_dir() { path } else { parent(path)? };
    let name = git(directory, &["rev-parse", "--abbrev-ref", "HEAD"])?;
    let name = name.trim();
    if name == "HEAD" {
        return git(directory, &["rev-parse", "--short", "HEAD"]).map(|sha| sha.trim().to_string());
    }
    Some(name.to_string())
}

/// A sign for every line of `current` that differs from `base`, indexed by
/// line.
#[must_use]
pub fn signs(base: &str, current: &str) -> Vec<Option<Sign>> {
    let lines = current.split('\n').count();
    let mut signs = vec![None; lines];
    let mut mark = |from: usize, count: usize, sign: Sign| {
        for line in from..from + count {
            if let Some(slot) = signs.get_mut(line) {
                *slot = Some(sign);
            }
        }
    };
    let diff = TextDiff::from_lines(base, current);
    let mut removed_at = Vec::new();
    for op in diff.ops() {
        match *op {
            DiffOp::Equal { .. } => {}
            DiffOp::Insert {
                new_index, new_len, ..
            } => mark(new_index, new_len, Sign::Added),
            DiffOp::Replace {
                new_index, new_len, ..
            } => mark(new_index, new_len, Sign::Modified),
            DiffOp::Delete { new_index, .. } => removed_at.push(new_index),
        }
    }
    // A deletion has no line of its own; it is shown on the line that took
    // the removed lines' place, unless that line has a sign already.
    for line in removed_at {
        let line = line.min(lines.saturating_sub(1));
        if let Some(slot) = signs.get_mut(line)
            && slot.is_none()
        {
            *slot = Some(Sign::Removed);
        }
    }
    signs
}

/// The first line of every run of changed lines, for `]c` and `[c`.
#[must_use]
pub fn hunk_starts(signs: &[Option<Sign>]) -> Vec<usize> {
    signs
        .iter()
        .enumerate()
        .filter(|(line, sign)| sign.is_some() && (*line == 0 || signs[line - 1] != **sign))
        .map(|(line, _)| line)
        .collect()
}

/// The status letter of every changed file under `root`, keyed by full path,
/// with the directories holding them marked `•`.
#[must_use]
pub fn statuses(root: &Path) -> HashMap<PathBuf, char> {
    let mut found = HashMap::new();
    let Some(top) = repository(root) else {
        return found;
    };
    let Some(output) = git(
        &top,
        &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
    ) else {
        return found;
    };
    let mut entries = output.split('\0');
    while let Some(entry) = entries.next() {
        if entry.len() < 4 {
            continue;
        }
        let (code, name) = entry.split_at(3);
        let code: Vec<char> = code.chars().collect();
        // A rename is followed by the name it had before.
        if code[0] == 'R' || code[1] == 'R' {
            entries.next();
        }
        let letter = match (code[0], code[1]) {
            ('?', _) => '?',
            ('A', _) => 'A',
            (_, 'D') | ('D', _) => 'D',
            ('R', _) | (_, 'R') => 'R',
            _ => 'M',
        };
        let path = top.join(name);
        let mut directory = path.parent();
        while let Some(current) = directory {
            if !current.starts_with(&top) {
                break;
            }
            found.entry(current.to_path_buf()).or_insert('•');
            directory = current.parent();
        }
        found.insert(path, letter);
    }
    found
}

/// Who last changed line `line` (zero-based) of `path`, when and why, as one
/// line of text.
#[must_use]
pub fn blame(path: &Path, line: usize) -> Option<String> {
    let directory = parent(path)?;
    let name = path.file_name()?.to_str()?;
    let range = format!("{},{}", line + 1, line + 1);
    let output = git(
        directory,
        &["blame", "--porcelain", "-L", &range, "--", name],
    )?;
    let mut lines = output.lines();
    let sha = lines.next()?.split_whitespace().next()?.to_string();
    if sha.chars().all(|ch| ch == '0') {
        return Some("not committed yet".to_string());
    }
    let mut author = String::new();
    let mut time = 0u64;
    let mut summary = String::new();
    for line in lines {
        if let Some(name) = line.strip_prefix("author ") {
            author = name.to_string();
        } else if let Some(stamp) = line.strip_prefix("author-time ") {
            time = stamp.parse().unwrap_or(0);
        } else if let Some(text) = line.strip_prefix("summary ") {
            summary = text.to_string();
        }
    }
    let age = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|now| now.as_secs().saturating_sub(time))
        .unwrap_or(0);
    Some(format!(
        "{} {author}, {} ago — {summary}",
        &sha[..sha.len().min(8)],
        relative(age)
    ))
}

/// "3 days", "5 months": a span of seconds, roughly.
fn relative(seconds: u64) -> String {
    let (amount, unit) = match seconds {
        0..3600 => (seconds / 60, "minute"),
        3600..86_400 => (seconds / 3600, "hour"),
        86_400..2_592_000 => (seconds / 86_400, "day"),
        2_592_000..31_536_000 => (seconds / 2_592_000, "month"),
        _ => (seconds / 31_536_000, "year"),
    };
    format!("{amount} {unit}{}", if amount == 1 { "" } else { "s" })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signs_mark_added_changed_and_removed_lines() {
        let found = signs("one\ntwo\nthree\nfour\n", "one\nTWO\nthree\nnew\n");
        assert_eq!(found[0], None);
        assert_eq!(found[1], Some(Sign::Modified));
        assert_eq!(found[3], Some(Sign::Modified));

        let removed = signs("a\nb\nc\n", "a\nc\n");
        assert_eq!(removed[1], Some(Sign::Removed));
        let added = signs("a\n", "a\nb\n");
        assert_eq!(added[1], Some(Sign::Added));
    }

    #[test]
    fn hunks_start_where_a_run_of_signs_does() {
        let signs = vec![
            None,
            Some(Sign::Added),
            Some(Sign::Added),
            None,
            Some(Sign::Modified),
        ];
        assert_eq!(hunk_starts(&signs), vec![1, 4]);
    }

    #[test]
    fn a_repository_reports_signs_status_and_blame() {
        let root = std::env::temp_dir().join("termi-git-repo");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("create fixture");
        let run = |args: &[&str]| {
            Command::new("git")
                .args(args)
                .current_dir(&root)
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_SYSTEM", "/dev/null")
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_ok_and(|status| status.success())
        };
        if !run(&["init", "-q", "-b", "main"]) {
            // No git on this machine; there is nothing to check.
            return;
        }
        let file = root.join("notes.txt");
        std::fs::write(&file, "first\n").expect("write file");
        assert!(run(&["add", "notes.txt"]));
        assert!(run(&[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.com",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-q",
            "-m",
            "start",
        ]));
        std::fs::write(&file, "first\nsecond\n").expect("change file");
        std::fs::write(root.join("new.txt"), "x").expect("add file");

        assert_eq!(head_text(&file).as_deref(), Some("first\n"));
        assert_eq!(branch(&file).as_deref(), Some("main"));
        let status = statuses(&root);
        let top = repository(&root).expect("inside the repository");
        assert_eq!(status.get(&top.join("notes.txt")), Some(&'M'));
        assert_eq!(status.get(&top.join("new.txt")), Some(&'?'));
        let line = blame(&file, 0).expect("blame the first line");
        assert!(line.contains("Test") && line.contains("start"), "{line}");
        assert_eq!(blame(&file, 1).as_deref(), Some("not committed yet"));
    }
}
