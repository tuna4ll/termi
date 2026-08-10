//! # Renderer
//!
//! **Purpose:** own the terminal itself.
//!
//! **Responsibility:** the *only* module allowed to put the terminal into raw
//! mode, swap to the alternate screen, and put it back again. Everything above
//! this layer draws into a [`ratatui::Frame`] and never touches stdout. Keeping
//! this in one place is what makes "restore the terminal no matter how we exit"
//! a single, auditable code path.
//!
//! **Public API:** [`Tui`], [`install_panic_hook`], [`restore`].

use std::io::{self, Stdout};
use std::panic;

use anyhow::Result;
use crossterm::cursor::{SetCursorStyle, Show};
use crossterm::event::{
    DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
};
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use crossterm::{execute, queue};
use ratatui::backend::CrosstermBackend;
use ratatui::{Frame, Terminal};

/// The concrete backend used throughout the editor.
type Backend = CrosstermBackend<Stdout>;

/// An initialised terminal that restores itself when dropped.
pub struct Tui {
    terminal: Terminal<Backend>,
}

impl Tui {
    /// Enter raw mode and the alternate screen, and take ownership of the
    /// resulting terminal.
    ///
    /// # Errors
    /// Returns an error if the terminal cannot be switched into raw mode or the
    /// alternate screen, which usually means stdout is not a tty.
    pub fn new(mouse: bool) -> Result<Self> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        // Bracketed paste turns a paste into one `Event::Paste` instead of a
        // burst of key presses — without it, pasted text triggers auto-indent on
        // every line and arrives mangled.
        execute!(stdout, EnterAlternateScreen, EnableBracketedPaste)?;
        // Capturing the mouse costs the terminal's own drag-to-select, so it is
        // only taken when the user has left it switched on.
        if mouse {
            execute!(stdout, EnableMouseCapture)?;
        }
        let terminal = Terminal::new(CrosstermBackend::new(stdout))?;
        Ok(Self { terminal })
    }

    /// Render one frame.
    ///
    /// # Errors
    /// Returns an error if the frame cannot be flushed to the terminal.
    pub fn draw<F: FnOnce(&mut Frame)>(&mut self, render: F) -> Result<()> {
        self.terminal.draw(render)?;
        Ok(())
    }

    /// Redraw everything on the next frame, discarding the diff cache.
    ///
    /// # Errors
    /// Returns an error if the terminal cannot be cleared.
    pub fn clear(&mut self) -> Result<()> {
        self.terminal.clear()?;
        Ok(())
    }

    /// Switch the hardware cursor between a block and a bar.
    ///
    /// # Errors
    /// Returns an error if the escape sequence cannot be written.
    pub fn set_cursor_shape(&mut self, bar: bool) -> Result<()> {
        let style = if bar {
            SetCursorStyle::SteadyBar
        } else {
            SetCursorStyle::SteadyBlock
        };
        queue!(io::stdout(), style)?;
        Ok(())
    }
}

impl Drop for Tui {
    fn drop(&mut self) {
        // Best effort: we are already on the way out, so a failure here has
        // nowhere useful to go.
        let _ = restore();
    }
}

/// Put the terminal back the way we found it.
///
/// Safe to call more than once, and safe to call when setup never completed.
///
/// # Errors
/// Returns an error if the terminal state cannot be reset.
pub fn restore() -> io::Result<()> {
    // Disabling mouse capture unconditionally: it is harmless when it was never
    // enabled, and this path has to work even when setup failed halfway.
    execute!(
        io::stdout(),
        DisableMouseCapture,
        DisableBracketedPaste,
        LeaveAlternateScreen,
        SetCursorStyle::DefaultUserShape,
        Show
    )?;
    disable_raw_mode()
}

/// Make sure a panic leaves the user with a usable shell.
///
/// Without this the alternate screen swallows the panic message and raw mode
/// makes the shell unusable afterwards.
pub fn install_panic_hook() {
    let previous = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        let _ = restore();
        previous(info);
    }));
}
