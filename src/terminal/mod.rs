//! # Embedded terminal sessions
//!
//! **Purpose:** run a command inside a platform pseudo-terminal and expose its
//! screen to the rest of the application.
//!
//! **Responsibility:** process lifetime, PTY input/output, terminal emulation
//! and resize propagation. This module deliberately knows nothing about editor
//! windows or ratatui; the application decides where a session lives and the UI
//! decides how its cells look.
//!
//! **Public API:** [`Terminal`].

use std::fmt;
use std::io::{Read, Write};
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;

use anyhow::{Context, Result};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};

const INITIAL_ROWS: u16 = 24;
const INITIAL_COLS: u16 = 80;
const SCROLLBACK_ROWS: usize = 10_000;
// vt100's wrapping path needs a previous row and enough columns for one wide
// glyph. The UI may clip either dimension to one cell, so keep the emulated PTY
// minimally valid and let the renderer show the visible intersection.
const MIN_ROWS: u16 = 2;
const MIN_COLS: u16 = 2;

type SharedWriter = Arc<Mutex<Box<dyn Write + Send>>>;

struct ScreenState {
    parser: vt100::Parser,
    running: bool,
}

/// One child process connected to an emulated terminal screen.
pub struct Terminal {
    screen: Arc<Mutex<ScreenState>>,
    writer: SharedWriter,
    master: Box<dyn MasterPty + Send>,
    child: Box<dyn Child + Send + Sync>,
    size: PtySize,
    name: String,
}

impl fmt::Debug for Terminal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Terminal")
            .field("size", &self.size)
            .field("name", &self.name)
            .field("running", &self.is_running())
            .finish_non_exhaustive()
    }
}

impl Terminal {
    /// Start the user's shell, optionally asking it to run one command.
    ///
    /// # Errors
    /// Returns an error when the PTY cannot be created or the process cannot be
    /// started. Reader failures after startup end the displayed session instead
    /// of taking down the editor.
    pub fn spawn(command: Option<&str>, cwd: &Path) -> Result<Self> {
        let size = PtySize {
            rows: INITIAL_ROWS,
            cols: INITIAL_COLS,
            pixel_width: 0,
            pixel_height: 0,
        };
        let pair = native_pty_system()
            .openpty(size)
            .context("unable to create a pseudo-terminal")?;
        let mut builder = command_builder(command);
        builder.cwd(cwd);
        let child = pair
            .slave
            .spawn_command(builder)
            .context("unable to start the terminal command")?;
        drop(pair.slave);

        let reader = pair
            .master
            .try_clone_reader()
            .context("unable to read from the pseudo-terminal")?;
        let writer = pair
            .master
            .take_writer()
            .context("unable to write to the pseudo-terminal")?;
        let writer = Arc::new(Mutex::new(writer));
        let screen = Arc::new(Mutex::new(ScreenState {
            parser: vt100::Parser::new(INITIAL_ROWS, INITIAL_COLS, SCROLLBACK_ROWS),
            running: true,
        }));
        read_output(Arc::clone(&screen), Arc::clone(&writer), reader);

        Ok(Self {
            screen,
            writer,
            master: pair.master,
            child,
            size,
            name: command.unwrap_or("shell").to_string(),
        })
    }

    /// Human-readable process name for the terminal status bar.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Whether the PTY output stream is still open.
    #[must_use]
    pub fn is_running(&self) -> bool {
        self.state().running
    }

    /// Read the current emulated screen while holding its short-lived lock.
    pub fn with_screen<R>(&self, read: impl FnOnce(&vt100::Screen) -> R) -> R {
        read(self.state().parser.screen())
    }

    /// Send bytes to the child process.
    ///
    /// # Errors
    /// Returns an error when the child has closed its input stream.
    pub fn write(&mut self, bytes: &[u8]) -> Result<()> {
        let mut writer = self
            .writer
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        writer
            .write_all(bytes)
            .and_then(|()| writer.flush())
            .context("unable to write to the terminal")
    }

    /// Encode and send one key, respecting the cursor mode requested by the
    /// child application.
    ///
    /// # Errors
    /// Returns an error when the child has closed its input stream.
    pub fn send_key(&mut self, key: KeyEvent) -> Result<()> {
        let application_cursor = self.with_screen(vt100::Screen::application_cursor);
        self.state().parser.screen_mut().set_scrollback(0);
        self.write(&encode_key(key, application_cursor))
    }

