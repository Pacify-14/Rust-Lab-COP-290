//! Command handling for the Vim-like interface.

use regex::Regex;
use std::fs::File;
use std::io::{self, BufRead, BufReader, Write};
use std::path::Path;

use crate::vim_mode::editor::{EditorState, column_name, formula_to_string, parse_cell_reference};
use crate::{CellRef, Formula, HashSet, cell, evaluate_cell, get_col_index, parse_formula};

/// Executes a command entered in command mode.
pub fn execute_command(
    command: &str,
    state: &mut EditorState,
    sheet: &mut Vec<Vec<cell>>,
    rows: i32,
    cols: i32,
) -> Result<(), String> {
    let cmd = command.trim_start_matches(':');

    // Quit commands
    if cmd == "q" || cmd == "quit" {
        return Err("quit".to_string()); // Signal to quit
    } else if cmd == "wq" {
        // Save and quit
        if let Err(e) = save_default(sheet, rows, cols) {
            return Err(format!("Error saving: {}", e));
        }
        return Err("quit".to_string()); // Signal to quit
    }
    // File operations
    else if cmd.starts_with("w ") || cmd == "w" {
        let parts: Vec<&str> = cmd.splitn(2, ' ').collect();
        let filename = parts.get(1).map(|s| *s).unwrap_or("spreadsheet.ss");

        match save_file(sheet, filename, rows, cols) {
            Ok(_) => return Ok(()),
            Err(e) => return Err(format!("Error saving: {}", e)),
        }
    } else if command.starts_with("e ") || command == "e" {
        // Edit command - clear sheet and load a file
        let current_dir = std::env::current_dir()
            .map_err(|e| format!("Failed to get current directory: {}", e))?;
        state.status_message = format!("Current directory: {}", current_dir.display());
        let filename = if command == "e" {
            return Err("No filename specified".to_string());
        } else {
            &command[2..]
        };

        // Clear the current sheet
        for row in 0..rows as usize {
            for col in 0..cols as usize {
                sheet[row][col].formula = None;
                sheet[row][col].val = 0;
                sheet[row][col].err = 0;
            }
        }

        // Load the specified file using load_file instead of load_sheet
        match load_file(sheet, filename, rows, cols) {
            Ok(_) => {
                state.reset_view();
                state.status_message = format!("Loaded file: {}", filename);

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

                // Evaluate cells after loading
                let mut evaluated = vec![false; total_cells];
                for r in 0..rows {
                    for c in 0..cols {
                        if sheet[r as usize][c as usize].formula.is_some() {
                            evaluate_cell(r, c, sheet, &graph, &mut evaluated, rows, cols);
                        }
                    }
                }

                return Ok(());
            }
            Err(e) => return Err(format!("Error loading file: {}", e)),
        }
    }
    // Search and replace
    else if cmd.starts_with("s/") {
        let parts: Vec<&str> = cmd[2..].splitn(3, '/').collect();
        if parts.len() >= 2 {
            let search = parts[0];
            let replace = parts[1];
            let global = parts.get(2).map(|s| *s == "g").unwrap_or(false);

            let count = search_and_replace(sheet, search, replace, global, rows, cols);
            return Ok(());
        }
    }
    // Jump to cell
    else if let Some((row, col)) = parse_cell_reference(cmd) {
        if row < rows as usize && col < cols as usize {
            state.cursor_row = row;
            state.cursor_col = col;

            // Adjust viewport if necessary
            if row < state.row_offset || row >= state.row_offset + 20 {
                state.row_offset = row.saturating_sub(5);
            }

            if col < state.col_offset || col >= state.col_offset + 20 {
                state.col_offset = col.saturating_sub(5);
            }

            return Ok(());
        } else {
            return Err(format!("Cell reference out of bounds: {}", cmd));
        }
    }
    // Batch formula assignment
    else if cmd.starts_with("i in ") {
        return execute_batch_formula(cmd, sheet, rows, cols);
    }
    // Help command
    else if cmd == "help" {
        return Err("Available commands:\n:q, :quit - Quit\n:w [filename] - Save\n:wq - Save and quit\n:e, :edit [filename] - Open file\n:s/search/replace[/g] - Search and replace\n:A1 - Jump to cell\n:i in range: formula - Batch formula assignment".to_string());
    } else if cmd.starts_with("%s/") {
        return search_and_replace_vim(state, sheet, cmd, rows, cols);
    } else if command.starts_with('/') {
        let pattern = &command[1..];
        return search_sheet(state, sheet, pattern, true, rows, cols);
    } else if command.starts_with('?') {
        let pattern = &command[1..];
        return search_sheet(state, sheet, pattern, false, rows, cols);
    } else if cmd == "n" {
        if state.search_pattern.is_none() {
            return Err("No previous search".to_string());
        }

        if !find_next_match(state, state.search_forward) {
            return Err("Pattern not found".to_string());
        }

        return Ok(());
    } else if cmd == "N" {
        if state.search_pattern.is_none() {
            return Err("No previous search".to_string());
        }

        if !find_next_match(state, !state.search_forward) {
            return Err("Pattern not found".to_string());
        }

        return Ok(());
    }

    Err(format!("Unknown command: {}", cmd))
}

