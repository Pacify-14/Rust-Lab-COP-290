//! Editor state and mode handling for the Vim-like interface.

use crate::{cell, evaluate_sheet, get_col_index};
use std::io;
use std::process;

/// Represents the current editing mode of the Vim-like interface.
#[derive(PartialEq, Clone, Debug)]
pub enum Mode {
    /// Normal mode for navigation and commands
    Normal,
    /// Insert mode for editing cell content
    Insert,
    /// Command mode for executing commands
    Command,
    /// Visual mode for selecting ranges
    Visual {
        /// Starting row of the selection
        start_row: usize,
        /// Starting column of the selection
        start_col: usize,
    },
}

/// Represents the current state of the editor.
pub struct EditorState {
    /// Current editing mode
    pub mode: Mode,
    /// Current cursor row position (0-indexed)
    pub cursor_row: usize,
    /// Current cursor column position (0-indexed)
    pub cursor_col: usize,
    /// Buffer for command input
    pub command_buffer: String,
    /// Status message to display
    pub status_message: String,
    /// Clipboard content for copy/paste operations
    pub clipboard: Option<ClipboardContent>,
    /// Current row offset for viewport
    pub row_offset: usize,
    /// Current column offset for viewport
    pub col_offset: usize,
    /// Current cell formula being edited in insert mode
    pub edit_buffer: String,
}

/// Represents content that can be stored in the clipboard.
#[derive(Clone)]
pub enum ClipboardContent {
    /// A single cell
    Cell {
        /// Row of the copied cell
        row: usize,
        /// Column of the copied cell
        col: usize,
        /// Value or formula of the copied cell
        value: String,
    },
    /// A range of cells
    Range {
        /// Starting row of the range
        start_row: usize,
        /// Starting column of the range
        start_col: usize,
        /// Ending row of the range
        end_row: usize,
        /// Ending column of the range
        end_col: usize,
        /// 2D array of values or formulas
        data: Vec<Vec<String>>,
    },
    /// An entire row
    Row {
        /// Row index
        row: usize,
        /// Row data
        data: Vec<String>,
    },
    /// An entire column
    Column {
        /// Column index
        col: usize,
        /// Column data
        data: Vec<String>,
    },
}

impl EditorState {
    /// Creates a new editor state with default values.
    pub fn new() -> Self {
        Self {
            mode: Mode::Normal,
            cursor_row: 0,
            cursor_col: 0,
            command_buffer: String::new(),
            status_message: String::from("Normal mode"),
            clipboard: None,
            row_offset: 0, // Explicitly set to 0
            col_offset: 0, // Explicitly set to 0
            edit_buffer: String::new(),
        }
    }
    pub fn reset_view(&mut self) {
        self.row_offset = 0;
        self.col_offset = 0;
        self.cursor_row = 0;
        self.cursor_col = 0;
    }

    /// Switches to insert mode.
    pub fn enter_insert_mode(&mut self, sheet: &Vec<Vec<cell>>) {
        self.mode = Mode::Insert;
        self.status_message = String::from("-- INSERT --");

        // Initialize edit buffer with current cell's formula or value
        if let Some(ref formula) = sheet[self.cursor_row][self.cursor_col].formula {
            self.edit_buffer = formula.clone();
        } else {
            self.edit_buffer = sheet[self.cursor_row][self.cursor_col].val.to_string();
        }
    }

    /// Switches to normal mode.
    pub fn enter_normal_mode(&mut self) {
        self.mode = Mode::Normal;
        self.status_message = String::from("Normal mode");
        self.edit_buffer.clear();
    }

    /// Switches to command mode.
    pub fn enter_command_mode(&mut self) {
        self.mode = Mode::Command;
        self.command_buffer.clear();
        self.command_buffer.push(':');
    }

    /// Switches to visual mode.
    pub fn enter_visual_mode(&mut self) {
        self.mode = Mode::Visual {
            start_row: self.cursor_row,
            start_col: self.cursor_col,
        };
        self.status_message = String::from("-- VISUAL --");
    }

    /// Gets the current visual selection range, if in visual mode.
    pub fn get_visual_selection(&self) -> Option<(usize, usize, usize, usize)> {
        match self.mode {
            Mode::Visual {
                start_row,
                start_col,
            } => {
                let min_row = start_row.min(self.cursor_row);
                let max_row = start_row.max(self.cursor_row);
                let min_col = start_col.min(self.cursor_col);
                let max_col = start_col.max(self.cursor_col);

                Some((min_row, min_col, max_row, max_col))
            }
            _ => None,
        }
    }