    /// Send pasted text, wrapping it when the child requested bracketed paste.
    ///
    /// # Errors
    /// Returns an error when the child has closed its input stream.
    pub fn paste(&mut self, text: &str) -> Result<()> {
        let bracketed = self.with_screen(vt100::Screen::bracketed_paste);
        self.state().parser.screen_mut().set_scrollback(0);
        if bracketed {
            self.write(b"\x1b[200~")?;
        }
        self.write(text.as_bytes())?;
        if bracketed {
            self.write(b"\x1b[201~")?;
        }
        Ok(())
    }

    /// Move through saved output. Negative deltas scroll away from the live
    /// prompt and positive deltas return towards it.
    pub fn scroll(&mut self, delta: isize) {
        let mut state = self.state();
        let screen = state.parser.screen_mut();
        let current = screen.scrollback();
        let next = if delta < 0 {
            current.saturating_add(delta.unsigned_abs())
        } else {
            current.saturating_sub(delta.unsigned_abs())
        };
        screen.set_scrollback(next);
    }

    /// Resize both the real PTY and its emulated screen.
    ///
    /// Zero-sized panes are promoted to one cell because neither backend
    /// accepts a terminal with no rows or columns.
    ///
    /// # Errors
    /// Returns an error when the operating-system PTY rejects the resize.
    pub fn resize(&mut self, rows: u16, cols: u16) -> Result<()> {
        let next = PtySize {
            rows: rows.max(MIN_ROWS),
            cols: cols.max(MIN_COLS),
            pixel_width: 0,
            pixel_height: 0,
        };
        if next == self.size {
            return Ok(());
        }
        self.master
            .resize(next)
            .context("unable to resize the terminal")?;
        self.state()
            .parser
            .screen_mut()
            .set_size(next.rows, next.cols);
        self.size = next;
        Ok(())
    }

    fn state(&self) -> MutexGuard<'_, ScreenState> {
        self.screen
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

fn read_output(
    screen: Arc<Mutex<ScreenState>>,
    writer: SharedWriter,
    mut reader: Box<dyn Read + Send>,
) {
    thread::spawn(move || {
        let mut bytes = [0_u8; 8192];
        let mut responder = QueryResponder::default();
        loop {
            match reader.read(&mut bytes) {
                Ok(0) | Err(_) => {
                    screen
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .running = false;
                    break;
                }
                Ok(count) => {
                    let replies = {
                        let mut state = screen
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner);
                        state.parser.process(&bytes[..count]);
                        responder.replies(&bytes[..count], state.parser.screen())
                    };
                    if !replies.is_empty() {
                        let mut writer = writer
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner);
                        let _ = writer.write_all(&replies).and_then(|()| writer.flush());
                    }
                }
            }
        }
    });
}

/// Replies to terminal capability queries consumed by the screen parser.
///
/// Interactive shells put the PTY in raw mode while they probe the terminal.
/// Without replies they keep waiting and stop echoing typed characters, which
/// looks exactly like a dead input path even though the bytes reach the child.
#[derive(Debug, Default)]
struct QueryResponder {
    incomplete: Vec<u8>,
}

impl QueryResponder {
    fn replies(&mut self, bytes: &[u8], screen: &vt100::Screen) -> Vec<u8> {
        let mut input = std::mem::take(&mut self.incomplete);
        input.extend_from_slice(bytes);
        let mut replies = Vec::new();
        let mut index = 0;

        while index < input.len() {
            if input[index] != 0x1b {
                index += 1;
                continue;
            }
            let Some(kind) = input.get(index + 1).copied() else {
                self.incomplete.extend_from_slice(&input[index..]);
                break;
            };
            match kind {
                b'[' => {
                    let Some(end) = input[index + 2..]
                        .iter()
                        .position(|byte| (0x40..=0x7e).contains(byte))
                        .map(|offset| index + 2 + offset)
                    else {
                        self.remember(&input[index..]);
                        break;
                    };
                    csi_reply(&input[index + 2..=end], screen, &mut replies);
                    index = end + 1;
                }
                b']' | b'P' => {
                    let Some((end, after)) = string_end(&input, index + 2) else {
                        self.remember(&input[index..]);
                        break;
                    };
                    let payload = &input[index + 2..end];
                    if kind == b']' {
                        osc_reply(payload, &mut replies);
                    } else {
                        dcs_reply(payload, &mut replies);
                    }
                    index = after;
                }
                _ => index += 2,
            }
        }
        replies
    }