/// Saves the spreadsheet to a file.
fn save_file(sheet: &Vec<Vec<cell>>, filename: &str, rows: i32, cols: i32) -> io::Result<()> {
    let path = Path::new(filename);
    let extension = path.extension().and_then(|s| s.to_str()).unwrap_or("");

    match extension {
        "csv" => save_as_csv(sheet, filename, rows, cols),
        "tsv" => save_as_tsv(sheet, filename, rows, cols),
        "ss" | _ => save_as_custom(sheet, filename, rows, cols),
    }
}

/// Saves the spreadsheet to the default file (spreadsheet.ss).
fn save_default(sheet: &Vec<Vec<cell>>, rows: i32, cols: i32) -> io::Result<()> {
    save_as_custom(sheet, "spreadsheet.ss", rows, cols)
}

/// Saves the spreadsheet as a CSV file.
fn save_as_csv(sheet: &Vec<Vec<cell>>, filename: &str, rows: i32, cols: i32) -> io::Result<()> {
    let mut file = File::create(filename)?;

    for row in 0..rows {
        let mut line = String::new();

        for col in 0..cols {
            if col > 0 {
                line.push(',');
            }

            if sheet[row as usize][col as usize].err != 0 {
                line.push_str("ERR");
            } else {
                line.push_str(&sheet[row as usize][col as usize].val.to_string());
            }
        }

        writeln!(file, "{}", line)?;
    }

    Ok(())
}

/// Saves the spreadsheet as a TSV file.
fn save_as_tsv(sheet: &Vec<Vec<cell>>, filename: &str, rows: i32, cols: i32) -> io::Result<()> {
    let mut file = File::create(filename)?;

    for row in 0..rows {
        let mut line = String::new();

        for col in 0..cols {
            if col > 0 {
                line.push('\t');
            }

            if sheet[row as usize][col as usize].err != 0 {
                line.push_str("ERR");
            } else {
                line.push_str(&sheet[row as usize][col as usize].val.to_string());
            }
        }

        writeln!(file, "{}", line)?;
    }

    Ok(())
}

/// Saves the spreadsheet in a custom format that preserves formulas.
fn save_as_custom(sheet: &Vec<Vec<cell>>, filename: &str, rows: i32, cols: i32) -> io::Result<()> {
    let mut file = File::create(filename)?;

    // Write header with dimensions
    writeln!(file, "ROWS:{} COLS:{}", rows, cols)?;

    // Write cell data with formulas
    for row in 0..rows {
        for col in 0..cols {
            let cell_ref = format!("{}{}", column_name(col as usize), row + 1);

            if let Some(ref formula) = sheet[row as usize][col as usize].formula {
                writeln!(file, "{}={}", cell_ref, formula_to_string(formula))?;
            } else if sheet[row as usize][col as usize].val != 0 {
                // Only write non-zero cells without formulas
                writeln!(
                    file,
                    "{}={}",
                    cell_ref, sheet[row as usize][col as usize].val
                )?;
            }
        }
    }

    Ok(())
}

