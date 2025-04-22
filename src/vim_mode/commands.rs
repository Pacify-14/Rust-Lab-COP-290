//! Command handling for the Vim-like interface.

use regex::Regex;
use std::fs::File;
use std::io::{self, BufRead, BufReader, Write};
use std::path::Path;

use crate::vim_mode::editor::{EditorState, column_name, parse_cell_reference};
use crate::{cell, get_col_index};

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
    } else if cmd.starts_with("e ") || cmd.starts_with("edit ") {
        let parts: Vec<&str> = cmd.splitn(2, ' ').collect();
        if let Some(filename) = parts.get(1) {
            match load_file(sheet, filename, rows, cols) {
                Ok(_) => return Ok(()),
                Err(e) => return Err(format!("Error loading: {}", e)),
            }
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
                writeln!(file, "{}={}", cell_ref, formula)?;
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
                sheet[row][col].formula = Some(formula_or_value.to_string());
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
                if formula.contains(search) {
                    let new_formula = if global {
                        formula.replace(search, replace)
                    } else {
                        formula.replacen(search, replace, 1)
                    };

                    if new_formula != *formula {
                        sheet[row as usize][col as usize].formula = Some(new_formula);
                        count += 1;
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

        for i in start..=end {
            if i > rows as usize {
                break;
            }

            // Replace 'i' in the formula with the current index
            let formula = formula_template.replace("i", &i.to_string());

            // Set the formula for the cell
            sheet[(i - 1) as usize][col].formula = Some(formula);
        }

        return Ok(());
    }

    Err("Invalid batch formula syntax. Use format: i in 1..10: Ai = Bi + 1".to_string())
}