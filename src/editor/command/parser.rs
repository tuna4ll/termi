//! # Command parsing
//!
//! **Purpose:** turn what the user typed after `:` into a [`Command`].
//!
//! **Responsibility:** splitting, `!` handling, and the substitute syntax.
//! Parsing returns a `Result` with a message meant to be shown verbatim in the
//! command bar, so error wording lives next to the syntax it describes.
//!
//! **Public API:** [`parse`].

use std::path::PathBuf;

use super::{Address, Base, Command, LineRange};
use crate::editor::window::Axis;

/// Parse a command line, without the leading `:`.
///
/// # Errors
/// Returns a message suitable for display when the command is unknown or its
/// arguments do not make sense.
pub fn parse(input: &str) -> Result<Command, String> {
    let input = input.trim();
    if input.is_empty() {
        return Err("empty command".to_string());
    }

    // A line range comes first: `:42` jumps to a line, `:%s` substitutes in
    // all of them, `:'<,'>s` in the last selection.
    let (range, input) = parse_range(input)?;
    if input.is_empty() {
        return match range {
            Some(LineRange {
                end:
                    Address {
                        base: Base::Line(line),
                        offset,
                    },
                ..
            }) => Ok(Command::GotoLine(
                line.saturating_sub(1).saturating_add_signed(offset),
            )),
            Some(range) => Ok(Command::GotoAddress(range.end)),
            None => Err("empty command".to_string()),
        };
    }
    if input.starts_with('s')
        && let Some(command) = parse_substitute(input, range)
    {
        return command;
    }
    if range.is_some() {
        return Err("only :s takes a range".to_string());
    }
    if let Some(rest) = input
        .strip_prefix("greplace")
        .or_else(|| input.strip_prefix("gs"))
        && rest
            .chars()
            .next()
            .is_some_and(|ch| !ch.is_alphanumeric() && !ch.is_whitespace())
    {
        return match parse_substitute(&format!("s{rest}"), None) {
            Some(Ok(Command::Substitute {
                pattern,
                replacement,
                case_sensitive,
                ..
            })) => Ok(Command::ProjectReplace {
                pattern,
                replacement,
                case_sensitive,
            }),
            _ => Err("usage: :greplace/pattern/replacement/".to_string()),
        };
    }

    let (head, rest) = match input.split_once(char::is_whitespace) {
        Some((head, rest)) => (head, rest.trim()),
        None => (input, ""),
    };
    let force = head.ends_with('!');
    let name = head.trim_end_matches('!');
    let argument = (!rest.is_empty()).then(|| rest.to_string());

    match name {
        "w" | "write" => Ok(Command::Write {
            path: argument.map(PathBuf::from),
            force,
        }),
        "sudow" | "sudowrite" => Ok(Command::SudoWrite),
        "recover" | "rec" => Ok(Command::Recover),
        "dropswap" => Ok(Command::DropSwap),
        "q" | "quit" => Ok(Command::Quit { force }),
        "wq" | "x" | "xit" => Ok(Command::WriteQuit {
            path: argument.map(PathBuf::from),
            force,
        }),
        "wa" | "wall" => Ok(Command::WriteAll),
        "qa" | "qall" | "quitall" => Ok(Command::QuitAll { force }),
        "wqa" | "wqall" | "xa" | "xall" => Ok(Command::WriteQuitAll),
        "e" | "edit" => match argument {
            Some(path) => Ok(Command::Edit {
                path: PathBuf::from(path),
                force,
            }),
            // `:e!` without a path is the idiomatic "throw away my changes and
            // reread the file".
            None if force => Ok(Command::Reload),
            None => Err("usage: :e <path>".to_string()),
        },
        "touch" => required_path(argument, "touch").map(Command::CreateFile),
        "mkdir" => required_path(argument, "mkdir").map(Command::CreateDirectory),
        "rename" => required_path(argument, "rename").map(Command::Rename),
        "copy" => required_path(argument, "copy").map(Command::Copy),
        "move" => required_path(argument, "move").map(Command::Move),
        "bn" | "bnext" => Ok(Command::CycleBuffer { forward: true }),
        "bp" | "bprev" => Ok(Command::CycleBuffer { forward: false }),
        // vi names these after the line the split runs along, not after the way
        // the windows end up stacked; the axis says which the arrangement is.
        "sp" | "split" | "new" => Ok(Command::Split {
            axis: Axis::Vertical,
        }),
        "vs" | "vsp" | "vsplit" | "vnew" => Ok(Command::Split {
            axis: Axis::Horizontal,
        }),
        "clo" | "close" => Ok(Command::CloseWindow),
        "on" | "only" => Ok(Command::OnlyWindow),
        "theme" => argument.map_or_else(
            || Err("usage: :theme <name>".to_string()),
            |name| Ok(Command::Theme(name)),
        ),
        "set" | "se" => parse_set(rest, force),
        "config" => match rest {
            "" | "edit" => Ok(Command::Config { reload: false }),
            "reload" => Ok(Command::Config { reload: true }),
            _ => Err("usage: :config [reload]".to_string()),
        },
        "earlier" | "ea" => count_argument(rest).map(Command::Earlier),
        "later" | "lat" => count_argument(rest).map(Command::Later),
        "help" | "h" if rest.is_empty() => Ok(Command::Help),
        "help" | "h" => Ok(Command::HelpTopic(rest.to_string())),
        "grep" | "rg" => Ok(Command::Grep(argument)),
        "blame" => Ok(Command::Blame),
        "find" | "files" => Ok(Command::Find),
        "buffers" | "ls" => Ok(Command::Buffers),
        "commands" => Ok(Command::Commands),
        "themes" => Ok(Command::Themes),
        other => Err(format!("unknown command: {other}")),
    }
}

