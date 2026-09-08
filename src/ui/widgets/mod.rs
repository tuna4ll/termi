//! # Widgets
//!
//! **Purpose:** one module per region of the screen.
//!
//! **Responsibility:** each widget borrows exactly the state it draws and
//! implements [`ratatui::widgets::Widget`]. None of them mutate the editor, so
//! the whole render pass is a pure function of the application state plus the
//! terminal size.
//!
//! **Public API:** the widget types re-exported below.

pub mod command_bar;
pub mod editor_view;
pub mod file_tree;
pub mod popup;
pub mod search_box;
pub mod status_bar;
pub mod tabs;
pub mod terminal_view;

pub use command_bar::CommandBar;
pub use editor_view::EditorView;
pub use file_tree::FileTree;
pub use popup::Popup;
pub use search_box::SearchBox;
pub use status_bar::StatusBar;
pub use tabs::{Tab, TabBar};
pub use terminal_view::{TerminalStatusBar, TerminalView};
