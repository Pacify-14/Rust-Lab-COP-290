//! Vim-like interface for the spreadsheet.

pub mod commands;
pub mod editor;
pub mod ui;
pub mod egui_ui;  // Add the new egui UI module

// Re-export commonly used items for convenience
pub use editor::{EditorState, Mode, ClipboardContent, column_name, parse_cell_reference};
pub use commands::execute_command;