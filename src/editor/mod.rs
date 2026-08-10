//! # Editor core
//!
//! **Purpose:** everything that manipulates text, with no knowledge of
//! terminals, keys or themes.
//!
//! **Responsibility:** this layer is deliberately UI-free so it stays testable
//! and reusable. It is split by concern:
//!
//! - [`document`] — the text itself, its file and its dirty state
//! - [`cursor`] — text coordinates and motions
//! - [`selection`] — ranges anchored to a cursor
//! - [`buffer`] — a file being edited: its text, history and highlighting
//! - [`window`] — a view onto a buffer: its cursors and scroll position
//! - [`edit`] — the operations that change a buffer's text through a window
//! - [`command`] — ex-style commands typed into the command bar

pub mod buffer;
pub mod command;
pub mod cursor;
pub mod document;
pub mod edit;
pub mod selection;
pub mod window;
