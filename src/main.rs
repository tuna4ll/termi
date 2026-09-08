//! `termi` — a modern, modular terminal code editor.
//!
//! The crate is split into layers that only depend downwards:
//!
//! - [`app`] drives the event loop and owns the state
//! - [`ui`] renders that state; [`renderer`] owns the terminal
//! - [`input`] turns keys into actions
//! - [`editor`] manipulates text and knows nothing about either
//! - [`config`], [`theme`], [`syntax`], [`search`], [`undo`], [`clipboard`] and
//!   [`filesystem`] are leaf services used by the layers above

mod app;
mod clipboard;
mod config;
mod editor;
mod filesystem;
mod input;
mod renderer;
mod search;
mod syntax;
mod terminal;
mod theme;
mod ui;
mod undo;

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Result;

const USAGE: &str = "\
termi — a terminal code editor

USAGE:
    termi [OPTIONS] [FILE]...

OPTIONS:
    -h, --help       Print this message
    -V, --version    Print the version

Configuration lives in <config-dir>/termi/config.toml, and themes in
<config-dir>/termi/themes/<name>.toml.";

fn main() -> ExitCode {
    match parse_arguments() {
        Arguments::Help => {
            println!("{USAGE}");
            ExitCode::SUCCESS
        }
        Arguments::Version => {
            println!("termi {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Arguments::Open(files) => match edit(files) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                // The terminal is already restored by this point, so a plain
                // stderr message is safe and is what a shell expects.
                eprintln!("termi: {error:#}");
                ExitCode::FAILURE
            }
        },
    }
}

/// What the command line asked for.
enum Arguments {
    Help,
    Version,
    Open(Vec<PathBuf>),
}

/// Parse `argv` by hand.
///
/// The editor takes two flags and a list of files; a full argument parser would
/// be more dependency than feature.
fn parse_arguments() -> Arguments {
    let mut files = Vec::new();
    for argument in std::env::args_os().skip(1) {
        match argument.to_str() {
            Some("-h" | "--help") => return Arguments::Help,
            Some("-V" | "--version") => return Arguments::Version,
            _ => files.push(PathBuf::from(argument)),
        }
    }
    Arguments::Open(files)
}

/// Set up the terminal, run the editor, and tear the terminal down again.
fn edit(files: Vec<PathBuf>) -> Result<()> {
    renderer::install_panic_hook();

    let mut editor = app::App::new();
    for file in files {
        // A file that cannot be opened is reported in the status bar rather than
        // aborting startup — the other files should still open.
        if let Err(error) = editor.open(file) {
            editor.error(error.to_string());
        }
    }

    let mut tui = renderer::Tui::new(editor.config.mouse)?;
    let result = app::run(&mut editor, &mut tui);
    drop(tui);
    result
}