/// Loads a spreadsheet from a file.
fn load_file(sheet: &mut Vec<Vec<cell>>, filename: &str, rows: i32, cols: i32) -> io::Result<()> {
    let path = Path::new(filename);
    let extension = path.extension().and_then(|s| s.to_str()).unwrap_or("");

    // Reset the sheet
    for row in 0..rows {
        for col in 0..cols {
            sheet[row as usize][col as usize].val = 0;
            sheet[row as usize][col as usize].formula = None;
            sheet[row as usize][col as usize].err = 0;
        }
    }

    match extension {
        "csv" => load_from_csv(sheet, filename, rows, cols),
        "tsv" => load_from_tsv(sheet, filename, rows, cols),
        "ss" => load_from_custom(sheet, filename, rows, cols),
        _ => {
            // Try to guess format based on content
            let file = File::open(filename)?;
            let reader = BufReader::new(file);
            let first_line = reader
                .lines()
                .next()
                .ok_or(io::Error::new(io::ErrorKind::InvalidData, "Empty file"))??;

            if first_line.contains('\t') {
                load_from_tsv(sheet, filename, rows, cols)
            } else if first_line.contains(',') {
                load_from_csv(sheet, filename, rows, cols)
            } else if first_line.starts_with("ROWS:") {
                load_from_custom(sheet, filename, rows, cols)
            } else {
                Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Unknown file format",
                ))
            }
        }
    }
}

/// Loads a spreadsheet from a CSV file.
fn load_from_csv(
    sheet: &mut Vec<Vec<cell>>,
    filename: &str,
    rows: i32,
    cols: i32,
) -> io::Result<()> {
    let file = File::open(filename)?;
    let reader = BufReader::new(file);

    for (row_idx, line_result) in reader.lines().enumerate() {
        if row_idx >= rows as usize {
            break;
        }

        let line = line_result?;
        let values: Vec<&str> = line.split(',').collect();

        for (col_idx, value) in values.iter().enumerate() {
            if col_idx >= cols as usize {
                break;
            }

            if let Ok(val) = value.parse::<i32>() {
                sheet[row_idx][col_idx].val = val;
                sheet[row_idx][col_idx].formula = None;
            } else if *value == "ERR" {
                sheet[row_idx][col_idx].err = 1;
            }
        }
    }

    Ok(())
}

/// Loads a spreadsheet from a TSV file.
fn load_from_tsv(
    sheet: &mut Vec<Vec<cell>>,
    filename: &str,
    rows: i32,
    cols: i32,
) -> io::Result<()> {
    let file = File::open(filename)?;
    let reader = BufReader::new(file);

    for (row_idx, line_result) in reader.lines().enumerate() {
        if row_idx >= rows as usize {
            break;
        }

        let line = line_result?;
        let values: Vec<&str> = line.split('\t').collect();

        for (col_idx, value) in values.iter().enumerate() {
            if col_idx >= cols as usize {
                break;
            }

            if let Ok(val) = value.parse::<i32>() {
                sheet[row_idx][col_idx].val = val;
                sheet[row_idx][col_idx].formula = None;
            } else if *value == "ERR" {
                sheet[row_idx][col_idx].err = 1;
            }
        }
    }

    Ok(())
}

/// Loads a spreadsheet from a custom format file.
fn load_from_custom(
    sheet: &mut Vec<Vec<cell>>,
    filename: &str,
    rows: i32,
    cols: i32,
) -> io::Result<()> {
    let file = File::open(filename)?;
    let reader = BufReader::new(file);
    let mut lines = reader.lines();

    // Read header
    if let Some(header_result) = lines.next() {
        let header = header_result?;
        if !header.starts_with("ROWS:") {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Invalid custom format: missing header",
            ));
        }
    }

    // Read cell data
    for line_result in lines {
        let line = line_result?;

        // Parse cell reference and formula/value
        let parts: Vec<&str> = line.splitn(2, '=').collect();
        if parts.len() != 2 {
            continue;
        }

        let cell_ref = parts[0];
        let formula_or_value = parts[1];

        if let Some((row, col)) = parse_cell_reference(cell_ref) {
            if row >= rows as usize || col >= cols as usize {
                continue;
            }

            if let Ok(val) = formula_or_value.parse::<i32>() {
                sheet[row][col].val = val;
                sheet[row][col].formula = None;
            } else {
                // Try to parse the formula string into a Formula AST
                match parse_formula(formula_or_value) {
                    Ok(formula_ast) => {
                        sheet[row][col].formula = Some(formula_ast);
                    }
                    Err(_) => {
                        // If parsing fails, store as a literal value of 0 with an error
                        sheet[row][col].val = 0;
                        sheet[row][col].err = 1;
                    }
                }
            }
        }
    }

    Ok(())
}

