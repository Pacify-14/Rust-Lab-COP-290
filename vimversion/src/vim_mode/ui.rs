//! Terminal UI handling for the Vim-like interface.

use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEvent, KeyModifiers},
    execute,
    style::{self, Color},
    terminal::{self, ClearType, EnterAlternateScreen, LeaveAlternateScreen},
};
use std::io::{self, Write};

use crate::vim_mode::commands::execute_command;
use crate::vim_mode::editor::{
    column_name, formula_to_string, parse_cell_reference, EditorState, Mode,
};
use crate::{cell, evaluate_cell, get_col_index, parse_formula, Formula, HashSet};

/// Initializes the terminal for the Vim-like interface.
pub fn init_terminal() -> io::Result<()> {
    terminal::enable_raw_mode()?;
    execute!(io::stdout(), EnterAlternateScreen)?;
    Ok(())
}

/// Cleans up the terminal when exiting the Vim-like interface.
pub fn cleanup_terminal() -> io::Result<()> {
    execute!(io::stdout(), LeaveAlternateScreen)?;
    terminal::disable_raw_mode()?;
    Ok(())
}

/// Renders the spreadsheet in the terminal.
pub fn render_sheet(
    sheet: &Vec<Vec<cell>>,
    state: &EditorState,
    rows: i32,
    cols: i32,
) -> io::Result<()> {
    // Get terminal size
    let (term_width, term_height) = terminal::size()?;

    // Clear screen
    execute!(
        io::stdout(),
        terminal::Clear(ClearType::All),
        cursor::MoveTo(0, 0)
    )?;

    // Calculate visible range
    let visible_rows = (term_height - 3).min(20) as usize;
    let visible_cols = (term_width / 10).min(10) as usize;

    let row_offset = state.row_offset;
    let col_offset = state.col_offset;

    // Render header row with column labels
    print!("      "); // Space for row numbers
    for col in col_offset..(col_offset + visible_cols.min(cols as usize - col_offset)) {
        let col_name = column_name(col);
        print!("{:^8} ", col_name);
    }
    println!();

    // Add a separator line
    print!("      ");
    for _ in col_offset..(col_offset + visible_cols.min(cols as usize - col_offset)) {
        print!("-------- ");
    }
    println!();

    // Render each row
    for row in row_offset..(row_offset + visible_rows.min(rows as usize - row_offset)) {
        // Print row number at the beginning of each line
        print!("{:4} | ", row + 1);

        // Print cells in this row
        for col in col_offset..(col_offset + visible_cols.min(cols as usize - col_offset)) {
            let is_cursor = row == state.cursor_row && col == state.cursor_col;
            let is_selected = match state.get_visual_selection() {
                Some((min_row, min_col, max_row, max_col)) => {
                    row >= min_row && row <= max_row && col >= min_col && col <= max_col
                }
                None => false,
            };

            let cell_value = if sheet[row][col].err != 0 {
                "ERR".to_string()
            } else {
                sheet[row][col].val.to_string()
            };

            if is_cursor {
                execute!(io::stdout(), style::SetAttribute(style::Attribute::Reverse))?;
            } else if is_selected {
                execute!(
                    io::stdout(),
                    style::SetAttribute(style::Attribute::Underlined)
                )?;
            }

            print!("{:^8}", cell_value);

            if is_cursor || is_selected {
                execute!(io::stdout(), style::SetAttribute(style::Attribute::Reset))?;
            }

            print!(" ");
        }
        println!(); // End of row - move to next line
    }

    // Add a blank line before status bar
    println!();

    // Render status bar
    let current_cell = format!("{}{}", column_name(state.cursor_col), state.cursor_row + 1);

    // Show formula or edit buffer depending on mode
    let formula = match state.mode {
        Mode::Insert => state.edit_buffer.clone(),
        _ => {
            if let Some(ref formula) = sheet[state.cursor_row][state.cursor_col].formula {
                formula_to_string(formula)
            } else {
                "".to_string()
            }
        }
    };

    execute!(
        io::stdout(),
        cursor::MoveTo(0, term_height - 2),
        terminal::Clear(ClearType::CurrentLine),
        style::SetBackgroundColor(Color::Blue),
        style::SetForegroundColor(Color::White)
    )?;

    print!(" {} | Formula: {} ", current_cell, formula);

    execute!(
        io::stdout(),
        style::SetAttribute(style::Attribute::Reset),
        cursor::MoveTo(0, term_height - 1),
        terminal::Clear(ClearType::CurrentLine)
    )?;

    match state.mode {
        Mode::Command => {
            print!("{}", state.command_buffer);
        }
        Mode::Insert => {
            execute!(
                io::stdout(),
                style::SetBackgroundColor(Color::Green),
                style::SetForegroundColor(Color::Black)
            )?;
            print!(" {} ", state.status_message);
            execute!(io::stdout(), style::SetAttribute(style::Attribute::Reset))?;
        }
        Mode::Visual { .. } => {
            execute!(
                io::stdout(),
                style::SetBackgroundColor(Color::Magenta),
                style::SetForegroundColor(Color::White)
            )?;
            print!(" {} ", state.status_message);
            execute!(io::stdout(), style::SetAttribute(style::Attribute::Reset))?;
        }
        _ => {
            execute!(
                io::stdout(),
                style::SetBackgroundColor(Color::DarkBlue),
                style::SetForegroundColor(Color::White)
            )?;
            print!(" {} ", state.status_message);
            execute!(io::stdout(), style::SetAttribute(style::Attribute::Reset))?;
        }
    }

    // Position cursor
    match state.mode {
        Mode::Command => {
            execute!(
                io::stdout(),
                cursor::MoveTo(state.command_buffer.len() as u16, term_height - 1)
            )?;
        }
        Mode::Insert | _ => {
            // Show cursor at current cell
            let cursor_x = 6 + (state.cursor_col - col_offset) * 9 + 4; // Adjusted for new column width
            let cursor_y = (state.cursor_row - row_offset) + 2; // +2 for header rows

            execute!(
                io::stdout(),
                cursor::MoveTo(cursor_x as u16, cursor_y as u16)
            )?;
        }
    }

    io::stdout().flush()?;
    Ok(())
}