    fn remember(&mut self, bytes: &[u8]) {
        // Escape strings are tiny in practice. A cap prevents malformed child
        // output from turning an unfinished sequence into unbounded memory.
        if bytes.len() <= 4096 {
            self.incomplete.extend_from_slice(bytes);
        }
    }
}

fn string_end(input: &[u8], start: usize) -> Option<(usize, usize)> {
    let mut index = start;
    while index < input.len() {
        if input[index] == 0x07 {
            return Some((index, index + 1));
        }
        if input[index] == 0x1b && input.get(index + 1) == Some(&b'\\') {
            return Some((index, index + 2));
        }
        index += 1;
    }
    None
}

fn csi_reply(sequence: &[u8], screen: &vt100::Screen, replies: &mut Vec<u8>) {
    match sequence {
        b"?u" => replies.extend_from_slice(b"\x1b[?0u"),
        b">q" | b">0q" => replies.extend_from_slice(
            format!("\x1bP>|termi {}\x1b\\", env!("CARGO_PKG_VERSION")).as_bytes(),
        ),
        b"c" | b"0c" => replies.extend_from_slice(b"\x1b[?1;2c"),
        b">c" | b">0c" => replies.extend_from_slice(b"\x1b[>0;1;0c"),
        b"5n" => replies.extend_from_slice(b"\x1b[0n"),
        b"6n" | b"?6n" => {
            let (row, col) = screen.cursor_position();
            if sequence.starts_with(b"?") {
                replies.extend_from_slice(format!("\x1b[?{};{}R", row + 1, col + 1).as_bytes());
            } else {
                replies.extend_from_slice(format!("\x1b[{};{}R", row + 1, col + 1).as_bytes());
            }
        }
        _ => {}
    }
}

fn osc_reply(sequence: &[u8], replies: &mut Vec<u8>) {
    match sequence {
        b"10;?" => replies.extend_from_slice(b"\x1b]10;rgb:c5c5/cdcd/d9d9\x1b\\"),
        b"11;?" => replies.extend_from_slice(b"\x1b]11;rgb:1e1e/2222/2727\x1b\\"),
        _ => {}
    }
}

fn dcs_reply(sequence: &[u8], replies: &mut Vec<u8>) {
    if let Some(capability) = sequence.strip_prefix(b"+q") {
        replies.extend_from_slice(b"\x1bP0+r");
        replies.extend_from_slice(capability);
        replies.extend_from_slice(b"\x1b\\");
    } else if let Some(setting) = sequence.strip_prefix(b"$q") {
        replies.extend_from_slice(b"\x1bP0$r");
        replies.extend_from_slice(setting);
        replies.extend_from_slice(b"\x1b\\");
    }
}

#[cfg(unix)]
fn command_builder(command: Option<&str>) -> CommandBuilder {
    let shell = std::env::var_os("SHELL").unwrap_or_else(|| "/bin/sh".into());
    let mut builder = CommandBuilder::new(shell);
    builder.env("TERM", "xterm-256color");
    builder.env("COLORTERM", "truecolor");
    if let Some(command) = command {
        builder.arg("-lc");
        builder.arg(command);
    }
    builder
}

#[cfg(windows)]
fn command_builder(command: Option<&str>) -> CommandBuilder {
    let shell = std::env::var_os("COMSPEC").unwrap_or_else(|| "cmd.exe".into());
    let mut builder = CommandBuilder::new(shell);
    builder.env("TERM", "xterm-256color");
    builder.env("COLORTERM", "truecolor");
    if let Some(command) = command {
        builder.arg("/C");
        builder.arg(command);
    }
    builder
}