    /// Applies the current edit buffer to the cell at the cursor position.
    pub fn apply_edit(&self, sheet: &mut Vec<Vec<cell>>) {
        if self.edit_buffer.is_empty() {
            sheet[self.cursor_row][self.cursor_col].formula = None;
            sheet[self.cursor_row][self.cursor_col].val = 0;
        } else if let Ok(value) = self.edit_buffer.parse::<i32>() {
            // If it's a simple number, store it directly
            sheet[self.cursor_row][self.cursor_col].formula = None;
            sheet[self.cursor_row][self.cursor_col].val = value;
        } else {
            // Otherwise, treat it as a formula
            sheet[self.cursor_row][self.cursor_col].formula = Some(self.edit_buffer.clone());
        }
    }
}

/// Column name utility function (converts 0-based index to "A", "B", ..., "Z", "AA", etc.)
pub fn column_name(col: usize) -> String {
    let mut name = String::new();
    let mut n = col + 1;

    while n > 0 {
        n -= 1;
        name.insert(0, (b'A' + (n % 26) as u8) as char);
        n /= 26;
    }

    name
}

/// Runs the Vim-like interface for the spreadsheet.
pub fn run_vim_interface(rows: i32, cols: i32) {
    if rows < 1 || rows > 100000 || cols < 1 || cols > (26 * 26 * 26 + 26 * 26 + 26) {
        println!("Invalid grid size.");
        process::exit(1);
    }

    // Initialize sheet
    let mut sheet: Vec<Vec<cell>> = Vec::with_capacity(rows as usize);
    for _ in 0..rows {
        let mut row_vec: Vec<cell> = Vec::with_capacity(cols as usize);
        for _ in 0..cols {
            row_vec.push(cell {
                val: 0,
                formula: None,
                err: 0,
            });
        }
        sheet.push(row_vec);
    }

    // Initialize editor state
    let mut state = EditorState::new();
    state.reset_view();

    // Initialize UI
    match crate::vim_mode::ui::init_terminal() {
        Ok(_) => {}
        Err(e) => {
            eprintln!("Error initializing terminal: {}", e);
            process::exit(1);
        }
    }

    // Main event loop
    loop {
        // Render the sheet
        if let Err(e) = crate::vim_mode::ui::render_sheet(&sheet, &state, rows, cols) {
            cleanup_and_exit(&e.to_string());
        }

        // Handle input
        match crate::vim_mode::ui::handle_input(&mut state, &mut sheet, rows, cols) {
            Ok(true) => {
                // Evaluate sheet after changes
                evaluate_sheet(rows, cols, &mut sheet);
            }
            Ok(false) => {
                // Exit requested
                break;
            }
            Err(e) => {
                cleanup_and_exit(&e.to_string());
            }
        }
    }

    // Cleanup terminal
    crate::vim_mode::ui::cleanup_terminal().unwrap_or_else(|e| {
        eprintln!("Error cleaning up terminal: {}", e);
    });
}

/// Cleans up the terminal and exits with an error message.
fn cleanup_and_exit(message: &str) {
    crate::vim_mode::ui::cleanup_terminal().unwrap_or_else(|e| {
        eprintln!("Error cleaning up terminal: {}", e);
    });
    eprintln!("Error: {}", message);
    process::exit(1);
}

/// Parses a cell reference string (e.g., "A1") into row and column indices.
pub fn parse_cell_reference(s: &str) -> Option<(usize, usize)> {
    let mut col_str = String::new();
    let mut row_str = String::new();

    for c in s.chars() {
        if c.is_alphabetic() {
            col_str.push(c.to_ascii_uppercase());
        } else if c.is_numeric() {
            row_str.push(c);
        } else {
            return None;
        }
    }

    if col_str.is_empty() || row_str.is_empty() {
        return None;
    }

    let row = match row_str.parse::<usize>() {
        Ok(r) => r.checked_sub(1)?, // Convert to 0-indexed
        Err(_) => return None,
    };

    let col = {
        let mut result = 0;
        for c in col_str.chars() {
            result = result * 26 + (c as usize - 'A' as usize + 1);
        }
        result.checked_sub(1)? // Convert to 0-indexed
    };

    Some((row, col))
}