/// An optional step count, one when left out.
fn count_argument(rest: &str) -> Result<usize, String> {
    if rest.is_empty() {
        return Ok(1);
    }
    rest.parse::<usize>()
        .map(|count| count.max(1))
        .map_err(|_| format!("not a count: {rest}"))
}

/// Read the required path shared by filesystem commands.
fn required_path(argument: Option<String>, command: &str) -> Result<PathBuf, String> {
    argument
        .map(PathBuf::from)
        .ok_or_else(|| format!("usage: :{command} <path>"))
}

/// `set key value`, where a missing value means "turn it on".
fn parse_set(rest: &str, persist: bool) -> Result<Command, String> {
    // `:set ts=2` and `:set ts 2` say the same thing.
    let rest = rest.replacen('=', " ", 1);
    let mut parts = rest.split_whitespace();
    let Some(key) = parts.next() else {
        return Err("usage: :set <option> [value]".to_string());
    };
    // `:set number` reads better than `:set number true`, and `:set nonumber`
    // is the established way to say the opposite.
    let (key, default) = match key.strip_prefix("no") {
        Some(stripped) if parts.clone().next().is_none() => (stripped, "false"),
        _ => (key, "true"),
    };
    Ok(Command::Set {
        key: key.to_string(),
        value: parts.next().unwrap_or(default).to_string(),
        persist,
    })
}

/// Read a line range off the front of a command line, returning it and the
/// rest of the line.
///
/// # Errors
/// Returns a message for an address that starts but does not finish, such as
/// a `'` without a mark name.
fn parse_range(input: &str) -> Result<(Option<LineRange>, &str), String> {
    if let Some(rest) = input.strip_prefix('%') {
        let whole = LineRange {
            start: Address {
                base: Base::Line(1),
                offset: 0,
            },
            end: Address {
                base: Base::Last,
                offset: 0,
            },
        };
        return Ok((Some(whole), rest.trim_start()));
    }
    let Some((start, rest)) = parse_address(input)? else {
        return Ok((None, input));
    };
    if let Some(after) = rest.strip_prefix(',') {
        let Some((end, rest)) = parse_address(after)? else {
            return Err("the range is missing its end".to_string());
        };
        return Ok((Some(LineRange { start, end }), rest.trim_start()));
    }
    Ok((Some(LineRange { start, end: start }), rest.trim_start()))
}