/// Encode one crossterm key event using the sequences understood by common
/// terminal applications.
#[must_use]
fn encode_key(key: KeyEvent, application_cursor: bool) -> Vec<u8> {
    let mut bytes = Vec::new();

    if key.modifiers.contains(KeyModifiers::CONTROL)
        && let KeyCode::Char(ch) = key.code
        && let Some(control) = control_byte(ch)
    {
        push_alt_prefix(&mut bytes, &key);
        bytes.push(control);
        return bytes;
    }

    match key.code {
        KeyCode::Char(ch) => {
            push_alt_prefix(&mut bytes, &key);
            let mut encoded = [0_u8; 4];
            bytes.extend_from_slice(ch.encode_utf8(&mut encoded).as_bytes());
        }
        KeyCode::Enter => {
            push_alt_prefix(&mut bytes, &key);
            bytes.push(b'\r');
        }
        KeyCode::Tab => {
            push_alt_prefix(&mut bytes, &key);
            bytes.push(b'\t');
        }
        KeyCode::BackTab => bytes.extend_from_slice(b"\x1b[Z"),
        KeyCode::Backspace => {
            push_alt_prefix(&mut bytes, &key);
            bytes.push(0x7f);
        }
        KeyCode::Esc => bytes.push(0x1b),
        KeyCode::Up => {
            bytes.extend_from_slice(cursor_key(&key, 'A', application_cursor).as_bytes());
        }
        KeyCode::Down => {
            bytes.extend_from_slice(cursor_key(&key, 'B', application_cursor).as_bytes());
        }
        KeyCode::Right => {
            bytes.extend_from_slice(cursor_key(&key, 'C', application_cursor).as_bytes());
        }
        KeyCode::Left => {
            bytes.extend_from_slice(cursor_key(&key, 'D', application_cursor).as_bytes());
        }
        KeyCode::Home => {
            bytes.extend_from_slice(cursor_key(&key, 'H', application_cursor).as_bytes());
        }
        KeyCode::End => {
            bytes.extend_from_slice(cursor_key(&key, 'F', application_cursor).as_bytes());
        }
        KeyCode::Insert => bytes.extend_from_slice(b"\x1b[2~"),
        KeyCode::Delete => bytes.extend_from_slice(b"\x1b[3~"),
        KeyCode::PageUp => bytes.extend_from_slice(b"\x1b[5~"),
        KeyCode::PageDown => bytes.extend_from_slice(b"\x1b[6~"),
        KeyCode::F(number) => bytes.extend_from_slice(function_key(number)),
        _ => {}
    }
    bytes
}

fn push_alt_prefix(bytes: &mut Vec<u8>, key: &KeyEvent) {
    if key.modifiers.contains(KeyModifiers::ALT) {
        bytes.push(0x1b);
    }
}

fn cursor_key(key: &KeyEvent, suffix: char, application_cursor: bool) -> String {
    let modified = key
        .modifiers
        .intersects(KeyModifiers::SHIFT | KeyModifiers::ALT | KeyModifiers::CONTROL);
    if application_cursor && !modified {
        format!("\x1bO{suffix}")
    } else {
        modified_csi(key, suffix)
    }
}

fn control_byte(ch: char) -> Option<u8> {
    match ch.to_ascii_lowercase() {
        '@' | ' ' => Some(0x00),
        'a'..='z' => Some(ch.to_ascii_lowercase() as u8 - b'a' + 1),
        '[' => Some(0x1b),
        '\\' => Some(0x1c),
        ']' => Some(0x1d),
        '^' => Some(0x1e),
        '_' => Some(0x1f),
        '?' => Some(0x7f),
        _ => None,
    }
}

fn modified_csi(key: &KeyEvent, suffix: char) -> String {
    let modifier = 1
        + usize::from(key.modifiers.contains(KeyModifiers::SHIFT))
        + 2 * usize::from(key.modifiers.contains(KeyModifiers::ALT))
        + 4 * usize::from(key.modifiers.contains(KeyModifiers::CONTROL));
    if modifier == 1 {
        format!("\x1b[{suffix}")
    } else {
        format!("\x1b[1;{modifier}{suffix}")
    }
}

