//! Vim-like interface for the spreadsheet.

pub mod commands;
pub mod editor;
pub mod egui_ui;
pub mod ui; // Add the new egui UI module

// Re-export commonly used items for convenience
pub use commands::execute_command;
pub use editor::{column_name, parse_cell_reference, ClipboardContent, EditorState, Mode};
