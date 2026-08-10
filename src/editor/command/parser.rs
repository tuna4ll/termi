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

use super::Command;
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

    // `:42` jumps to a line. Checked first because a bare number is not a name.
    if let Ok(line) = input.parse::<usize>() {
        return Ok(Command::GotoLine(line.saturating_sub(1)));
    }
    if (input.starts_with('s') || input.starts_with("%s"))
        && let Some(command) = parse_substitute(input)
    {
        return Ok(command);
    }

    let (head, rest) = match input.split_once(char::is_whitespace) {
        Some((head, rest)) => (head, rest.trim()),
        None => (input, ""),
    };
    let force = head.ends_with('!');
    let name = head.trim_end_matches('!');
    let argument = (!rest.is_empty()).then(|| rest.to_string());

    match name {
        "w" | "write" => Ok(Command::Write(argument.map(PathBuf::from))),
        "q" | "quit" => Ok(Command::Quit { force }),
        "wq" | "x" => Ok(Command::WriteQuit { force }),
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
        "set" => parse_set(rest),
        "help" | "h" => Ok(Command::Help),
        other => Err(format!("unknown command: {other}")),
    }
}

/// `set key value`, where a missing value means "turn it on".
fn parse_set(rest: &str) -> Result<Command, String> {
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
    })
}

/// `[%]s/pattern/replacement[/flags]`, with any single character as delimiter.
fn parse_substitute(input: &str) -> Option<Command> {
    let whole_file = input.starts_with('%');
    let body = input.strip_prefix('%').unwrap_or(input).strip_prefix('s')?;

    let delimiter = body.chars().next()?;
    if delimiter.is_alphanumeric() {
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
        return None;
    }
    let replacement = fields.get(1).cloned().unwrap_or_default();
    let all = fields.get(2).is_some_and(|flags| flags.contains('g'));

    Some(Command::Substitute {
        pattern,
        replacement,
        all,
        whole_file,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_with_and_without_a_path() {
        assert_eq!(parse("w"), Ok(Command::Write(None)));
        assert_eq!(
            parse("w  out.rs"),
            Ok(Command::Write(Some(PathBuf::from("out.rs"))))
        );
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
    fn set_accepts_a_value_a_flag_and_a_negated_flag() {
        assert_eq!(
            parse("set tab_width 2"),
            Ok(Command::Set {
                key: "tab_width".to_string(),
                value: "2".to_string()
            })
        );
        assert_eq!(
            parse("set number"),
            Ok(Command::Set {
                key: "number".to_string(),
                value: "true".to_string()
            })
        );
        assert_eq!(
            parse("set nonumber"),
            Ok(Command::Set {
                key: "number".to_string(),
                value: "false".to_string()
            })
        );
    }

    #[test]
    fn substitute_reads_pattern_replacement_and_flags() {
        assert_eq!(
            parse("%s/foo/bar/g"),
            Ok(Command::Substitute {
                pattern: "foo".to_string(),
                replacement: "bar".to_string(),
                all: true,
                whole_file: true,
            })
        );
        assert_eq!(
            parse("s/foo/bar"),
            Ok(Command::Substitute {
                pattern: "foo".to_string(),
                replacement: "bar".to_string(),
                all: false,
                whole_file: false,
            })
        );
    }

    #[test]
    fn substitute_keeps_regex_escapes_but_unescapes_the_delimiter() {
        assert_eq!(
            parse(r"%s/\d+\/x/n/"),
            Ok(Command::Substitute {
                pattern: r"\d+/x".to_string(),
                replacement: "n".to_string(),
                all: false,
                whole_file: true,
            })
        );
    }

    #[test]
    fn set_is_not_mistaken_for_a_substitution() {
        assert!(matches!(parse("set number"), Ok(Command::Set { .. })));
    }

    #[test]
    fn unknown_commands_are_reported_by_name() {
        assert_eq!(
            parse("frobnicate"),
            Err("unknown command: frobnicate".into())
        );
    }
}