fn function_key(number: u8) -> &'static [u8] {
    match number {
        1 => b"\x1bOP",
        2 => b"\x1bOQ",
        3 => b"\x1bOR",
        4 => b"\x1bOS",
        5 => b"\x1b[15~",
        6 => b"\x1b[17~",
        7 => b"\x1b[18~",
        8 => b"\x1b[19~",
        9 => b"\x1b[20~",
        10 => b"\x1b[21~",
        11 => b"\x1b[23~",
        12 => b"\x1b[24~",
        _ => b"",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn printable_and_control_keys_are_encoded() {
        assert_eq!(
            encode_key(KeyEvent::new(KeyCode::Char('ğ'), KeyModifiers::NONE), false,),
            "ğ".as_bytes()
        );
        assert_eq!(
            encode_key(
                KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
                false,
            ),
            b"\x03"
        );
    }

    #[test]
    fn alt_and_modified_arrows_use_terminal_sequences() {
        assert_eq!(
            encode_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::ALT), false,),
            b"\x1bx"
        );
        assert_eq!(
            encode_key(KeyEvent::new(KeyCode::Up, KeyModifiers::CONTROL), false,),
            b"\x1b[1;5A"
        );
        assert_eq!(
            encode_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE), true),
            b"\x1bOA"
        );
        assert_eq!(
            encode_key(KeyEvent::new(KeyCode::Up, KeyModifiers::ALT), false),
            b"\x1b[1;3A"
        );
    }

    #[test]
    fn a_spawned_command_reaches_the_emulated_screen() {
        #[cfg(unix)]
        let command = "printf TERMI_PTY_OK";
        #[cfg(windows)]
        let command = "echo TERMI_PTY_OK";

        let cwd = std::env::current_dir().expect("the test has a working directory");
        let terminal = Terminal::spawn(Some(command), &cwd).expect("spawn a terminal command");
        let deadline = Instant::now() + Duration::from_secs(2);

        loop {
            let contents = terminal.with_screen(vt100::Screen::contents);
            if contents.contains("TERMI_PTY_OK") {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "terminal output never reached the screen: {contents:?}"
            );
            thread::sleep(Duration::from_millis(10));
        }
    }

    #[cfg(unix)]
    #[test]
    fn typed_keys_reach_the_child_process() {
        let cwd = std::env::current_dir().expect("the test has a working directory");
        let mut terminal = Terminal::spawn(Some("cat"), &cwd).expect("spawn cat in a terminal");
        terminal.resize(1, 1).expect("resize the terminal");
        terminal
            .send_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE))
            .expect("write a key to the terminal");
        terminal
            .send_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
            .expect("write another key to the terminal");
        let deadline = Instant::now() + Duration::from_secs(2);

        loop {
            let contents = terminal.with_screen(vt100::Screen::contents);
            if contents.contains('y') {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "typed key never reached the screen: {contents:?}"
            );
            thread::sleep(Duration::from_millis(10));
        }
    }

    #[cfg(unix)]
    #[test]
    fn the_default_shell_accepts_a_typed_command() {
        let cwd = std::env::current_dir().expect("the test has a working directory");
        let mut terminal = Terminal::spawn(None, &cwd).expect("spawn the default shell");
        let command = r"printf '\x54\x45\x52\x4d\x49\x5f\x53\x48\x45\x4c\x4c\x5f\x4f\x4b'";
        for ch in command.chars() {
            terminal
                .send_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE))
                .expect("type into the default shell");
        }
        terminal
            .send_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .expect("submit the shell command");
        let deadline = Instant::now() + Duration::from_secs(3);

        loop {
            let contents = terminal.with_screen(vt100::Screen::contents);
            if contents.contains("TERMI_SHELL_OK") {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "default shell never accepted the command: {contents:?}"
            );
            thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn shell_capability_queries_receive_terminal_replies() {
        let mut parser = vt100::Parser::new(24, 80, 0);
        let queries = b"\x1b[?u\x1b[>0q\x1b]11;?\x1b\\\x1bP+q696e646e\x1b\\\x1b[0c";
        parser.process(queries);

        let replies = QueryResponder::default().replies(queries, parser.screen());
        assert!(replies.windows(5).any(|part| part == b"\x1b[?0u"));
        assert!(replies.windows(4).any(|part| part == b">|te"));
        assert!(replies.windows(5).any(|part| part == b"]11;r"));
        assert!(replies.windows(5).any(|part| part == b"0+r69"));
        assert!(replies.ends_with(b"\x1b[?1;2c"));
    }
}
