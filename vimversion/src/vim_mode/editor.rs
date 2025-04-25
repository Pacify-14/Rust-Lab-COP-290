//! Editor state and mode handling for the Vim-like interface.
use crate::HashSet;
use crate::{cell, evaluate_cell, evaluate_formula, parse_formula, CellRef, Formula};
use egui::ViewportBuilder;
use std::process;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

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
    pub search_pattern: Option<String>,
    /// Direction of the search (true for forward, false for backward)
    pub search_forward: bool,
    /// Last search matches
    pub search_matches: Vec<(usize, usize)>,
    /// Current match index
    pub current_match: Option<usize>,
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
            search_pattern: None,
            search_forward: true,
            search_matches: Vec::new(),
            current_match: None,
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
        self.status_message = String::from("-- INSERT MODE --");

        // Initialize edit buffer with current cell's formula or value
        let row = self.cursor_row;
        let col = self.cursor_col;

        if let Some(ref formula) = sheet[row][col].formula {
            // If it has a formula, show it with = prefix
            // We need to convert the Formula AST to a string representation
            self.edit_buffer = format!("={}", formula_to_string(formula));
        } else {
            // Otherwise just show the value
            self.edit_buffer = sheet[row][col].val.to_string();
        }
    }

    pub fn enter_normal_mode(&mut self) {
        self.mode = Mode::Normal;
        self.status_message = String::from("-- NORMAL MODE --");
        self.edit_buffer.clear();
    }

    pub fn enter_command_mode(&mut self) {
        self.mode = Mode::Command;
        self.status_message = String::from("-- COMMAND MODE --");
        self.command_buffer.clear();
        self.command_buffer.push(':');
    }

    pub fn enter_visual_mode(&mut self) {
        self.mode = Mode::Visual {
            start_row: self.cursor_row,
            start_col: self.cursor_col,
        };
        self.status_message = String::from("-- VISUAL MODE --");
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
        // ONLY modify the cell at the current cursor position
        let row = self.cursor_row;
        let col = self.cursor_col;

        if self.edit_buffer.is_empty() {
            // Clear the cell
            sheet[row][col].formula = None;
            sheet[row][col].val = 0;
            sheet[row][col].err = 0;
        } else if self.edit_buffer.starts_with('=') {
            // It's a formula (starts with =)
            let formula_str = self.edit_buffer[1..].to_string();
            // Parse the formula string into a Formula AST
            match parse_formula(&formula_str) {
                Ok(formula_ast) => {
                    sheet[row][col].formula = Some(formula_ast);
                    // Value will be calculated by evaluate_sheet
                }
                Err(_) => {
                    // Invalid formula
                    sheet[row][col].formula = None;
                    sheet[row][col].val = 0;
                    sheet[row][col].err = 1;
                }
            }
        } else if let Ok(value) = self.edit_buffer.parse::<i32>() {
            // It's a simple number
            sheet[row][col].formula = None;
            sheet[row][col].val = value;
            sheet[row][col].err = 0;
        } else {
            // Try to parse as a formula without the = prefix
            match parse_formula(&self.edit_buffer) {
                Ok(formula_ast) => {
                    sheet[row][col].formula = Some(formula_ast);
                    // Value will be calculated by evaluate_sheet
                }
                Err(_) => {
                    // Invalid formula
                    sheet[row][col].formula = None;
                    sheet[row][col].val = 0;
                    sheet[row][col].err = 1;
                }
            }
        }
    }
}

// Helper function to convert Formula AST to string representation
pub fn formula_to_string(formula: &Formula) -> String {
    use crate::Formula::*;
    use crate::Op;

    match formula {
        Literal(n) => n.to_string(),
        Cell(cell_ref) => {
            let col_name = column_name(cell_ref.col as usize);
            format!("{}{}", col_name, cell_ref.row + 1)
        }
        Inc { base, offset } => {
            let base_str = formula_to_string(&Cell(*base));
            format!("{} + {}", base_str, offset)
        }
        Arith { op, left, right } => {
            let op_str = match op {
                Op::Add => "+",
                Op::Sub => "-",
                Op::Mul => "*",
                Op::Div => "/",
            };
            format!(
                "{} {} {}",
                formula_to_string(left),
                op_str,
                formula_to_string(right)
            )
        }
        Range { func, start, end } => {
            let start_col = column_name(start.col as usize);
            let end_col = column_name(end.col as usize);
            format!(
                "{}({}{}:{}{})",
                func,
                start_col,
                start.row + 1,
                end_col,
                end.row + 1
            )
        }
        SleepLiteral(n) => format!("SLEEP({})", n),
        SleepCell(cell_ref) => {
            let col_name = column_name(cell_ref.col as usize);
            format!("SLEEP({}{})", col_name, cell_ref.row + 1)
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
    let sheet: Vec<Vec<cell>> = (0..rows)
        .map(|_| {
            (0..cols)
                .map(|_| cell {
                    val: 0,
                    formula: None,
                    err: 0,
                })
                .collect()
        })
        .collect();

    // Wrap sheet in Arc<Mutex<>> for thread-safe sharing
    let sheet = Arc::new(Mutex::new(sheet));

    // Check if we should use egui or terminal UI
    run_egui_interface(rows, cols, sheet);
}

/// Runs the egui-based interface
fn run_egui_interface(rows: i32, cols: i32, sheet: Arc<Mutex<Vec<Vec<cell>>>>) {
    let app = crate::vim_mode::egui_ui::SpreadsheetApp::new(rows, cols, sheet);

    let mut native_options = eframe::NativeOptions::default();

    // Set only the fields you need
    native_options.viewport = ViewportBuilder::default()
        .with_inner_size([1024.0, 768.0])
        .with_min_inner_size([800.0, 600.0]);

    eframe::run_native(
        "Vim Spreadsheet",
        native_options,
        Box::new(|_cc| Box::new(app)),
    )
    .expect("Failed to start egui application");
}

/// Runs the terminal-based interface
fn run_terminal_interface(rows: i32, cols: i32, sheet_arc: Arc<Mutex<Vec<Vec<cell>>>>) {
    // Extract the sheet from Arc<Mutex<>> for terminal UI
    let mut sheet = sheet_arc.lock().unwrap().clone();

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
                // Continue running
                // Create a graph for dependency tracking
                let total_cells = (rows * cols) as usize;
                let mut graph = Vec::with_capacity(total_cells);
                for _ in 0..total_cells {
                    graph.push(Some(Box::new(crate::DAGNode {
                        in_degree: 0,
                        dependents: HashSet::new(),
                        dependencies: HashSet::new(),
                    })));
                }

                // Evaluate cells that need updating
                let mut evaluated = vec![false; total_cells];
                for r in 0..rows {
                    for c in 0..cols {
                        if sheet[r as usize][c as usize].formula.is_some() {
                            crate::evaluate_cell(
                                r,
                                c,
                                &mut sheet,
                                &graph,
                                &mut evaluated,
                                rows,
                                cols,
                            );
                        }
                    }
                }
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