fn parse_address(input: &str) -> Result<Option<(Address, &str)>, String> {
    let digits = input.chars().take_while(char::is_ascii_digit).count();
    let (base, mut rest) = if digits > 0 {
        let line = input[..digits]
            .parse::<usize>()
            .map_err(|_| "line number too large".to_string())?;
        (Base::Line(line), &input[digits..])
    } else if let Some(rest) = input.strip_prefix('.') {
        (Base::Current, rest)
    } else if let Some(rest) = input.strip_prefix('$') {
        (Base::Last, rest)
    } else if let Some(rest) = input.strip_prefix('\'') {
        let mark = rest.chars().next().ok_or("a mark name must follow '")?;
        (Base::Mark(mark), &rest[mark.len_utf8()..])
    } else if input.starts_with(['+', '-']) {
        (Base::Current, input)
    } else {
        return Ok(None);
    };
    let mut offset = 0isize;
    while let Some(sign) = rest.chars().next().filter(|ch| matches!(ch, '+' | '-')) {
        let after = &rest[1..];
        let count = after.chars().take_while(char::is_ascii_digit).count();
        let amount = if count == 0 {
            1
        } else {
            after[..count].parse::<isize>().unwrap_or(0)
        };
        offset += if sign == '+' { amount } else { -amount };
        rest = &after[count..];
    }
    Ok(Some((Address { base, offset }, rest)))
}