/// Performs search and replace on cell formulas.
fn search_and_replace(
    sheet: &mut Vec<Vec<cell>>,
    search: &str,
    replace: &str,
    global: bool,
    rows: i32,
    cols: i32,
) -> usize {
    let mut count = 0;

    for row in 0..rows {
        for col in 0..cols {
            if let Some(ref formula) = sheet[row as usize][col as usize].formula {
                let formula_str = formula_to_string(formula);
                if formula_str.contains(search) {
                    let new_formula_str = if global {
                        formula_str.replace(search, replace)
                    } else {
                        formula_str.replacen(search, replace, 1)
                    };

                    if new_formula_str != formula_str {
                        // Try to parse the new formula string
                        match parse_formula(&new_formula_str) {
                            Ok(new_formula) => {
                                sheet[row as usize][col as usize].formula = Some(new_formula);
                                count += 1;
                            }
                            Err(_) => {
                                // If parsing fails, leave the formula unchanged
                                // Optionally, you could set an error flag here
                            }
                        }
                    }
                }
            }
        }
    }

    count
}

/// Executes a batch formula assignment command.
fn execute_batch_formula(
    cmd: &str,
    sheet: &mut Vec<Vec<cell>>,
    rows: i32,
    cols: i32,
) -> Result<(), String> {
    // Parse command like ":i in 1..10: Ai = Bi + 1"
    let re = Regex::new(r"i in (\d+)\.\.(\d+): ([A-Z])i = (.+)").unwrap();

    if let Some(caps) = re.captures(cmd) {
        let start: usize = caps[1]
            .parse()
            .map_err(|_| "Invalid range start".to_string())?;
        let end: usize = caps[2]
            .parse()
            .map_err(|_| "Invalid range end".to_string())?;
        let col_letter = &caps[3];
        let formula_template = &caps[4];

        let col = get_col_index(col_letter) as usize;
        if col >= cols as usize {
            return Err(format!("Column {} is out of bounds", col_letter));
        }

        let mut count = 0;
        for i in start..=end {
            if i > rows as usize {
                break;
            }

            // Replace 'i' in the formula with the current index
            let formula_str = formula_template.replace("i", &i.to_string());

            // Try to parse the formula string
            match parse_formula(&formula_str) {
                Ok(formula) => {
                    // Set the formula for the cell
                    sheet[(i - 1) as usize][col].formula = Some(formula);
                    count += 1;
                }
                Err(_) => {
                    return Err(format!("Invalid formula: {}", formula_str));
                }
            }
        }

        return Ok(());
    }

    // Try alternative syntax: "i,j in 1..5,1..5: Ci,j = Ai,j + Bi,j"
    let re2 =
        Regex::new(r"i,j in (\d+)\.\.(\d+),(\d+)\.\.(\d+): ([A-Z])i,([A-Z])j = (.+)").unwrap();
    if let Some(caps) = re2.captures(cmd) {
        let row_start: usize = caps[1]
            .parse()
            .map_err(|_| "Invalid row range start".to_string())?;
        let row_end: usize = caps[2]
            .parse()
            .map_err(|_| "Invalid row range end".to_string())?;
        let col_start: usize = caps[3]
            .parse()
            .map_err(|_| "Invalid column range start".to_string())?;
        let col_end: usize = caps[4]
            .parse()
            .map_err(|_| "Invalid column range end".to_string())?;

        let target_row_col = &caps[5];
        let target_col_col = &caps[6];
        let formula_template = &caps[7];

        let mut count = 0;
        for i in row_start..=row_end {
            if i > rows as usize {
                break;
            }

            for j in col_start..=col_end {
                // Calculate target cell coordinates
                let row_idx = i - 1; // Convert to 0-indexed

                // For the column, we need to replace 'i' in the column letter if present
                let col_str = if target_col_col.contains('i') {
                    target_col_col.replace('i', &i.to_string())
                } else {
                    format!("{}{}", target_col_col, j)
                };

                let col_idx = match parse_cell_reference(&format!("{}{}", col_str, 1)) {
                    Some((_, col)) => col,
                    None => continue, // Skip invalid column
                };

                if row_idx >= rows as usize || col_idx >= cols as usize {
                    continue; // Skip out of bounds
                }

                // Replace 'i' and 'j' in the formula
                let formula_str = formula_template
                    .replace("i", &i.to_string())
                    .replace("j", &j.to_string());

                // Try to parse the formula string
                match parse_formula(&formula_str) {
                    Ok(formula) => {
                        // Set the formula for the cell
                        sheet[row_idx][col_idx].formula = Some(formula);
                        count += 1;
                    }
                    Err(_) => {
                        return Err(format!("Invalid formula: {}", formula_str));
                    }
                }
            }
        }

        if count > 0 {
            return Ok(());
        }
    }

    Err("Invalid batch formula syntax. Use format: i in 1..10: Ai = Bi + 1".to_string())
}

