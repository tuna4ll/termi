//! The `termi` binary: read the command line, then run the editor.

use std::ffi::OsString;
use std::io::Read;
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Result;

use termi::app::state::Startup;
use termi::editor::cursor::Position;
use termi::{app, editor, filesystem, renderer};

const USAGE: &str = "\
termi — a terminal code editor

USAGE:
    termi [OPTIONS] [+LINE] [FILE[:LINE[:COLUMN]]]...
    command | termi -

OPTIONS:
    -h, --help           Print this message
    -V, --version        Print the version
    -R, --readonly       Open every file read-only
    -c, --config FILE    Read settings from FILE instead of the usual place
        --clean          Ignore the config file and any saved session
        --no-session     Neither restore nor save the session
    +LINE                Put the caret on LINE of the next file (+ alone: the end)
    -                    Edit what arrives on standard input
    --                   Treat everything after this as a file name

With no files, the last session in this directory is restored.

Configuration lives in <config-dir>/termi/config.toml, themes in
<config-dir>/termi/themes/<name>.toml and language scripts in
<config-dir>/termi/syntax/<name>.lua.";

fn main() -> ExitCode {
    match parse_arguments(std::env::args_os().skip(1).collect()) {
        Ok(Arguments::Help) => {
            println!("{USAGE}");
            ExitCode::SUCCESS
        }
        Ok(Arguments::Version) => {
            println!("termi {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Ok(Arguments::Open(request)) => match edit(request) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                // The terminal is already restored by this point, so a plain
                // stderr message is safe and is what a shell expects.
                eprintln!("termi: {error:#}");
                ExitCode::FAILURE
            }
        },
        Err(message) => {
            eprintln!("termi: {message}\n\n{USAGE}");
            ExitCode::FAILURE
        }
    }
}

/// What the command line asked for.
#[derive(Debug, PartialEq, Eq)]
enum Arguments {
    Help,
    Version,
    Open(Request),
}

/// The files to open and how.
#[derive(Debug, Default, PartialEq, Eq)]
struct Request {
    /// Each file, with where to put the caret in it.
    files: Vec<(PathBuf, Option<Target>)>,
    /// Read standard input into the first buffer.
    stdin: bool,
    startup: StartupFlags,
}

/// The startup options, kept comparable for the tests.
#[derive(Debug, Default, PartialEq, Eq)]
struct StartupFlags {
    config_path: Option<PathBuf>,
    clean: bool,
    no_session: bool,
    readonly: bool,
}

/// Where in a file to start.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Target {
    /// A one-based line and, optionally, column.
    At(usize, Option<usize>),
    /// The last line, for a bare `+`.
    End,
}

/// Parse `argv` by hand.
///
/// The editor takes a handful of flags and a list of files; a full argument
/// parser would be more dependency than feature.
fn parse_arguments(arguments: Vec<OsString>) -> Result<Arguments, String> {
    let mut request = Request::default();
    let mut pending_line: Option<Target> = None;
    let mut only_files = false;
    let mut arguments = arguments.into_iter();
    while let Some(argument) = arguments.next() {
        let text = argument.to_str();
        if !only_files {
            match text {
                Some("-h" | "--help") => return Ok(Arguments::Help),
                Some("-V" | "--version") => return Ok(Arguments::Version),
                Some("-R" | "--readonly") => {
                    request.startup.readonly = true;
                    continue;
                }
                Some("-c" | "--config") => {
                    let path = arguments
                        .next()
                        .ok_or_else(|| "--config needs a file".to_string())?;
                    request.startup.config_path = Some(PathBuf::from(path));
                    continue;
                }
                Some("--clean") => {
                    request.startup.clean = true;
                    continue;
                }
                Some("--no-session") => {
                    request.startup.no_session = true;
                    continue;
                }
                Some("--") => {
                    only_files = true;
                    continue;
                }
                Some("-") => {
                    request.stdin = true;
                    continue;
                }
                Some("+") => {
                    pending_line = Some(Target::End);
                    continue;
                }
                Some(line) if line.starts_with('+') => {
                    let number = line[1..]
                        .parse::<usize>()
                        .map_err(|_| format!("{line} is not a line number"))?;
                    pending_line = Some(Target::At(number, None));
                    continue;
                }
                Some(flag) if flag.starts_with('-') && flag.len() > 1 => {
                    return Err(format!("unknown option {flag}"));
                }
                _ => {}
            }
        }
        let (path, target) = split_position(PathBuf::from(argument));
        request.files.push((path, pending_line.take().or(target)));
    }
    Ok(Arguments::Open(request))
}