/// `s/pattern/replacement[/flags]`, with any single character as delimiter.
///
/// Returns `None` when the text is not a substitution at all — `:set` and
/// `:split` also start with an `s` — and `Some(Err)` for one that is but is
/// written wrongly.
fn parse_substitute(input: &str, range: Option<LineRange>) -> Option<Result<Command, String>> {
    let body = input.strip_prefix('s')?;
    let delimiter = body.chars().next()?;
    if delimiter.is_alphanumeric() || delimiter.is_whitespace() {
        // `:set`-style words must not be mistaken for a substitution.
        return None;
    }

    // The pattern may contain escaped delimiters, so split manually.
    let mut fields = vec![String::new()];
    let mut escaped = false;
    for ch in body.chars().skip(1) {
        if escaped {
            // Keep the backslash unless it was escaping the delimiter itself,
            // so regex escapes such as `\d` survive intact.
            if ch != delimiter {
                fields.last_mut()?.push('\\');
            }
            fields.last_mut()?.push(ch);
            escaped = false;
        } else if ch == '\\' {
            escaped = true;
        } else if ch == delimiter {
            fields.push(String::new());
        } else {
            fields.last_mut()?.push(ch);
        }
    }

    let pattern = fields.first()?.clone();
    if pattern.is_empty() {
        return Some(Err("the pattern is empty".to_string()));
    }
    let replacement = fields.get(1).cloned().unwrap_or_default();
    let flags = fields.get(2).cloned().unwrap_or_default();
    if let Some(unknown) = flags.chars().find(|flag| !"gciI".contains(*flag)) {
        return Some(Err(format!("unknown flag {unknown} — use g, c, i or I")));
    }
    let case_sensitive = if flags.contains('I') {
        Some(true)
    } else if flags.contains('i') {
        Some(false)
    } else {
        None
    };

    Some(Ok(Command::Substitute {
        pattern,
        replacement,
        all: flags.contains('g'),
        confirm: flags.contains('c'),
        case_sensitive,
        range,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_with_and_without_a_path() {
        assert_eq!(
            parse("w"),
            Ok(Command::Write {
                path: None,
                force: false
            })
        );
        assert_eq!(
            parse("w  out.rs"),
            Ok(Command::Write {
                path: Some(PathBuf::from("out.rs")),
                force: false
            })
        );
        assert_eq!(
            parse("w!"),
            Ok(Command::Write {
                path: None,
                force: true
            })
        );
        assert_eq!(
            parse("set ff=dos"),
            Ok(Command::Set {
                key: "ff".to_string(),
                value: "dos".to_string(),
                persist: false
            })
        );
    }

    #[test]
    fn the_all_buffer_variants() {
        assert_eq!(parse("wa"), Ok(Command::WriteAll));
        assert_eq!(parse("qa"), Ok(Command::QuitAll { force: false }));
        assert_eq!(parse("qa!"), Ok(Command::QuitAll { force: true }));
        assert_eq!(parse("xa"), Ok(Command::WriteQuitAll));
        assert_eq!(parse("wqa"), Ok(Command::WriteQuitAll));
    }

    #[test]
    fn the_bang_suffix_forces_the_command() {
        assert_eq!(parse("q!"), Ok(Command::Quit { force: true }));
        assert_eq!(parse("q"), Ok(Command::Quit { force: false }));
    }

    #[test]
    fn edit_without_a_path_only_works_as_a_reload() {
        assert_eq!(parse("e!"), Ok(Command::Reload));
        assert!(parse("e").is_err());
    }

    #[test]
    fn creation_commands_require_a_path() {
        assert_eq!(
            parse("touch src/new file.rs"),
            Ok(Command::CreateFile(PathBuf::from("src/new file.rs")))
        );
        assert_eq!(
            parse("mkdir src/widgets"),
            Ok(Command::CreateDirectory(PathBuf::from("src/widgets")))
        );
        assert_eq!(parse("touch"), Err("usage: :touch <path>".into()));
        assert_eq!(parse("mkdir"), Err("usage: :mkdir <path>".into()));
    }

    #[test]
    fn tree_path_commands_keep_spaces_in_the_destination() {
        assert_eq!(
            parse("rename src/new name.rs"),
            Ok(Command::Rename(PathBuf::from("src/new name.rs")))
        );
        assert_eq!(
            parse("copy backup folder"),
            Ok(Command::Copy(PathBuf::from("backup folder")))
        );
        assert_eq!(
            parse("move archive/file.rs"),
            Ok(Command::Move(PathBuf::from("archive/file.rs")))
        );
    }

    #[test]
    fn window_commands_are_not_mistaken_for_a_substitution() {
        // `:sp` and `:split` both start with the substitute prefix, so the two
        // syntaxes have to be told apart by what follows it.
        assert_eq!(
            parse("sp"),
            Ok(Command::Split {
                axis: Axis::Vertical
            })
        );
        assert_eq!(
            parse("split"),
            Ok(Command::Split {
                axis: Axis::Vertical
            })
        );
        assert_eq!(
            parse("vsplit"),
            Ok(Command::Split {
                axis: Axis::Horizontal
            })
        );
        assert_eq!(parse("close"), Ok(Command::CloseWindow));
        assert_eq!(parse("only"), Ok(Command::OnlyWindow));
    }

    #[test]
    fn a_bare_number_jumps_to_a_line() {
        assert_eq!(parse("42"), Ok(Command::GotoLine(41)));
        assert_eq!(parse("1"), Ok(Command::GotoLine(0)));
    }

    #[test]
    fn picker_commands_select_their_sources() {
        assert_eq!(parse("find"), Ok(Command::Find));
        assert_eq!(parse("buffers"), Ok(Command::Buffers));
        assert_eq!(parse("commands"), Ok(Command::Commands));
        assert_eq!(parse("themes"), Ok(Command::Themes));
    }

    #[test]
    fn set_accepts_a_value_a_flag_and_a_negated_flag() {
        assert_eq!(
            parse("set tab_width 2"),
            Ok(Command::Set {
                key: "tab_width".to_string(),
                value: "2".to_string(),
                persist: false
            })
        );
        assert_eq!(
            parse("set number"),
            Ok(Command::Set {
                key: "number".to_string(),
                value: "true".to_string(),
                persist: false
            })
        );
        assert_eq!(
            parse("set nonumber"),
            Ok(Command::Set {
                key: "number".to_string(),
                value: "false".to_string(),
                persist: false
            })
        );
    }

    fn at(base: Base) -> Address {
        Address { base, offset: 0 }
    }

    #[test]
    fn substitute_reads_pattern_replacement_and_flags() {
        assert_eq!(
            parse("%s/foo/bar/gc"),
            Ok(Command::Substitute {
                pattern: "foo".to_string(),
                replacement: "bar".to_string(),
                all: true,
                confirm: true,
                case_sensitive: None,
                range: Some(LineRange {
                    start: at(Base::Line(1)),
                    end: at(Base::Last),
                }),
            })
        );
        assert_eq!(
            parse("s/foo/bar/i"),
            Ok(Command::Substitute {
                pattern: "foo".to_string(),
                replacement: "bar".to_string(),
                all: false,
                confirm: false,
                case_sensitive: Some(false),
                range: None,
            })
        );
        assert!(parse("s/foo/bar/z").is_err());
    }

    #[test]
    fn substitute_keeps_regex_escapes_but_unescapes_the_delimiter() {
        assert!(matches!(
            parse(r"%s/\d+\/x/n/"),
            Ok(Command::Substitute { pattern, .. }) if pattern == r"\d+/x"
        ));
    }

    #[test]
    fn ranges_take_lines_marks_and_offsets() {
        let range = |text: &str| match parse(text) {
            Ok(Command::Substitute { range, .. }) => range,
            other => panic!("{text}: {other:?}"),
        };
        assert_eq!(
            range("'<,'>s/a/b/"),
            Some(LineRange {
                start: at(Base::Mark('<')),
                end: at(Base::Mark('>')),
            })
        );
        assert_eq!(
            range(".,.+2s/a/b/"),
            Some(LineRange {
                start: at(Base::Current),
                end: Address {
                    base: Base::Current,
                    offset: 2
                },
            })
        );
        assert_eq!(
            range("3,$-1s/a/b/"),
            Some(LineRange {
                start: at(Base::Line(3)),
                end: Address {
                    base: Base::Last,
                    offset: -1
                },
            })
        );
        assert_eq!(parse("42"), Ok(Command::GotoLine(41)));
        assert_eq!(parse("3,4"), Ok(Command::GotoLine(3)));
        assert_eq!(parse("$"), Ok(Command::GotoAddress(at(Base::Last))));
        assert!(parse("3,4w").is_err());
    }

    #[test]
    fn project_wide_commands() {
        assert_eq!(parse("grep todo"), Ok(Command::Grep(Some("todo".into()))));
        assert_eq!(parse("grep"), Ok(Command::Grep(None)));
        assert_eq!(
            parse("greplace/old/new/I"),
            Ok(Command::ProjectReplace {
                pattern: "old".into(),
                replacement: "new".into(),
                case_sensitive: Some(true),
            })
        );
        assert_eq!(
            parse("help ranges"),
            Ok(Command::HelpTopic("ranges".into()))
        );
    }

    #[test]
    fn set_is_not_mistaken_for_a_substitution() {
        assert!(matches!(parse("set number"), Ok(Command::Set { .. })));
        assert!(matches!(
            parse("set! ts=2"),
            Ok(Command::Set { persist: true, .. })
        ));
        assert_eq!(parse("config reload"), Ok(Command::Config { reload: true }));
    }

    #[test]
    fn unknown_commands_are_reported_by_name() {
        assert_eq!(
            parse("frobnicate"),
            Err("unknown command: frobnicate".into())
        );
    }
}
