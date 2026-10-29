//! `termi` — a modern, modular terminal code editor.
//!
//! The crate is split into layers that only depend downwards:
//!
//! - [`app`] drives the event loop and owns the state
//! - [`ui`] renders that state; [`renderer`] owns the terminal
//! - [`input`] turns keys into actions
//! - [`editor`] manipulates text and knows nothing about either
//! - [`config`], [`theme`], [`syntax`], [`search`], [`undo`], [`clipboard`],
//!   [`filesystem`], [`git`] and [`lsp`] are leaf services used by the
//!   layers above
//!
//! The binary is a thin shell around this library: it reads the command line
//! and hands over to [`app::run`]. Keeping the editor a library is what lets
//! the integration tests, the benchmarks and the fuzz targets drive the same
//! code the terminal does, with no terminal in the way.

pub mod app;
pub mod clipboard;
pub mod config;
pub mod editor;
pub mod filesystem;
pub mod git;
pub mod input;
pub mod lsp;
pub mod picker;
pub mod renderer;
pub mod search;
pub mod syntax;
pub mod terminal;
pub mod theme;
pub mod ui;
pub mod undo;