/// `src/main.rs:42:7` as a path and a place in it, the way compilers and
/// `grep -n` print locations. A file that really is called `notes:3` wins.
fn split_position(path: PathBuf) -> (PathBuf, Option<Target>) {
    if path.exists() {
        return (path, None);
    }
    let Some(text) = path.to_str() else {
        return (path, None);
    };
    let mut parts = text.rsplitn(3, ':');
    let last = parts.next().and_then(|part| part.parse::<usize>().ok());
    let middle = parts.next();
    let rest = parts.next();
    match (last, middle, rest) {
        (Some(column), Some(line), Some(file)) if !file.is_empty() => {
            if let Ok(line) = line.parse::<usize>() {
                return (PathBuf::from(file), Some(Target::At(line, Some(column))));
            }
            (
                PathBuf::from(format!("{file}:{line}")),
                Some(Target::At(column, None)),
            )
        }
        (Some(line), Some(file), None) if !file.is_empty() => {
            (PathBuf::from(file), Some(Target::At(line, None)))
        }
        _ => (path, None),
    }
}

/// Set up the terminal, run the editor, and tear the terminal down again.
fn edit(request: Request) -> Result<()> {
    renderer::install_panic_hook();

    // Standard input has to be read before the terminal is taken over; the
    // keyboard then comes from the terminal itself rather than the pipe.
    let piped = if request.stdin {
        let mut bytes = Vec::new();
        std::io::stdin().read_to_end(&mut bytes)?;
        Some(filesystem::decode(&bytes)?.0)
    } else {
        None
    };

    let flags = request.startup;
    let mut editor = app::App::start(Startup {
        config_path: flags.config_path,
        clean: flags.clean,
        no_session: flags.no_session,
        readonly: flags.readonly,
    });
    if let Some(text) = piped {
        editor.buffers[0] =
            editor::buffer::Buffer::new(editor::document::Document::from_text(&text, None));
    }
    let restore_session = request.files.is_empty()
        && !request.stdin
        && editor.config.restore_session
        && !editor.startup.clean
        && !editor.startup.no_session;
    for (file, target) in request.files {
        // A file that cannot be opened is reported in the status bar rather than
        // aborting startup — the other files should still open.
        if let Err(error) = editor.open(file) {
            editor.error(error.to_string());
            continue;
        }
        if let Some(target) = target {
            let document = &editor.buffer().document;
            let position = match target {
                Target::End => Position::new(document.last_line(), 0),
                Target::At(line, column) => Position::new(
                    line.saturating_sub(1),
                    column.map_or(0, |column| column.saturating_sub(1)),
                ),
            };
            let position = document.clamp(position, false);
            editor.window_mut().cursor_mut().move_to(position, false);
        }
    }
    if restore_session {
        app::session::restore(&mut editor);
    }

    let mut tui = renderer::Tui::new(editor.config.mouse)?;
    let result = app::run(&mut editor, &mut tui);
    drop(tui);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(arguments: &[&str]) -> Result<Arguments, String> {
        parse_arguments(arguments.iter().map(OsString::from).collect())
    }

    fn request(arguments: &[&str]) -> Request {
        match parse(arguments) {
            Ok(Arguments::Open(request)) => request,
            other => panic!("{arguments:?} parsed as {other:?}"),
        }
    }

    #[test]
    fn file_positions_in_compiler_style() {
        let parsed = request(&["src/nowhere.rs:42:7", "other.txt:3"]);
        assert_eq!(
            parsed.files,
            vec![
                (
                    PathBuf::from("src/nowhere.rs"),
                    Some(Target::At(42, Some(7)))
                ),
                (PathBuf::from("other.txt"), Some(Target::At(3, None))),
            ]
        );
    }

    #[test]
    fn a_plus_line_applies_to_the_next_file() {
        let parsed = request(&["+10", "a.txt", "b.txt", "+", "c.txt"]);
        assert_eq!(parsed.files[0].1, Some(Target::At(10, None)));
        assert_eq!(parsed.files[1].1, None);
        assert_eq!(parsed.files[2].1, Some(Target::End));
    }

    #[test]
    fn flags_and_the_double_dash() {
        let parsed = request(&["-R", "--config", "my.toml", "--no-session", "-", "--", "-R"]);
        assert!(parsed.startup.readonly);
        assert!(parsed.startup.no_session);
        assert!(parsed.stdin);
        assert_eq!(parsed.startup.config_path, Some(PathBuf::from("my.toml")));
        assert_eq!(parsed.files, vec![(PathBuf::from("-R"), None)]);
        assert!(parse(&["--frobnicate"]).is_err());
        assert_eq!(parse(&["-V"]), Ok(Arguments::Version));
    }
}