/// Handles keyboard input based on the current mode.
pub fn handle_input(
    state: &mut EditorState,
    sheet: &mut Vec<Vec<cell>>,
    rows: i32,
    cols: i32,
) -> io::Result<bool> {
    if let Event::Key(key) = event::read()? {
        match state.mode {
            Mode::Normal => handle_normal_mode(key, state, sheet, rows, cols),
            Mode::Insert => handle_insert_mode(key, state, sheet, rows, cols),
            Mode::Command => handle_command_mode(key, state, sheet, rows, cols),
            Mode::Visual { .. } => handle_visual_mode(key, state, sheet, rows, cols),
        }
    } else {
        Ok(true) // Continue running if not a key event
    }
}

/// Handles keyboard input in normal mode.
fn handle_normal_mode(
    key: KeyEvent,
    state: &mut EditorState,
    sheet: &mut Vec<Vec<cell>>,
    rows: i32,
    cols: i32,
) -> io::Result<bool> {
    match key.code {
        KeyCode::Char('q') if key.modifiers.contains(KeyModifiers::CONTROL) => return Ok(false), // Quit
        KeyCode::Char('h') => {
            if state.cursor_col > 0 {
                state.cursor_col -= 1;
                if state.cursor_col < state.col_offset {
                    state.col_offset = state.cursor_col;
                }
            }
        }
        KeyCode::Char('j') => {
            if state.cursor_row + 1 < rows as usize {
                state.cursor_row += 1;
                if state.cursor_row >= state.row_offset + 20 {
                    state.row_offset += 1;
                }
            }
        }
        KeyCode::Char('k') => {
            if state.cursor_row > 0 {
                state.cursor_row -= 1;
                if state.cursor_row < state.row_offset {
                    state.row_offset = state.cursor_row;
                }
            }
        }
        KeyCode::Char('l') => {
            if state.cursor_col + 1 < cols as usize {
                state.cursor_col += 1;
                if state.cursor_col >= state.col_offset + 20 {
                    state.col_offset += 1;
                }
            }
        }
        KeyCode::Char('i') => state.enter_insert_mode(sheet),
        KeyCode::Char(':') => state.enter_command_mode(),
        KeyCode::Char('v') => state.enter_visual_mode(),
        KeyCode::Char('y') => {
            // Yank (copy) current cell
            let formula_str =
                if let Some(ref formula) = sheet[state.cursor_row][state.cursor_col].formula {
                    formula_to_string(formula)
                } else {
                    sheet[state.cursor_row][state.cursor_col].val.to_string()
                };

            state.clipboard = Some(crate::vim_mode::editor::ClipboardContent::Cell {
                row: state.cursor_row,
                col: state.cursor_col,
                value: formula_str,
            });

            state.status_message = format!(
                "Yanked cell {}{}",
                column_name(state.cursor_col),
                state.cursor_row + 1
            );
        }
        KeyCode::Char('p') => {
            // Paste from clipboard
            if let Some(ref content) = state.clipboard {
                match content {
                    crate::vim_mode::editor::ClipboardContent::Cell { value, .. } => {
                        // Try to parse the formula string
                        match parse_formula(value) {
                            Ok(formula) => {
                                sheet[state.cursor_row][state.cursor_col].formula = Some(formula);
                                sheet[state.cursor_row][state.cursor_col].err = 0;
                                state.status_message = format!(
                                    "Pasted to {}{}",
                                    column_name(state.cursor_col),
                                    state.cursor_row + 1
                                );
                            }
                            Err(_) => {
                                // If parsing fails, try to interpret as a number
                                if let Ok(val) = value.parse::<i32>() {
                                    sheet[state.cursor_row][state.cursor_col].formula = None;
                                    sheet[state.cursor_row][state.cursor_col].val = val;
                                    sheet[state.cursor_row][state.cursor_col].err = 0;
                                    state.status_message = format!(
                                        "Pasted value {} to {}{}",
                                        val,
                                        column_name(state.cursor_col),
                                        state.cursor_row + 1
                                    );
                                } else {
                                    // Set as error
                                    sheet[state.cursor_row][state.cursor_col].formula = None;
                                    sheet[state.cursor_row][state.cursor_col].val = 0;
                                    sheet[state.cursor_row][state.cursor_col].err = 1;
                                    state.status_message = "Invalid formula or value".to_string();
                                }
                            }
                        }
                    }
                    crate::vim_mode::editor::ClipboardContent::Range {
                        start_row,
                        start_col,
                        end_row,
                        end_col,
                        data,
                    } => {
                        // Paste range
                        let height = end_row - start_row + 1;
                        let width = end_col - start_col + 1;

                        for i in 0..height {
                            for j in 0..width {
                                let target_row = state.cursor_row + i;
                                let target_col = state.cursor_col + j;

                                if target_row < rows as usize && target_col < cols as usize {
                                    // Try to parse the formula string
                                    match parse_formula(&data[i][j]) {
                                        Ok(formula) => {
                                            sheet[target_row][target_col].formula = Some(formula);
                                            sheet[target_row][target_col].err = 0;
                                        }
                                        Err(_) => {
                                            // If parsing fails, try to interpret as a number
                                            if let Ok(val) = data[i][j].parse::<i32>() {
                                                sheet[target_row][target_col].formula = None;
                                                sheet[target_row][target_col].val = val;
                                                sheet[target_row][target_col].err = 0;
                                            } else {
                                                // Set as error
                                                sheet[target_row][target_col].formula = None;
                                                sheet[target_row][target_col].val = 0;
                                                sheet[target_row][target_col].err = 1;
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        state.status_message = format!(
                            "Pasted range to {}{}",
                            column_name(state.cursor_col),
                            state.cursor_row + 1
                        );
                    }
                    _ => {}
                }

                // Create a temporary graph for dependency tracking
                let total_cells = (rows * cols) as usize;
                let mut graph = Vec::with_capacity(total_cells);
                for _ in 0..total_cells {
                    graph.push(Some(Box::new(crate::DAGNode {
                        in_degree: 0,
                        dependents: HashSet::new(),
                        dependencies: HashSet::new(),
                    })));
                }

                // Evaluate cells after pasting
                let mut evaluated = vec![false; total_cells];
                for r in 0..rows {
                    for c in 0..cols {
                        if sheet[r as usize][c as usize].formula.is_some() {
                            evaluate_cell(r, c, sheet, &graph, &mut evaluated, rows, cols);
                        }
                    }
                }
            }
        }
        KeyCode::Char('d') => {
            // Delete current cell
            sheet[state.cursor_row][state.cursor_col].formula = None;
            sheet[state.cursor_row][state.cursor_col].val = 0;
            sheet[state.cursor_row][state.cursor_col].err = 0;
            state.status_message = format!(
                "Deleted cell {}{}",
                column_name(state.cursor_col),
                state.cursor_row + 1
            );
        }
        KeyCode::Char('G') => {
            // Go to last row
            state.cursor_row = (rows - 1) as usize;
            if state.cursor_row >= state.row_offset + 20 {
                state.row_offset = state.cursor_row.saturating_sub(10);
            }
        }
        KeyCode::Char('0') => {
            // Go to first column
            state.cursor_col = 0;
            state.col_offset = 0;
        }
        KeyCode::Char('$') => {
            // Go to last column
            state.cursor_col = (cols - 1) as usize;
            if state.cursor_col >= state.col_offset + 20 {
                state.col_offset = state.cursor_col.saturating_sub(10);
            }
        }
        KeyCode::Char('g') => {
            // Go to first row
            state.cursor_row = 0;
            state.row_offset = 0;
        }
        KeyCode::Char('f') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            // Page down
            let visible_rows = 20;
            state.row_offset = (state.row_offset + visible_rows).min(rows as usize - visible_rows);
            state.cursor_row = state.row_offset;
        }
        KeyCode::Char('b') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            // Page up
            state.row_offset = state.row_offset.saturating_sub(20);
            state.cursor_row = state.row_offset;
        }
        _ => {}
    }

    Ok(true)
}

/// Handles keyboard input in insert mode.
fn handle_insert_mode(
    key: KeyEvent,
    state: &mut EditorState,
    sheet: &mut Vec<Vec<cell>>,
    rows: i32,
    cols: i32,
) -> io::Result<bool> {
    match key.code {
        KeyCode::Esc => {
            // Apply the edit and return to normal mode
            state.apply_edit(sheet);
            state.enter_normal_mode();

            // Create a temporary graph for dependency tracking
            let total_cells = (rows * cols) as usize;
            let mut graph = Vec::with_capacity(total_cells);
            for _ in 0..total_cells {
                graph.push(Some(Box::new(crate::DAGNode {
                    in_degree: 0,
                    dependents: HashSet::new(),
                    dependencies: HashSet::new(),
                })));
            }

            // Evaluate cells after editing
            let mut evaluated = vec![false; total_cells];
            for r in 0..rows {
                for c in 0..cols {
                    if sheet[r as usize][c as usize].formula.is_some() {
                        evaluate_cell(r, c, sheet, &graph, &mut evaluated, rows, cols);
                    }
                }
            }
        }
        KeyCode::Char(c) => {
            state.edit_buffer.push(c);
        }
        KeyCode::Backspace => {
            state.edit_buffer.pop();
        }
        KeyCode::Enter => {
            // Apply the edit, move to the next row, and stay in insert mode
            state.apply_edit(sheet);

            // Create a temporary graph for dependency tracking
            let total_cells = (rows * cols) as usize;
            let mut graph = Vec::with_capacity(total_cells);
            for _ in 0..total_cells {
                graph.push(Some(Box::new(crate::DAGNode {
                    in_degree: 0,
                    dependents: HashSet::new(),
                    dependencies: HashSet::new(),
                })));
            }

            // Evaluate cells after editing
            let mut evaluated = vec![false; total_cells];
            for r in 0..rows {
                for c in 0..cols {
                    if sheet[r as usize][c as usize].formula.is_some() {
                        evaluate_cell(r, c, sheet, &graph, &mut evaluated, rows, cols);
                    }
                }
            }

            if state.cursor_row + 1 < rows as usize {
                state.cursor_row += 1;
                if state.cursor_row >= state.row_offset + 20 {
                    state.row_offset += 1;
                }
                state.enter_insert_mode(sheet);
            } else {
                state.enter_normal_mode();
            }
        }
        KeyCode::Tab => {
            // Apply the edit, move to the next column, and stay in insert mode
            state.apply_edit(sheet);

            // Create a temporary graph for dependency tracking
            let total_cells = (rows * cols) as usize;
            let mut graph = Vec::with_capacity(total_cells);
            for _ in 0..total_cells {
                graph.push(Some(Box::new(crate::DAGNode {
                    in_degree: 0,
                    dependents: HashSet::new(),
                    dependencies: HashSet::new(),
                })));
            }

            // Evaluate cells after editing
            let mut evaluated = vec![false; total_cells];
            for r in 0..rows {
                for c in 0..cols {
                    if sheet[r as usize][c as usize].formula.is_some() {
                        evaluate_cell(r, c, sheet, &graph, &mut evaluated, rows, cols);
                    }
                }
            }

            if state.cursor_col + 1 < cols as usize {
                state.cursor_col += 1;
                if state.cursor_col >= state.col_offset + 20 {
                    state.col_offset += 1;
                }
                state.enter_insert_mode(sheet);
            } else {
                state.enter_normal_mode();
            }
        }
        _ => {}
    }

    Ok(true)
}

/// Handles keyboard input in command mode.
fn handle_command_mode(
    key: KeyEvent,
    state: &mut EditorState,
    sheet: &mut Vec<Vec<cell>>,
    rows: i32,
    cols: i32,
) -> io::Result<bool> {
    match key.code {
        KeyCode::Esc => {
            state.enter_normal_mode();
        }
        KeyCode::Char(c) => {
            state.command_buffer.push(c);
        }
        KeyCode::Backspace => {
            if state.command_buffer.len() > 1 {
                // Keep the initial ':'
                state.command_buffer.pop();
            }
        }
        KeyCode::Enter => {
            let command = state.command_buffer.clone();
            state.enter_normal_mode();

            match execute_command(&command, state, sheet, rows, cols) {
                Ok(()) => {
                    // Create a temporary graph for dependency tracking
                    let total_cells = (rows * cols) as usize;
                    let mut graph = Vec::with_capacity(total_cells);
                    for _ in 0..total_cells {
                        graph.push(Some(Box::new(crate::DAGNode {
                            in_degree: 0,
                            dependents: HashSet::new(),
                            dependencies: HashSet::new(),
                        })));
                    }

                    // Evaluate cells after command execution
                    let mut evaluated = vec![false; total_cells];
                    for r in 0..rows {
                        for c in 0..cols {
                            if sheet[r as usize][c as usize].formula.is_some() {
                                evaluate_cell(r, c, sheet, &graph, &mut evaluated, rows, cols);
                            }
                        }
                    }
                }
                Err(message) => {
                    if message == "quit" {
                        return Ok(false); // Exit the application
                    }
                    state.status_message = message;
                }
            }
        }
        _ => {}
    }

    Ok(true)
}

/// Handles keyboard input in visual mode.
fn handle_visual_mode(
    key: KeyEvent,
    state: &mut EditorState,
    sheet: &mut Vec<Vec<cell>>,
    rows: i32,
    cols: i32,
) -> io::Result<bool> {
    match key.code {
        KeyCode::Esc => {
            state.enter_normal_mode();
        }
        KeyCode::Char('h') => {
            if state.cursor_col > 0 {
                state.cursor_col -= 1;
                if state.cursor_col < state.col_offset {
                    state.col_offset = state.cursor_col;
                }
            }
        }
        KeyCode::Char('j') => {
            if state.cursor_row + 1 < rows as usize {
                state.cursor_row += 1;
                if state.cursor_row >= state.row_offset + 20 {
                    state.row_offset += 1;
                }
            }
        }
        KeyCode::Char('k') => {
            if state.cursor_row > 0 {
                state.cursor_row -= 1;
                if state.cursor_row < state.row_offset {
                    state.row_offset = state.cursor_row;
                }
            }
        }
        KeyCode::Char('l') => {
            if state.cursor_col + 1 < cols as usize {
                state.cursor_col += 1;
                if state.cursor_col >= state.col_offset + 20 {
                    state.col_offset += 1;
                }
            }
        }
        KeyCode::Char('y') => {
            // Yank (copy) selected range
            if let Some((min_row, min_col, max_row, max_col)) = state.get_visual_selection() {
                let height = max_row - min_row + 1;
                let width = max_col - min_col + 1;

                let mut data = Vec::with_capacity(height);
                for i in 0..height {
                    let mut row_data = Vec::with_capacity(width);
                    for j in 0..width {
                        let formula_str =
                            if let Some(ref formula) = sheet[min_row + i][min_col + j].formula {
                                formula_to_string(formula)
                            } else {
                                sheet[min_row + i][min_col + j].val.to_string()
                            };
                        row_data.push(formula_str);
                    }
                    data.push(row_data);
                }

                state.clipboard = Some(crate::vim_mode::editor::ClipboardContent::Range {
                    start_row: min_row,
                    start_col: min_col,
                    end_row: max_row,
                    end_col: max_col,
                    data,
                });

                state.status_message = format!(
                    "Yanked range {}{}:{}{}",
                    column_name(min_col),
                    min_row + 1,
                    column_name(max_col),
                    max_row + 1
                );

                state.enter_normal_mode();
            }
        }
        KeyCode::Char('d') => {
            // Delete selected range
            if let Some((min_row, min_col, max_row, max_col)) = state.get_visual_selection() {
                for i in min_row..=max_row {
                    for j in min_col..=max_col {
                        sheet[i][j].formula = None;
                        sheet[i][j].val = 0;
                        sheet[i][j].err = 0;
                    }
                }

                state.status_message = format!(
                    "Deleted range {}{}:{}{}",
                    column_name(min_col),
                    min_row + 1,
                    column_name(max_col),
                    max_row + 1
                );
                state.enter_normal_mode();

                // Create a temporary graph for dependency tracking
                let total_cells = (rows * cols) as usize;
                let mut graph = Vec::with_capacity(total_cells);
                for _ in 0..total_cells {
                    graph.push(Some(Box::new(crate::DAGNode {
                        in_degree: 0,
                        dependents: HashSet::new(),
                        dependencies: HashSet::new(),
                    })));
                }

                // Evaluate cells after deletion
                let mut evaluated = vec![false; total_cells];
                for r in 0..rows {
                    for c in 0..cols {
                        if sheet[r as usize][c as usize].formula.is_some() {
                            evaluate_cell(r, c, sheet, &graph, &mut evaluated, rows, cols);
                        }
                    }
                }
            }
        }
        _ => {}
    }

    Ok(true)
}