fn save_sheet(filename: &str, sheet: &Vec<Vec<cell>>, rows: i32, cols: i32) -> Result<(), String> {
    use std::fs::File;
    use std::io::{self, Write};

    let file = File::create(filename).map_err(|e| format!("Failed to create file: {}", e))?;
    let mut writer = io::BufWriter::new(file);

    for row in 0..rows as usize {
        for col in 0..cols as usize {
            let cell_value = if let Some(ref formula) = sheet[row][col].formula {
                format!("\"={}\"", formula_to_string(formula))
            } else {
                sheet[row][col].val.to_string()
            };

            if col > 0 {
                write!(writer, ",").map_err(|e| format!("Failed to write to file: {}", e))?;
            }
            write!(writer, "{}", cell_value)
                .map_err(|e| format!("Failed to write to file: {}", e))?;
        }
        writeln!(writer).map_err(|e| format!("Failed to write to file: {}", e))?;
    }

    Ok(())
}

// Helper function to load a sheet from a CSV file
fn load_sheet(
    filename: &str,
    sheet: &mut Vec<Vec<cell>>,
    rows: i32,
    cols: i32,
) -> Result<(), String> {
    use std::fs::File;
    use std::io::{self, BufRead};

    let file = File::open(filename).map_err(|e| format!("Failed to open file: {}", e))?;
    let reader = io::BufReader::new(file);

    let mut row_idx = 0;
    for line in reader.lines() {
        if row_idx >= rows as usize {
            break;
        }

        let line = line.map_err(|e| format!("Failed to read line: {}", e))?;
        let values = line.split(',').collect::<Vec<&str>>();

        for (col_idx, value) in values.iter().enumerate() {
            if col_idx >= cols as usize {
                break;
            }

            let value = value.trim();
            if value.starts_with("\"=") && value.ends_with("\"") {
                // It's a formula
                let formula_str = value[2..value.len() - 1].to_string();
                match parse_formula(&formula_str) {
                    Ok(formula) => {
                        sheet[row_idx][col_idx].formula = Some(formula);
                        sheet[row_idx][col_idx].err = 0;
                    }
                    Err(_) => {
                        // If parsing fails, set as error
                        sheet[row_idx][col_idx].formula = None;
                        sheet[row_idx][col_idx].val = 0;
                        sheet[row_idx][col_idx].err = 1;
                    }
                }
            } else if let Ok(num) = value.parse::<i32>() {
                // It's a number
                sheet[row_idx][col_idx].formula = None;
                sheet[row_idx][col_idx].val = num;
                sheet[row_idx][col_idx].err = 0;
            } else {
                // Treat as 0 if not parseable
                sheet[row_idx][col_idx].formula = None;
                sheet[row_idx][col_idx].val = 0;
                sheet[row_idx][col_idx].err = 0;
            }
        }

        row_idx += 1;
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

    // Evaluate cells after loading
    let mut evaluated = vec![false; total_cells];
    for r in 0..rows {
        for c in 0..cols {
            if sheet[r as usize][c as usize].formula.is_some() {
                evaluate_cell(r, c, sheet, &graph, &mut evaluated, rows, cols);
            }
        }
    }

    Ok(())
}

pub fn search_sheet(
    state: &mut EditorState,
    sheet: &Vec<Vec<cell>>,
    pattern: &str,
    forward: bool,
    rows: i32,
    cols: i32,
) -> Result<(), String> {
    if pattern.is_empty() {
        return Err("Empty search pattern".to_string());
    }

    state.search_pattern = Some(pattern.to_string());
    state.search_forward = forward;
    state.search_matches.clear();
    state.current_match = None;

    // Find all matches in the sheet
    for row in 0..(rows as usize) {
        for col in 0..(cols as usize) {
            let cell_value = if let Some(ref formula) = sheet[row][col].formula {
                formula_to_string(formula)
            } else {
                sheet[row][col].val.to_string()
            };

            if cell_value.contains(pattern) {
                state.search_matches.push((row, col));
            }
        }
    }

    if state.search_matches.is_empty() {
        return Err(format!("Pattern not found: {}", pattern));
    }

    // Find the first match based on search direction and current cursor
    find_next_match(state, forward);

    Ok(())
}

/// Finds the next or previous match based on search direction
pub fn find_next_match(state: &mut EditorState, forward: bool) -> bool {
    if state.search_matches.is_empty() {
        return false;
    }

    let (current_row, current_col) = (state.cursor_row, state.cursor_col);

    if forward {
        // Find the next match after current position
        let next_match = state.search_matches.iter().position(|&(row, col)| {
            (row > current_row) || (row == current_row && col > current_col)
        });

        if let Some(idx) = next_match {
            state.current_match = Some(idx);
        } else if !state.search_matches.is_empty() {
            // Wrap around to the first match
            state.current_match = Some(0);
        }
    } else {
        // Find the previous match before current position
        let prev_matches: Vec<_> = state
            .search_matches
            .iter()
            .enumerate()
            .filter(|&(_, &(row, col))| {
                (row < current_row) || (row == current_row && col < current_col)
            })
            .collect();

        if !prev_matches.is_empty() {
            // Get the last match before current position
            state.current_match = Some(prev_matches.last().unwrap().0);
        } else if !state.search_matches.is_empty() {
            // Wrap around to the last match
            state.current_match = Some(state.search_matches.len() - 1);
        }
    }

    if let Some(idx) = state.current_match {
        let (row, col) = state.search_matches[idx];
        state.cursor_row = row;
        state.cursor_col = col;

        // Ensure the cursor is visible
        if state.cursor_row < state.row_offset {
            state.row_offset = state.cursor_row;
        } else if state.cursor_row >= state.row_offset + 20 {
            // Assuming visible_rows = 20
            state.row_offset = state.cursor_row.saturating_sub(19);
        }

        if state.cursor_col < state.col_offset {
            state.col_offset = state.cursor_col;
        } else if state.cursor_col >= state.col_offset + 10 {
            // Assuming visible_cols = 10
            state.col_offset = state.cursor_col.saturating_sub(9);
        }

        return true;
    }

    false
}

pub fn search_and_replace_vim(
    state: &mut EditorState,
    sheet: &mut Vec<Vec<cell>>,
    command: &str,
    rows: i32,
    cols: i32,
) -> Result<(), String> {
    // Parse the search and replace command (:%s/pattern/replacement/[g])
    let parts: Vec<&str> = command.split('/').collect();
    if parts.len() < 3 {
        return Err(
            "Invalid search and replace format. Use :%s/pattern/replacement/[g]".to_string(),
        );
    }

    let pattern = parts[1];
    let replacement = parts[2];
    let global = parts.len() > 3 && parts[3].contains('g');

    if pattern.is_empty() {
        return Err("Empty search pattern".to_string());
    }

    let mut count = 0;

    // Perform the replacement
    for row in 0..(rows as usize) {
        for col in 0..(cols as usize) {
            if let Some(ref formula) = sheet[row][col].formula.clone() {
                let formula_str = formula_to_string(formula);
                if formula_str.contains(pattern) {
                    let new_formula_str = if global {
                        formula_str.replace(pattern, replacement)
                    } else {
                        formula_str.replacen(pattern, replacement, 1)
                    };

                    if new_formula_str != formula_str {
                        // Try to parse the new formula
                        match parse_formula(&new_formula_str) {
                            Ok(new_formula) => {
                                sheet[row][col].formula = Some(new_formula);
                                count += 1;
                            }
                            Err(_) => {
                                // If parsing fails, leave the formula unchanged
                            }
                        }
                    }
                }
            }
        }
    }

    if count == 0 {
        return Err(format!("Pattern not found: {}", pattern));
    }

    state.status_message = format!("Replaced {} occurrences", count);
    Ok(())
}
