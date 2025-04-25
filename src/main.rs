#![allow(warnings)]

//! # Spreadsheet Engine
//!
//! This crate implements a minimal spreadsheet engine supporting formulas, arithmetic, ranges, dependencies, and cycle detection.
//!
//! ## Features
//! - Parse and evaluate formulas (literals, cell references, arithmetic, ranges, SLEEP).
//! - Dependency tracking and cycle detection via a DAG.
//! - Viewport navigation and output control.
//! - Extensive test coverage.

use once_cell::sync::Lazy;
use regex::Regex;
use std::cell::Ref;
use std::cell::RefCell;
use std::collections::HashSet;
use std::collections::VecDeque;
use std::env;
use std::io::{self, BufRead, Write};
use std::process;
use std::str;
use std::thread_local;
use std::time::Instant;

use libc;
/// An AST for all the ways you can compute a cell
/// Your AST for formulas

/// Represents all possible formulas that can be assigned to a spreadsheet cell.
///
/// This enum forms the Abstract Syntax Tree (AST) for formulas, supporting:
/// - Literal values
/// - Cell references
/// - Arithmetic operations
/// - Range functions (SUM, AVG, MIN, MAX, STDEV)
/// - SLEEP operations
#[derive(Clone, Debug)]
pub enum Formula {
    /// A literal integer value (e.g., `42`)
    Literal(i32),
    /// Reference to another cell (e.g., `A1`)
    Cell(CellRef),
    /// Optimized increment: a cell plus an integer offset (e.g., `A1+3`)
    Inc { base: CellRef, offset: i32 },
    /// Arithmetic operation between two formulas (e.g., `A1 + B2`)
    Arith {
        op: Op,
        left: Box<Formula>,
        right: Box<Formula>,
    },
    /// Range function over a rectangular region (e.g., `SUM(A1:B2)`)
    Range {
        func: String,
        start: CellRef,
        end: CellRef,
    },
    /// SLEEP operation with a literal duration
    SleepLiteral(i32),
    /// SLEEP operation with duration from another cell
    SleepCell(CellRef),
}

/// Supported arithmetic operators for formulas.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    Add,
    Sub,
    Mul,
    Div,
}

/// Parses a formula string into an AST.
///
/// # Arguments
///
/// * `formula` - The formula string (e.g., `"A1+3"`, `"SUM(A1:B2)"`)
///
/// # Returns
///
/// * `Ok(Formula)` if parsing succeeds.
/// * `Err(())` if parsing fails.
///
/// # Examples
///
/// ```
/// let f = parse_formula("A1+2").unwrap();
/// ```
/// Parse & turn a string‐formula into an AST
fn parse_formula(formula: &str) -> Result<Formula, ()> {
    let f = formula.trim();

    // 1) Plain integer
    if let Ok(n) = f.parse::<i32>() {
        return Ok(Formula::Literal(n));
    }

    // 2) Bare cell reference
    if let Some((r, c)) = parse_cell_ref(f) {
        return Ok(Formula::Cell(CellRef { row: r, col: c }));
    }

    // 3) Arithmetic: e.g. "A1+1" or "3 * B2"
    if let Some(caps) = RE_ARITH.captures(f) {
        let left_str = &caps[1];
        let op_str = &caps[2];
        let right_str = &caps[3];

        // recursively build sub‐ASTs
        let left = parse_formula(left_str).map_err(|_| ())?;
        let right = parse_formula(right_str).map_err(|_| ())?;
        let op = match op_str {
            "+" => Op::Add,
            "-" => Op::Sub,
            "*" => Op::Mul,
            "/" => Op::Div,
            _ => return Err(()),
        };

        // INC optimization: Cell + Literal
        if op == Op::Add {
            if let (Formula::Cell(base), Formula::Literal(off)) = (&left, &right) {
                return Ok(Formula::Inc {
                    base: *base,
                    offset: *off,
                });
            }
        }

        return Ok(Formula::Arith {
            op,
            left: Box::new(left),
            right: Box::new(right),
        });
    }

    // 4) Range functions: SUM(A1:B2), AVG(...), MIN(...), MAX(...), STDEV(...)
    if let Some(caps) = RE_RANGE_FUNC.captures(f) {
        let func_name = &caps[1]; // e.g. "MAX"
        let range_str = &caps[2]; // e.g. "A1:B3"
        let parts: Vec<&str> = range_str.split(':').collect();
        if parts.len() == 2 {
            if let (Some((sr, sc)), Some((er, ec))) =
                (parse_cell_ref(parts[0]), parse_cell_ref(parts[1]))
            {
                return Ok(Formula::Range {
                    func: func_name.to_string(),
                    start: CellRef { row: sr, col: sc },
                    end: CellRef { row: er, col: ec },
                });
            }
        }
        return Err(());
    }

    // 5) SLEEP(…)
    if let Some(caps) = RE_SLEEP.captures(f) {
        let inner = &caps[1];
        // either a literal
        if let Ok(n) = inner.parse::<i32>() {
            return Ok(Formula::SleepLiteral(n));
        }
        // or a cell ref
        if let Some((sr, sc)) = parse_cell_ref(inner) {
            return Ok(Formula::SleepCell(CellRef { row: sr, col: sc }));
        }
        return Err(());
    }

    // 6) Nothing matched → error
    Err(())
}

const _CLOCKS_PER_SEC: i64 = 1_000_000;
const VIEWPORT_SIZE: i32 = 10;
static mut inval_r: bool = false;
static mut unrec_cmd: bool = false;
static mut sleeptimetotal: f64 = 0.0;

/// Represents a spreadsheet cell, including its value, formula, and error state.
#[derive(Clone)]
#[allow(non_camel_case_types)]
pub struct cell {
    /// The computed value of the cell
    pub val: i32,
    /// The formula assigned to the cell, if any
    pub formula: Option<Formula>,
    /// Error flag (0 = OK, nonzero = error)
    pub err: i32,
}

/// Represents a cell update event, used for tracking changes.
#[derive(Copy, Clone)]
pub struct CellUpdate {
    /// Row index of the updated cell
    pub row: i32,
    /// Column index of the updated cell
    pub col: i32,
    /// Whether the cell was updated
    pub is_updated: bool,
}

static mut last_update: CellUpdate = CellUpdate {
    row: -1,
    col: -1,
    is_updated: false,
};

/// Represents a reference to a cell by row and column (zero-based).
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct CellRef {
    pub row: i32,
    pub col: i32,
}

pub struct Node {
    pub cell: CellRef,
    pub next: Option<Box<Node>>,
}

/// Node for dependency tracking (DAG).
pub struct DAGNode {
    pub in_degree: i32,
    pub dependents: HashSet<(i32, i32)>, // Replacing linked list
    pub dependencies: HashSet<(i32, i32)>, // Replacing linked list
}

/// Prints the spreadsheet viewport to stdout.
///
/// Only prints if `output_enabled != 0`.
fn print_sheet(
    R: i32,
    C: i32,
    sheet: &Vec<Vec<cell>>,
    row_offset: i32,
    col_offset: i32,
    output_enabled: i32,
) {
    if output_enabled == 0 {
        return;
    }
    print!("  ");
    print_columns(C, col_offset);
    println!("\n");
    for i in row_offset..(row_offset + VIEWPORT_SIZE) {
        if i >= R {
            break;
        }
        print!("{}\t", i + 1);
        for j in col_offset..(col_offset + VIEWPORT_SIZE) {
            if j >= C {
                break;
            }
            if sheet[i as usize][j as usize].err != 0 {
                print!("ERR\t");
            } else {
                print!("{}\t", sheet[i as usize][j as usize].val);
            }
        }
        println!();
    }
}

/// Prints column labels for the current viewport.
fn print_columns(C: i32, col_offset: i32) {
    print!("\t");
    for i in col_offset..(col_offset + VIEWPORT_SIZE) {
        if i >= C {
            break;
        }
        let mut temp = i + 1;
        let mut label = String::new();
        while temp > 0 {
            let rem = (temp - 1) % 26;
            label.insert(0, (b'A' + rem as u8) as char);
            temp = (temp - 1) / 26;
        }
        print!("{}\t", label);
    }
    println!();
}

static RE_ARITH: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^\s*([A-Z]+\d+|\d+)\s*([\+\-\*/])\s*([A-Z]+\d+|\d+)\s*$").unwrap());
static RE_RANGE_FUNC: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^(SUM|AVG|MIN|MAX|STDEV)\(([A-Z]+\d+:[A-Z]+\d+)\)$").unwrap());

static RE_SLEEP: Lazy<Regex> = Lazy::new(|| Regex::new(r"^SLEEP\((\d+|[A-Z]+\d+)\)$").unwrap());
static RE_CELL: Lazy<Regex> = Lazy::new(|| Regex::new(r"^[A-Z]+\d+$").unwrap());

/// Checks if a formula string is valid.
///
/// Returns `true` if the formula can be parsed, `false` otherwise.
fn is_valid_formula(formula: &str) -> bool {
    if RE_ARITH.is_match(formula) {
        return true;
    }
    if RE_RANGE_FUNC.is_match(formula) {
        return true;
    }
    if RE_SLEEP.is_match(formula) {
        return true;
    }
    if RE_CELL.is_match(formula) {
        return true;
    }
    formula.trim().parse::<i32>().is_ok()
}

/// Processes a user input line (command or cell assignment).
///
/// Handles navigation, output control, and formula assignment.
fn process_input(
    input: &str,
    R: i32,
    C: i32,
    sheet: &mut Vec<Vec<cell>>,
    graph: &mut Vec<Option<Box<DAGNode>>>,
    row_offset: &mut i32,
    col_offset: &mut i32,
    output_enabled: &mut i32,
) {
    // Reset per-command flags
    unsafe {
        sleeptimetotal = 0.0;
        inval_r = false;
        unrec_cmd = false;
        last_update.is_updated = false;
    }

    // ========== Command Handling FIRST ==========
    let trimmed = input.trim();
    match trimmed {
        "q" => process::exit(0),
        "w" => {
            *row_offset = (*row_offset - 10).max(0);
            return;
        }
        "s" => {
            *row_offset = (*row_offset + 10).min(R - VIEWPORT_SIZE);
            return;
        }
        "a" => {
            *col_offset = (*col_offset - 10).max(0);
            // Ensure viewport remains valid
            *col_offset = (*col_offset).max(0).min(C - VIEWPORT_SIZE);
            return;
        }
        "d" => {
            *col_offset = (*col_offset + 10).min(C - VIEWPORT_SIZE);
            return;
        }
        "disable_output" => {
            *output_enabled = 0;
            return;
        }
        "enable_output" => {
            *output_enabled = 1;
            return;
        }
        _ if trimmed.starts_with("scroll_to") => {
            let parts: Vec<&str> = trimmed[9..].trim().split_whitespace().collect();
            if !parts.is_empty() {
                let cell_ref = parts[0];
                let col_str: String = cell_ref.chars().take_while(|c| c.is_alphabetic()).collect();
                let row_str: String = cell_ref.chars().skip_while(|c| c.is_alphabetic()).collect();

                if let Ok(row) = row_str.parse::<i32>() {
                    let col_index = get_col_index(&col_str);
                    if col_index >= 0 && col_index < C && row >= 1 && row <= R {
                        // Clamp so that the requested cell is always at the top/left of the viewport,
                        // but if near the end, show as many as possible (even if < VIEWPORT_SIZE)
                        *row_offset = (row - 1).max(0);
                        *col_offset = col_index.max(0);
                    }
                }
            }
            return;
        }
        _ => {} // Proceed to cell assignment logic
    }

    // ========== Cell Assignment Logic ==========
    let mut col_str = String::new();
    let mut chars = input.chars();

    // Extract column letters
    while let Some(c) = chars.clone().next() {
        if c.is_ascii_alphabetic() {
            col_str.push(c);
            chars.next();
        } else {
            break;
        }
    }

    // Split into row=formula parts
    let rest: String = chars.collect();
    let parts: Vec<&str> = rest.split('=').collect();
    if parts.len() != 2 {
        unsafe {
            unrec_cmd = true;
        }
        return;
    }

    // Parse row number
    let row_num = match parts[0].trim().parse::<i32>() {
        Ok(r) => r,
        Err(_) => {
            unsafe {
                unrec_cmd = true;
            }
            return;
        }
    };

    // Validate column and row bounds
    let col_idx = get_col_index(&col_str);
    if col_idx < 0 || col_idx >= C || row_num < 1 || row_num > R {
        unsafe {
            inval_r = true;
        }
        return;
    }

    let r_index = (row_num - 1) as usize;
    let c_index = col_idx as usize;
    let formula_str = parts[1].trim();

    // Check for invalid self-referential ranges
    if check_invalid_range(formula_str, row_num - 1, col_idx) != 0 {
        unsafe {
            inval_r = true;
        }
        return;
    }

    // Parse formula into AST
    match parse_formula(formula_str) {
        Ok(new_ast) => {
            // Remove old dependencies
            if let Some(old_ast) = &sheet[r_index][c_index].formula {
                for dep in get_dependencies_from_formula(old_ast) {
                    remove_dependency(graph, row_num - 1, col_idx, dep.row, dep.col, R, C);
                }
            }

            // Add new dependencies
            let new_deps = get_dependencies_from_formula(&new_ast);
            for dep in &new_deps {
                add_dependency(graph, row_num - 1, col_idx, dep.row, dep.col, R, C);
            }

            // Update cell and flags
            sheet[r_index][c_index].formula = Some(new_ast);
            unsafe {
                last_update.row = row_num - 1;
                last_update.col = col_idx;
                last_update.is_updated = true;
            }
        }
        Err(_) => unsafe {
            unrec_cmd = true;
        },
    }
}

/// Converts a column label (e.g., `"A"`, `"AA"`) to a zero-based column index.
///
/// Returns -1 for invalid input.
fn get_col_index(col: &str) -> i32 {
    let mut index: i32 = 0;
    for c in col.chars() {
        if c >= 'A' && c <= 'Z' {
            index = index * 26 + (c as i32 - 'A' as i32 + 1);
        } else {
            return -1;
        }
    }
    index - 1
}

/// Evaluates a cell and propagates updates to dependents.
///
/// Ensures dependencies are evaluated before the cell itself.
fn evaluate_cell(
    row: i32,
    col: i32,
    sheet: &mut Vec<Vec<cell>>,
    graph: &Vec<Option<Box<DAGNode>>>,
    evaluated: &mut Vec<bool>,
    R: i32,
    C: i32,
) {
    let mut stack = Vec::new();
    stack.push((row, col));
    while let Some((r, c)) = stack.pop() {
        let idx = (r * C + c) as usize;
        if evaluated[idx] {
            continue;
        }

        let mut all_deps_evaluated = true;
        if let Some(ref dag) = graph[idx] {
            for &(dep_row, dep_col) in &dag.dependencies {
                let dep_idx = (dep_row * C + dep_col) as usize;
                if !evaluated[dep_idx] {
                    all_deps_evaluated = false;
                    stack.push((r, c));
                    stack.push((dep_row, dep_col));
                    break;
                }
            }
        }

        if all_deps_evaluated {
            if let Some(ref formula) = sheet[r as usize][c as usize].formula {
                let mut error_flag = 0;
                let val = evaluate_formula(formula, sheet, &mut error_flag);
                sheet[r as usize][c as usize].val = val;
                sheet[r as usize][c as usize].err = error_flag;
            }
            evaluated[idx] = true;

            if let Some(ref dag) = graph[idx] {
                for &(dep_row, dep_col) in &dag.dependents {
                    stack.push((dep_row, dep_col));
                }
            }
        }
    }
}

/// Evaluates a formula and returns its integer value.
///
/// Sets `error_flag` if evaluation fails (e.g., out-of-bounds, division by zero).
///
/// # Arguments
///
/// * `formula` - The formula AST node
/// * `sheet` - The spreadsheet data
/// * `error_flag` - Mutable reference to error flag
///
/// # Returns
///
/// The computed value or 0 on error.
fn evaluate_formula(formula: &Formula, sheet: &Vec<Vec<cell>>, error_flag: &mut i32) -> i32 {
    let rows = sheet.len() as i32;
    let cols = if rows > 0 { sheet[0].len() as i32 } else { 0 };

    match formula {
        // 1) literal number
        Formula::Literal(n) => *n,

        // 2) simple cell lookup with bounds‐ and error‐checking
        Formula::Cell(CellRef { row, col }) => {
            if *row < 0 || *row >= rows || *col < 0 || *col >= cols {
                *error_flag = 1;
                return 0;
            }
            let c = &sheet[*row as usize][*col as usize];
            if c.err != 0 {
                *error_flag = 1;
                0
            } else {
                c.val
            }
        }

        // 3) the optimized “base + offset” case
        Formula::Inc { base, offset } => {
            // reuse the Cell‐case logic for bounds/errors
            let base_val = evaluate_formula(&Formula::Cell(*base), sheet, error_flag);
            base_val + offset
        }

        // 4) full arithmetic
        Formula::Arith { op, left, right } => {
            let l = evaluate_formula(left, sheet, error_flag);
            let r = evaluate_formula(right, sheet, error_flag);
            if *error_flag != 0 {
                return 0;
            }
            match op {
                Op::Add => l.wrapping_add(r),
                Op::Sub => l.wrapping_sub(r),
                Op::Mul => l.wrapping_mul(r),
                Op::Div => {
                    if r == 0 {
                        *error_flag = 1;
                        0
                    } else {
                        l / r
                    }
                }
            }
        }

        // 5) range functions: SUM, AVG, MIN, MAX, STDEV
        Formula::Range { func, start, end } => {
            let sr = start.row;
            let sc = start.col;
            let er = end.row;
            let ec = end.col;

            // bounds on the corners
            if sr < 0 || sc < 0 || er < sr || ec < sc || er >= rows || ec >= cols {
                *error_flag = 1;
                return 0;
            }

            // collect all values, erroring early if any cell has err!=0
            let mut vals = Vec::new();
            for r in sr..=er {
                for c in sc..=ec {
                    let cell = &sheet[r as usize][c as usize];
                    if cell.err != 0 {
                        *error_flag = 1;
                        return 0;
                    }
                    vals.push(cell.val);
                }
            }
            let n = vals.len();
            if n == 0 {
                return 0;
            }

            match func.as_str() {
                "SUM" => vals.iter().sum(),
                "AVG" => vals.iter().sum::<i32>() / (n as i32),
                "MIN" => *vals.iter().min().unwrap(),
                "MAX" => *vals.iter().max().unwrap(),

                "STDEV" => {
                    if n < 2 {
                        0
                    } else {
                        let mean = vals.iter().map(|&v| v as f64).sum::<f64>() / (n as f64);
                        let var = vals
                            .iter()
                            .map(|&v| {
                                let d = v as f64 - mean;
                                d * d
                            })
                            .sum::<f64>()
                            / ((n as f64) - 1.0);
                        var.sqrt().round() as i32
                    }
                }

                _ => {
                    // unknown range function
                    *error_flag = 1;
                    0
                }
            }
        }

        // 6) SLEEP variants – here we just return the “sleep count”,
        //    leaving any actual thread::sleep or timing bookkeeping
        //    to the caller (or you can inline it here if you prefer!)
        Formula::SleepLiteral(n) => *n,

        Formula::SleepCell(cell_ref) => {
            evaluate_formula(&Formula::Cell(*cell_ref), sheet, error_flag)
        }
    }
}

/// Returns a list of all cell dependencies for a given formula.
fn get_dependencies_from_formula(formula: &Formula) -> Vec<CellRef> {
    let mut deps = Vec::new();

    match formula {
        // no dependencies
        Formula::Literal(_) => {}

        // single cell
        Formula::Cell(c) => deps.push(*c),

        // optimized “base + offset”
        Formula::Inc { base, .. } => deps.push(*base),

        // recurse into both sides
        Formula::Arith { left, right, .. } => {
            deps.extend(get_dependencies_from_formula(left));
            deps.extend(get_dependencies_from_formula(right));
        }

        // every cell in the rectangular range
        Formula::Range { start, end, .. } => {
            for r in start.row..=end.row {
                for c in start.col..=end.col {
                    deps.push(CellRef { row: r, col: c });
                }
            }
        }

        // SLEEP(n) has no cell deps
        Formula::SleepLiteral(_) => {}

        // SLEEP(cell) depends on that one cell
        Formula::SleepCell(c) => deps.push(*c),
    }

    deps
}

/// Parses a cell reference string (e.g., `"A1"`) into (row, col).
///
/// Returns `Some((row, col))` if valid, `None` otherwise.
fn parse_cell_ref(s: &str) -> Option<(i32, i32)> {
    if RE_CELL.is_match(s) {
        let mut col = String::new();
        let mut row_str = String::new();
        for c in s.chars() {
            if c.is_ascii_alphabetic() {
                col.push(c);
            } else if c.is_digit(10) {
                row_str.push(c);
            }
        }
        if let Ok(row) = row_str.parse::<i32>() {
            let col_index = get_col_index(&col);
            if col_index >= 0 {
                return Some((row - 1, col_index)); // Adjust row to 0-based index
            }
        }
    }
    None
}

static mut cycle_detected: bool = false;

/// Adds a dependency edge to the dependency graph.
///
/// Detects cycles and sets the global `cycle_detected` flag.
fn add_dependency(
    graph: &mut Vec<Option<Box<DAGNode>>>,
    dep_row: i32,
    dep_col: i32,
    ref_row: i32,
    ref_col: i32,
    R: i32,
    C: i32,
) {
    if dep_row < 0
        || dep_row >= R
        || dep_col < 0
        || dep_col >= C
        || ref_row < 0
        || ref_row >= R
        || ref_col < 0
        || ref_col >= C
    {
        return;
    }
    if dep_row == ref_row && dep_col == ref_col {
        unsafe {
            cycle_detected = true;
        }
        return;
    }
    let dependent_index = (dep_row * C + dep_col) as usize;
    let reference_index = (ref_row * C + ref_col) as usize;
    if is_reachable(graph, R, C, dependent_index, reference_index) {
        unsafe {
            cycle_detected = true;
        }
        return;
    }
    if let Some(ref mut dag) = graph[reference_index] {
        dag.dependents.insert((dep_row, dep_col));
    }
    if let Some(ref mut dag) = graph[dependent_index] {
        dag.dependencies.insert((ref_row, ref_col));
        dag.in_degree += 1;
    }
}

/// Removes a dependency edge from the dependency graph.
fn remove_dependency(
    graph: &mut Vec<Option<Box<DAGNode>>>,
    dep_row: i32,
    dep_col: i32,
    ref_row: i32,
    ref_col: i32,
    R: i32,
    C: i32,
) {
    let dependent_index = (dep_row * C + dep_col) as usize;
    let reference_index = (ref_row * C + ref_col) as usize;
    if let Some(ref mut dag) = graph[reference_index] {
        dag.dependents.remove(&(dep_row, dep_col));
    }
    if let Some(ref mut dag) = graph[dependent_index] {
        if dag.dependencies.remove(&(ref_row, ref_col)) {
            dag.in_degree -= 1;
        }
    }
}

/// Removes a specific cell reference from a linked list of nodes.
///
/// Used to manage dependency lists in the spreadsheet's internal graph representation.
fn remove_from_list(mut list: Option<Box<Node>>, target: CellRef) -> Option<Box<Node>> {
    let mut current = &mut list;
    loop {
        match current {
            Some(node) if node.cell.row == target.row && node.cell.col == target.col => {
                *current = node.next.take();
                break;
            }
            Some(node) => {
                current = &mut node.next;
            }
            None => break,
        }
    }
    list
}

/// Performs a depth-first search (DFS) in the dependency graph to check for reachability.
///
/// Used internally for cycle detection when adding dependencies between spreadsheet cells.
///
/// # Arguments
/// - `graph`: Reference to the dependency graph (vector of optional DAG nodes)
/// - `R`: Number of rows in the spreadsheet
/// - `C`: Number of columns in the spreadsheet
/// - `curr`: The current node index (flattened row-major)
/// - `target`: The target node index to reach
/// - `visited`: Mutable vector tracking visited nodes
///
/// # Returns
/// `true` if `target` is reachable from `curr`, `false` otherwise.
///
/// # Example
/// ```
/// let found = dfs(&graph, 5, 5, start_idx, target_idx, &mut visited);
/// ```
fn dfs(
    graph: &Vec<Option<Box<DAGNode>>>,
    R: i32,
    C: i32,
    curr: usize,
    target: usize,
    visited: &mut Vec<bool>,
) -> bool {
    let total = (R * C) as usize;
    if curr >= total || visited[curr] {
        return false;
    }
    if curr == target {
        return true;
    }
    visited[curr] = true;
    if let Some(ref dag) = graph[curr] {
        for &(dep_row, dep_col) in &dag.dependents {
            let next = (dep_row * C + dep_col) as usize;
            if next < total && dfs(graph, R, C, next, target, visited) {
                return true;
            }
        }
    }
    false
}

/// Determines whether there is a path from `start` to `target` in the dependency graph.
///
/// This function is used to detect cycles before adding a new dependency edge,
/// ensuring the spreadsheet remains a directed acyclic graph (DAG).
fn is_reachable(
    graph: &Vec<Option<Box<DAGNode>>>,
    R: i32,
    C: i32,
    start: usize,
    target: usize,
) -> bool {
    let mut visited = vec![false; (R * C) as usize];
    dfs(graph, R, C, start, target, &mut visited)
}

/// Checks if a formula's range is invalid (e.g., inverted or self-referential).
///
/// Returns 1 if invalid, 0 otherwise.
fn check_invalid_range(formula: &str, current_row: i32, current_col: i32) -> i32 {
    // Look for a “A1:B2”‐style range inside parentheses
    if formula.contains('(') && formula.contains(':') && formula.contains(')') {
        let open_paren = formula.find('(').unwrap();
        let close_paren = formula.find(')').unwrap();
        let inner = &formula[open_paren + 1..close_paren];
        let parts: Vec<&str> = inner.split(':').collect();

        if parts.len() == 2 {
            // parse_cell_ref returns Option<(i32,i32)>
            if let (Some((s_row, s_col)), Some((e_row, e_col))) =
                (parse_cell_ref(parts[0]), parse_cell_ref(parts[1]))
            {
                // Invalid if the start is “after” the end
                if s_col > e_col || s_row > e_row {
                    return 1;
                }
                // Also mark invalid if the current cell lies *inside* that range
                if current_row >= s_row
                    && current_row <= e_row
                    && current_col >= s_col
                    && current_col <= e_col
                {
                    return 1;
                }
            }
        }
    }

    0
}

/// Evaluates a range formula (e.g., `"SUM(A1:B2)"`) and returns the result.
///
/// Sets `error_flag` if any cell in the range has an error.
fn evaluate_range(
    range: &str,
    R: i32,
    C: i32,
    sheet: &Vec<Vec<cell>>,
    func: &str,
    error_flag: &mut i32,
) -> i32 {
    let mut start_row = 0;
    let mut end_row = 0;
    let mut start_col = 0;
    let mut end_col = 0;
    if parse_range(
        range,
        &mut start_row,
        &mut end_row,
        &mut start_col,
        &mut end_col,
    ) != 0
    {
        unsafe {
            inval_r = true;
        }
        return 0;
    }
    let total_cells = ((end_row - start_row + 1) * (end_col - start_col + 1)) as usize;
    let mut values: Vec<i32> = Vec::with_capacity(total_cells);
    for i in start_row..=end_row {
        for j in start_col..=end_col {
            if sheet[i as usize][j as usize].err != 0 {
                *error_flag = 1;
                return 0;
            }
            values.push(sheet[i as usize][j as usize].val);
        }
    }
    let count = values.len();
    let mut result = 0;
    if func == "SUM" {
        result = values.iter().sum();
    } else if func == "AVG" {
        if count > 0 {
            result = values.iter().sum::<i32>() / count as i32;
        }
    } else if func == "MIN" {
        if count > 0 {
            result = values.iter().min().unwrap_or(&0).clone();
        }
    } else if func == "MAX" {
        if count > 0 {
            result = values.iter().max().unwrap_or(&0).clone();
        }
    } else if func == "STDEV" {
        result = stdev(&values);
    }
    result
}

/// Parses a range string (e.g., `"A1:B2"`) into start/end rows and columns.
///
/// Returns 0 on success, -1 on failure.
fn parse_range(
    range: &str,
    start_row: &mut i32,
    end_row: &mut i32,
    start_col: &mut i32,
    end_col: &mut i32,
) -> i32 {
    let parts: Vec<&str> = range.split(':').collect();
    if parts.len() == 2 {
        if let (Some((s_row, s_col)), Some((e_row, e_col))) =
            (parse_cell_ref(parts[0]), parse_cell_ref(parts[1]))
        {
            // assign into the out‑parameters
            *start_row = s_row;
            *end_row = e_row;
            *start_col = s_col;
            *end_col = e_col;

            // check for inverted ranges
            if *start_row > *end_row || *start_col > *end_col {
                unsafe {
                    inval_r = true;
                }
                return -1;
            }
            return 0;
        }
    }

    // parse_cell_ref failed or wrong number of parts
    unsafe {
        inval_r = true;
    }
    -1
}

/// Computes the sample standard deviation of a list of values.
///
/// Returns 0 if the input has fewer than 2 values.
fn stdev(values: &Vec<i32>) -> i32 {
    let count = values.len();
    if count <= 1 {
        return 0;
    }
    let mean = values.iter().sum::<i32>() as f64 / count as f64;
    let sum_squared_diff = values
        .iter()
        .map(|&x| (x as f64 - mean).powi(2))
        .sum::<f64>();
    let stdev = (sum_squared_diff / count as f64).sqrt();
    (stdev + 0.5) as i32
}

/// Retrieves the value from a formula string, which may be a cell reference or a literal.
///
/// Used for evaluating simple formulas that are either a direct cell reference (e.g., "B3")
/// or an integer literal. Sets an error flag if the input is invalid or references an error cell.
fn get_value_from_formula(
    formula: &str,
    R: i32,
    C: i32,
    sheet: &Vec<Vec<cell>>,
    error_flag: &mut i32,
) -> i32 {
    // If it’s a cell reference like “B3”
    if let Some((row, col)) = parse_cell_ref(formula) {
        // out‑of‑bounds?
        if col < 0 || col >= C || row < 0 || row >= R {
            *error_flag = 1;
            return 0;
        }
        // grab that cell
        let cell = &sheet[row as usize][col as usize];
        // any existing error in it?
        if cell.err != 0 {
            *error_flag = 1;
            return 0;
        }
        // otherwise return its value
        return cell.val;
    }

    // Otherwise, try parsing it as a literal integer
    if let Ok(value) = formula.trim().parse::<i32>() {
        return value;
    }

    // Neither a valid ref nor a number → error
    *error_flag = 1;
    0
}

/// Entry point for the spreadsheet engine.
///
/// Initializes the spreadsheet grid and dependency graph based on command-line arguments,
/// then enters an interactive loop to process user input for cell updates, navigation,
/// and output control. Handles error reporting and cycle detection after each command.
///
/// # Usage
/// ```
/// ./sheet R C
/// ```
/// where `R` and `C` are the number of rows and columns.
///
/// # Panics
/// Exits with an error message if arguments are missing or invalid.
///
/// # Example
/// ```
/// $ ./spreadsheet 10 10
/// ```
fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() != 3 {
        println!("Usage: ./sheet R C");
        process::exit(1);
    }
    let R: i32 = args[1].parse().unwrap_or(0);
    let C: i32 = args[2].parse().unwrap_or(0);
    if R < 1 || R > 999 || C < 1 || C > (26 * 26 * 26 + 26 * 26 + 26) {
        println!("Invalid grid size.");
        process::exit(1);
    }
    let mut sheet: Vec<Vec<cell>> = vec![
        vec![
            cell {
                val: 0,
                formula: None,
                err: 0
            };
            C as usize
        ];
        R as usize
    ];
    // Graph initialization
    let total_cells = (R * C) as usize;
    let mut graph: Vec<Option<Box<DAGNode>>> = (0..total_cells)
        .map(|_| {
            Some(Box::new(DAGNode {
                in_degree: 0,
                dependents: HashSet::new(),
                dependencies: HashSet::new(),
            }))
        })
        .collect();
    for i in 0..total_cells {
        graph[i] = Some(Box::new(DAGNode {
            in_degree: 0,
            dependents: HashSet::new(),
            dependencies: HashSet::new(),
        }));
    }
    let mut row_offset: i32 = 0;
    let mut col_offset: i32 = 0;
    let mut output_enabled: i32 = 1;
    print_sheet(R, C, &sheet, row_offset, col_offset, output_enabled);
    let stdin = io::stdin();
    let mut input_line = String::new();
    print!("[0.0] (ok) > ");
    io::stdout().flush().unwrap();
    while let Ok(n) = stdin.lock().read_line(&mut input_line) {
        if n == 0 {
            break;
        }
        if let Some(pos) = input_line.find('\n') {
            input_line.truncate(pos);
        }
        unsafe {
            sleeptimetotal = 0.0;
            inval_r = false;
            unrec_cmd = false;
            cycle_detected = false;
            last_update.is_updated = false;
        }
        let mut updated_row = -1;
        let mut updated_col = -1;
        let mut backup_formula: Option<Formula> = None;
        {
            let mut col_str = String::new();
            let mut chars = input_line.chars();
            while let Some(c) = chars.clone().next() {
                if c.is_ascii_alphabetic() {
                    col_str.push(c);
                    chars.next();
                } else {
                    break;
                }
            }
            let rest: String = chars.collect();
            let parts: Vec<&str> = rest.split('=').collect();
            if parts.len() == 2 {
                if let Ok(row) = parts[0].trim().parse::<i32>() {
                    updated_row = row - 1;
                    updated_col = get_col_index(&col_str);
                    if updated_col >= 0 && updated_col < C && updated_row >= 0 && updated_row < R {
                        backup_formula = sheet[updated_row as usize][updated_col as usize]
                            .formula
                            .clone();
                    }
                }
            }
        }
        let start = Instant::now();
        process_input(
            &input_line,
            R,
            C,
            &mut sheet,
            &mut graph,
            &mut row_offset,
            &mut col_offset,
            &mut output_enabled,
        );
        unsafe {
            if last_update.is_updated {
                let row = last_update.row;
                let col = last_update.col;
                let idx = (row * C + col) as usize;

                if let Some(ref mut dag) = graph[idx] {
                    // “drain” out all old dependencies into a Vec<CellRef>
                    let deps_to_remove: Vec<CellRef> = dag
                        .dependencies
                        .drain() // removes & yields each (i32,i32)
                        .map(|(r, c)| CellRef { row: r, col: c })
                        .collect();

                    // drop the mutable borrow before calling remove_dependency
                    drop(dag);

                    // now remove each edge
                    for dep in deps_to_remove {
                        remove_dependency(&mut graph, row, col, dep.row, dep.col, R, C);
                    }

                    // reset in_degree on this node
                    if let Some(ref mut dag) = graph[idx] {
                        dag.in_degree = 0;
                    }
                }

                let new_formula = sheet[row as usize][col as usize].formula.as_ref().unwrap();
                let new_deps = get_dependencies_from_formula(new_formula);
                let mut cycle_detected_local = false;
                for dep in &new_deps {
                    if is_reachable(
                        &graph,
                        R,
                        C,
                        (row * C + col) as usize,
                        (dep.row * C + dep.col) as usize,
                    ) {
                        cycle_detected_local = true;
                        break;
                    }
                }
                if cycle_detected_local {
                    sheet[row as usize][col as usize].formula = backup_formula;
                    cycle_detected = true;
                } else {
                    for dep in new_deps {
                        add_dependency(&mut graph, row, col, dep.row, dep.col, R, C);
                    }
                    let mut evaluated = vec![false; total_cells];
                    evaluate_cell(row, col, &mut sheet, &graph, &mut evaluated, R, C);
                }
            }
        }
        if output_enabled != 0 {
            print_sheet(R, C, &sheet, row_offset, col_offset, output_enabled);
        }
        let end = Instant::now();
        unsafe {
            let sleep_time = sleeptimetotal;
            sleeptimetotal = 0.0;
            print!("[{:.2}]", sleep_time);
            if unrec_cmd {
                print!(" (unrecognized cmd) > ");
            } else if inval_r {
                print!(" (Invalid range) > ");
            } else if cycle_detected {
                print!(" (Cycle Detected, change cmd) > ");
            } else {
                print!(" (ok) > ");
            }
            io::stdout().flush().unwrap();
        }
        input_line.clear();
    }
}

fn clock() -> i64 {
    unsafe { libc::time(std::ptr::null_mut()) as i64 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_literal_parsing() {
        let parsed = parse_formula("42");
        assert!(matches!(parsed, Ok(Formula::Literal(42))));
    }

    #[test]
    fn test_cell_parsing() {
        let parsed = parse_formula("A1");
        assert!(matches!(
            parsed,
            Ok(Formula::Cell(CellRef { row: 0, col: 0 }))
        ));
    }

    #[test]
    fn test_arithmetic_parsing() {
        let parsed = parse_formula("2+3");
        assert!(matches!(parsed, Ok(Formula::Arith { op: Op::Add, .. })));
    }

    #[test]
    fn test_range_parsing() {
        let parsed = parse_formula("SUM(A1:B2)");
        assert!(matches!(parsed, Ok(Formula::Range { func, .. }) if func == "SUM"));
    }

    #[test]
    fn test_sleep_literal_parsing() {
        let parsed = parse_formula("SLEEP(5)");
        assert!(matches!(parsed, Ok(Formula::SleepLiteral(5))));
    }

    #[test]
    fn test_invalid_formula() {
        let parsed = parse_formula("garbage");
        assert!(parsed.is_err());
    }

    #[test]
    fn test_is_valid_formula_true() {
        assert!(is_valid_formula("B1+5"));
    }

    #[test]
    fn test_is_valid_formula_false() {
        assert!(!is_valid_formula("XYZ"));
    }

    #[test]
    fn test_get_col_index_valid() {
        assert_eq!(get_col_index("A"), 0);
        assert_eq!(get_col_index("Z"), 25);
        assert_eq!(get_col_index("AA"), 26);
        assert_eq!(get_col_index("AZ"), 51);
        assert_eq!(get_col_index("BA"), 52);
    }

    #[test]
    fn test_get_col_index_invalid() {
        assert_eq!(get_col_index("1A"), -1);
        assert_eq!(get_col_index(""), -1);
    }

    #[test]
    fn test_parse_cell_ref_valid() {
        assert_eq!(parse_cell_ref("A1"), Some((0, 0)));
        assert_eq!(parse_cell_ref("B2"), Some((1, 1)));
    }

    #[test]
    fn test_parse_cell_ref_invalid() {
        assert_eq!(parse_cell_ref("123"), None);
        assert_eq!(parse_cell_ref("A"), None);
    }

    #[test]
    fn test_check_invalid_range_valid() {
        assert_eq!(check_invalid_range("SUM(A1:B2)", 3, 3), 0);
    }

    #[test]
    fn test_check_invalid_range_self_reference() {
        assert_eq!(check_invalid_range("SUM(A1:B2)", 0, 0), 1);
    }

    #[test]
    fn test_get_dependencies_literal() {
        let deps = get_dependencies_from_formula(&Formula::Literal(10));
        assert!(deps.is_empty());
    }

    #[test]
    fn test_get_dependencies_cell() {
        let deps = get_dependencies_from_formula(&Formula::Cell(CellRef { row: 1, col: 2 }));
        assert_eq!(deps, vec![CellRef { row: 1, col: 2 }]);
    }

    #[test]
    fn test_get_dependencies_arith() {
        let f = Formula::Arith {
            op: Op::Add,
            left: Box::new(Formula::Cell(CellRef { row: 0, col: 0 })),
            right: Box::new(Formula::Literal(3)),
        };
        let deps = get_dependencies_from_formula(&f);
        assert_eq!(deps, vec![CellRef { row: 0, col: 0 }]);
    }

    #[test]
    fn test_get_value_from_formula_literal() {
        let sheet = vec![vec![cell {
            val: 5,
            formula: None,
            err: 0,
        }]];
        let mut err = 0;
        let val = get_value_from_formula("42", 1, 1, &sheet, &mut err);
        assert_eq!(val, 42);
        assert_eq!(err, 0);
    }

    #[test]
    fn test_get_value_from_formula_cell() {
        let sheet = vec![vec![cell {
            val: 9,
            formula: None,
            err: 0,
        }]];
        let mut err = 0;
        let val = get_value_from_formula("A1", 1, 1, &sheet, &mut err);
        assert_eq!(val, 9);
        assert_eq!(err, 0);
    }

    #[test]
    fn test_get_value_from_formula_invalid() {
        let sheet = vec![vec![cell {
            val: 9,
            formula: None,
            err: 0,
        }]];
        let mut err = 0;
        let val = get_value_from_formula("INVALID", 1, 1, &sheet, &mut err);
        assert_eq!(val, 0);
        assert_eq!(err, 1);
    }

    #[test]
    fn test_stdev_single_value() {
        let v = vec![5];
        assert_eq!(stdev(&v), 0);
    }

    #[test]
    fn test_evaluate_literal_formula() {
        let mut err = 0;
        let sheet = vec![];
        let val = evaluate_formula(&Formula::Literal(42), &sheet, &mut err);
        assert_eq!(val, 42);
        assert_eq!(err, 0);
    }

    #[test]
    fn test_evaluate_cell_formula_valid() {
        let mut err = 0;
        let sheet = vec![vec![cell {
            val: 10,
            formula: None,
            err: 0,
        }]];
        let val = evaluate_formula(&Formula::Cell(CellRef { row: 0, col: 0 }), &sheet, &mut err);
        assert_eq!(val, 10);
        assert_eq!(err, 0);
    }

    #[test]
    fn test_evaluate_cell_formula_error() {
        let mut err = 0;
        let sheet = vec![vec![cell {
            val: 10,
            formula: None,
            err: 1,
        }]];
        let val = evaluate_formula(&Formula::Cell(CellRef { row: 0, col: 0 }), &sheet, &mut err);
        assert_eq!(val, 0);
        assert_eq!(err, 1);
    }

    #[test]
    fn test_evaluate_cell_formula_out_of_bounds() {
        let mut err = 0;
        let sheet: Vec<Vec<cell>> = vec![];
        let val = evaluate_formula(&Formula::Cell(CellRef { row: 1, col: 1 }), &sheet, &mut err);
        assert_eq!(val, 0);
        assert_eq!(err, 1);
    }

    #[test]
    fn test_evaluate_inc_formula() {
        let mut err = 0;
        let sheet = vec![vec![cell {
            val: 10,
            formula: None,
            err: 0,
        }]];
        let val = evaluate_formula(
            &Formula::Inc {
                base: CellRef { row: 0, col: 0 },
                offset: 5,
            },
            &sheet,
            &mut err,
        );
        assert_eq!(val, 15);
        assert_eq!(err, 0);
    }

    #[test]
    fn test_evaluate_arith_add() {
        let mut err = 0;
        let formula = Formula::Arith {
            op: Op::Add,
            left: Box::new(Formula::Literal(2)),
            right: Box::new(Formula::Literal(3)),
        };
        let val = evaluate_formula(&formula, &vec![], &mut err);
        assert_eq!(val, 5);
        assert_eq!(err, 0);
    }

    #[test]
    fn test_evaluate_arith_div_by_zero() {
        let mut err = 0;
        let formula = Formula::Arith {
            op: Op::Div,
            left: Box::new(Formula::Literal(10)),
            right: Box::new(Formula::Literal(0)),
        };
        let val = evaluate_formula(&formula, &vec![], &mut err);
        assert_eq!(val, 0);
        assert_eq!(err, 1);
    }

    #[test]
    fn test_evaluate_range_sum() {
        let mut err = 0;
        let sheet = vec![
            vec![
                cell {
                    val: 1,
                    formula: None,
                    err: 0,
                },
                cell {
                    val: 2,
                    formula: None,
                    err: 0,
                },
            ],
            vec![
                cell {
                    val: 3,
                    formula: None,
                    err: 0,
                },
                cell {
                    val: 4,
                    formula: None,
                    err: 0,
                },
            ],
        ];
        let formula = Formula::Range {
            func: "SUM".to_string(),
            start: CellRef { row: 0, col: 0 },
            end: CellRef { row: 1, col: 1 },
        };
        let val = evaluate_formula(&formula, &sheet, &mut err);
        assert_eq!(val, 10);
        assert_eq!(err, 0);
    }

    #[test]
    fn test_evaluate_range_invalid_bounds() {
        let mut err = 0;
        let sheet = vec![vec![cell {
            val: 0,
            formula: None,
            err: 0,
        }]];
        let formula = Formula::Range {
            func: "SUM".to_string(),
            start: CellRef { row: 1, col: 1 },
            end: CellRef { row: 2, col: 2 },
        };
        let val = evaluate_formula(&formula, &sheet, &mut err);
        assert_eq!(val, 0);
        assert_eq!(err, 1);
    }

    #[test]
    fn test_evaluate_sleep_literal() {
        let mut err = 0;
        let sheet = vec![];
        let val = evaluate_formula(&Formula::SleepLiteral(7), &sheet, &mut err);
        assert_eq!(val, 7);
        assert_eq!(err, 0);
    }

    #[test]
    fn test_evaluate_sleep_cell() {
        let mut err = 0;
        let sheet = vec![vec![cell {
            val: 4,
            formula: None,
            err: 0,
        }]];
        let val = evaluate_formula(
            &Formula::SleepCell(CellRef { row: 0, col: 0 }),
            &sheet,
            &mut err,
        );
        assert_eq!(val, 4);
        assert_eq!(err, 0);
    }

    #[test]
    fn test_add_dependency_self_cycle() {
        let mut graph = vec![Some(Box::new(DAGNode {
            in_degree: 0,
            dependencies: HashSet::new(),
            dependents: HashSet::new(),
        }))];
        unsafe {
            cycle_detected = false;
        }
        add_dependency(&mut graph, 0, 0, 0, 0, 1, 1);
        unsafe {
            assert!(cycle_detected);
        }
    }

    #[test]
    fn test_remove_dependency() {
        let mut graph = vec![
            Some(Box::new(DAGNode {
                in_degree: 1,
                dependencies: [(0, 1)].iter().cloned().collect(),
                dependents: HashSet::new(),
            })),
            Some(Box::new(DAGNode {
                in_degree: 0,
                dependencies: HashSet::new(),
                dependents: [(0, 0)].iter().cloned().collect(),
            })),
        ];
        remove_dependency(&mut graph, 0, 0, 0, 1, 1, 2);
        assert!(graph[0].as_ref().unwrap().dependencies.is_empty());
        assert!(graph[1].as_ref().unwrap().dependents.is_empty());
        assert_eq!(graph[0].as_ref().unwrap().in_degree, 0);
    }

    #[test]
    fn test_is_reachable_true() {
        let mut graph = vec![
            Some(Box::new(DAGNode {
                in_degree: 0,
                dependencies: HashSet::new(),
                dependents: [(0, 1)].iter().cloned().collect(),
            })),
            Some(Box::new(DAGNode {
                in_degree: 0,
                dependencies: HashSet::new(),
                dependents: HashSet::new(),
            })),
        ];
        let result = is_reachable(&graph, 1, 2, 0, 1);
        assert!(result);
    }

    #[test]
    fn test_is_reachable_false() {
        let graph = vec![
            Some(Box::new(DAGNode {
                in_degree: 0,
                dependencies: HashSet::new(),
                dependents: HashSet::new(),
            })),
            Some(Box::new(DAGNode {
                in_degree: 0,
                dependencies: HashSet::new(),
                dependents: HashSet::new(),
            })),
        ];
        assert!(!is_reachable(&graph, 1, 2, 0, 1));
    }

    #[test]
    fn test_parse_range_valid() {
        let mut sr = 0;
        let mut er = 0;
        let mut sc = 0;
        let mut ec = 0;
        let result = parse_range("A1:B2", &mut sr, &mut er, &mut sc, &mut ec);
        assert_eq!(result, 0);
        assert_eq!((sr, er, sc, ec), (0, 1, 0, 1));
    }

    #[test]
    fn test_parse_range_invalid_format() {
        let mut sr = 0;
        let mut er = 0;
        let mut sc = 0;
        let mut ec = 0;
        let result = parse_range("invalid", &mut sr, &mut er, &mut sc, &mut ec);
        assert_eq!(result, -1);
    }

    #[test]
    fn test_evaluate_range_with_error() {
        let sheet = vec![vec![cell {
            val: 1,
            formula: None,
            err: 1,
        }]];
        let mut err = 0;
        let result = evaluate_range("A1:A1", 1, 1, &sheet, "SUM", &mut err);
        assert_eq!(result, 0);
        assert_eq!(err, 1);
    }

    #[test]
    fn test_process_input_invalid_command_sets_unrec_cmd() {
        let mut sheet = vec![vec![cell {
            val: 0,
            formula: None,
            err: 0,
        }]];
        let mut graph = vec![Some(Box::new(DAGNode {
            in_degree: 0,
            dependencies: HashSet::new(),
            dependents: HashSet::new(),
        }))];
        let mut row_offset = 0;
        let mut col_offset = 0;
        let mut output_enabled = 1;

        unsafe {
            unrec_cmd = false;
        }
        process_input(
            "XYZ",
            1,
            1,
            &mut sheet,
            &mut graph,
            &mut row_offset,
            &mut col_offset,
            &mut output_enabled,
        );
        unsafe {
            assert!(unrec_cmd);
        }
    }

    #[test]
    fn test_check_invalid_range_inverted_start_end() {
        let result = check_invalid_range("SUM(B2:A1)", 0, 0);
        assert_eq!(result, 1);
    }

    #[test]
    fn test_check_invalid_range_inside_own_range() {
        let result = check_invalid_range("SUM(A1:C3)", 1, 1); // Inside range
        assert_eq!(result, 1);
    }

    #[test]
    fn test_sleep_cell_invalid_cell_sets_error() {
        let sheet = vec![vec![cell {
            val: 0,
            formula: None,
            err: 1,
        }]];
        let mut err = 0;
        let val = evaluate_formula(
            &Formula::SleepCell(CellRef { row: 0, col: 0 }),
            &sheet,
            &mut err,
        );
        assert_eq!(val, 0);
        assert_eq!(err, 1);
    }

    #[test]
    fn test_parse_formula_sleep_cell() {
        let parsed = parse_formula("SLEEP(A1)");
        assert!(matches!(
            parsed,
            Ok(Formula::SleepCell(CellRef { row: 0, col: 0 }))
        ));
    }

    #[test]
    fn test_parse_formula_sleep_invalid() {
        let parsed = parse_formula("SLEEP(XYZ)");
        assert!(parsed.is_err());
    }

    #[test]
    fn test_parse_formula_range_func_invalid_colon() {
        let parsed = parse_formula("SUM(A1B2)"); // No colon
        assert!(parsed.is_err());
    }

    #[test]
    fn test_parse_formula_arith_invalid_operator() {
        let parsed = parse_formula("A1%2"); // Unsupported operator %
        assert!(parsed.is_err());
    }

    #[test]
    fn test_parse_formula_arith_spaces() {
        let parsed = parse_formula(" 3 * 4 ");
        assert!(matches!(parsed, Ok(Formula::Arith { op: Op::Mul, .. })));
    }

    #[test]
    fn test_parse_formula_invalid_range_parts_len() {
        let parsed = parse_formula("SUM(A1)");
        assert!(parsed.is_err()); // parts.len() != 2
    }

    #[test]
    fn test_parse_formula_range_func_invalid_cell() {
        let parsed = parse_formula("SUM(A1:X)");
        assert!(parsed.is_err()); // right cell ref fails
    }

    #[test]
    fn test_is_valid_formula_arith() {
        assert!(is_valid_formula("A1 + 2"));
    }

    #[test]
    fn test_is_valid_formula_range() {
        assert!(is_valid_formula("AVG(A1:B2)"));
    }

    #[test]
    fn test_is_valid_formula_sleep() {
        assert!(is_valid_formula("SLEEP(10)"));
    }

    #[test]
    fn test_is_valid_formula_cell_ref() {
        assert!(is_valid_formula("B4"));
    }

    #[test]
    fn test_is_valid_formula_integer_literal() {
        assert!(is_valid_formula("  42 "));
    }

    #[test]
    fn test_is_valid_formula_false_branch() {
        assert!(!is_valid_formula("!!@#"));
    }

    #[test]
    fn test_check_invalid_range_bad_parens() {
        let result = check_invalid_range("SUMA1:B2)", 0, 0); // No opening (
        assert_eq!(result, 0); // Should skip and return 0
    }

    #[test]
    fn test_check_invalid_range_empty_parts() {
        let result = check_invalid_range("SUM(:)", 0, 0); // empty split
        assert_eq!(result, 0); // parse_cell_ref fails
    }

    #[test]
    fn test_get_value_from_formula_empty_string() {
        let sheet = vec![vec![cell {
            val: 0,
            formula: None,
            err: 0,
        }]];
        let mut err = 0;
        let result = get_value_from_formula("", 1, 1, &sheet, &mut err);
        assert_eq!(result, 0);
        assert_eq!(err, 1);
    }

    #[test]
    fn test_get_value_from_formula_valid_literal_with_spaces() {
        let sheet = vec![vec![cell {
            val: 0,
            formula: None,
            err: 0,
        }]];
        let mut err = 0;
        let result = get_value_from_formula("   20 ", 1, 1, &sheet, &mut err);
        assert_eq!(result, 20);
        assert_eq!(err, 0);
    }

    #[test]
    fn test_evaluate_cell_simple_dependency() {
        let mut sheet = vec![vec![
            cell {
                val: 0,
                formula: Some(Formula::Literal(3)),
                err: 0,
            },
            cell {
                val: 0,
                formula: Some(Formula::Cell(CellRef { row: 0, col: 0 })),
                err: 0,
            },
        ]];
        let mut graph = vec![
            Some(Box::new(DAGNode {
                in_degree: 0,
                dependencies: HashSet::new(),
                dependents: [(0, 1)].iter().cloned().collect(),
            })),
            Some(Box::new(DAGNode {
                in_degree: 1,
                dependencies: [(0, 0)].iter().cloned().collect(),
                dependents: HashSet::new(),
            })),
        ];
        let mut evaluated = vec![false, false];
        evaluate_cell(0, 0, &mut sheet, &graph, &mut evaluated, 1, 2);
        assert_eq!(sheet[0][0].val, 3);
        assert_eq!(sheet[0][1].val, 3);
        assert!(evaluated[0]);
        assert!(evaluated[1]);
    }

    #[test]
    fn test_evaluate_cell_cycle_ignored_due_to_evaluated() {
        let mut sheet = vec![vec![
            cell {
                val: 1,
                formula: Some(Formula::Literal(1)),
                err: 0,
            },
            cell {
                val: 2,
                formula: Some(Formula::Cell(CellRef { row: 0, col: 0 })),
                err: 0,
            },
        ]];
        let mut graph = vec![
            Some(Box::new(DAGNode {
                in_degree: 0,
                dependencies: HashSet::new(),
                dependents: [(0, 1)].iter().cloned().collect(),
            })),
            Some(Box::new(DAGNode {
                in_degree: 1,
                dependencies: [(0, 0)].iter().cloned().collect(),
                dependents: HashSet::new(),
            })),
        ];
        let mut evaluated = vec![true, false]; // Pretend cell 0,0 is already evaluated
        evaluate_cell(0, 1, &mut sheet, &graph, &mut evaluated, 1, 2);
        assert_eq!(sheet[0][1].val, 1); // Should have pulled value from 0,0
    }

    #[test]
    fn test_dependency_graph_multiple_dependents() {
        let mut sheet = vec![vec![
            cell {
                val: 0,
                formula: Some(Formula::Literal(4)),
                err: 0,
            },
            cell {
                val: 0,
                formula: Some(Formula::Cell(CellRef { row: 0, col: 0 })),
                err: 0,
            },
            cell {
                val: 0,
                formula: Some(Formula::Cell(CellRef { row: 0, col: 0 })),
                err: 0,
            },
        ]];
        let mut graph = vec![
            Some(Box::new(DAGNode {
                in_degree: 0,
                dependencies: HashSet::new(),
                dependents: [(0, 1), (0, 2)].iter().cloned().collect(),
            })),
            Some(Box::new(DAGNode {
                in_degree: 1,
                dependencies: [(0, 0)].iter().cloned().collect(),
                dependents: HashSet::new(),
            })),
            Some(Box::new(DAGNode {
                in_degree: 1,
                dependencies: [(0, 0)].iter().cloned().collect(),
                dependents: HashSet::new(),
            })),
        ];
        let mut evaluated = vec![false; 3];
        evaluate_cell(0, 0, &mut sheet, &graph, &mut evaluated, 1, 3);
        assert_eq!(sheet[0][1].val, 4);
        assert_eq!(sheet[0][2].val, 4);
    }
}

#[cfg(test)]
mod additional_tests {
    use super::*;

    // Move helper functions here to make them available to all tests
    fn initialize_sheet(rows: i32, cols: i32) -> Vec<Vec<cell>> {
        vec![
            vec![
                cell {
                    val: 0,
                    formula: None,
                    err: 0
                };
                cols as usize
            ];
            rows as usize
        ]
    }

    fn initialize_graph(rows: i32, cols: i32) -> Vec<Option<Box<DAGNode>>> {
        (0..rows * cols)
            .map(|_| {
                Some(Box::new(DAGNode {
                    in_degree: 0,
                    dependents: HashSet::new(),
                    dependencies: HashSet::new(),
                }))
            })
            .collect()
    }

    #[test]
    fn test_parse_formula_arithmetic_all_operators() {
        // Test all arithmetic operators
        assert!(parse_formula("A1*B2").is_ok());
        assert!(parse_formula("C3/2").is_ok());
        assert!(parse_formula("5-3").is_ok());
        assert!(parse_formula("X5+Y6").is_ok());
    }

    #[test]
    fn test_parse_formula_invalid_operator() {
        // Test invalid operator
        assert!(parse_formula("A1%2").is_err());
    }

    #[test]
    fn test_scroll_commands_edge_cases() {
        let mut sheet = initialize_sheet(15, 15);
        let mut graph = initialize_graph(15, 15);
        let mut row_offset = 5;
        let mut col_offset = 5;
        let mut output = 1;

        // Scroll beyond left boundary
        process_input(
            "a",
            15,
            15,
            &mut sheet,
            &mut graph,
            &mut row_offset,
            &mut col_offset,
            &mut output,
        );
        assert_eq!(col_offset, 0);

        // Scroll beyond top
        process_input(
            "w",
            15,
            15,
            &mut sheet,
            &mut graph,
            &mut row_offset,
            &mut col_offset,
            &mut output,
        );
        assert_eq!(row_offset, 0);
    }

    #[test]
    fn test_lowercase_cell_reference() {
        // Should fail with invalid column
        assert_eq!(get_col_index("a1"), -1);
        assert_eq!(parse_cell_ref("a1"), None);
    }

    #[test]
    fn test_cycle_detection_multiple_nodes() {
        let R = 3;
        let C = 3;
        let mut graph = initialize_graph(R, C);

        // A1 -> B2 -> C3 -> A1
        add_dependency(&mut graph, 0, 0, 1, 1, R, C);
        add_dependency(&mut graph, 1, 1, 2, 2, R, C);
        add_dependency(&mut graph, 2, 2, 0, 0, R, C);

        unsafe { assert!(cycle_detected) };
    }

    #[test]
    fn test_evaluate_inc_formula() {
        let mut sheet = initialize_sheet(2, 2);
        sheet[0][0].val = 5;
        let formula = parse_formula("A1+3").unwrap();
        let mut err = 0;
        assert_eq!(evaluate_formula(&formula, &sheet, &mut err), 8);
    }

    #[test]
    fn test_division_by_zero_propagation() {
        let mut sheet = initialize_sheet(1, 2);
        sheet[0][1].formula = Some(Formula::Literal(0));
        let formula = parse_formula("A1/B1").unwrap();
        let mut err = 0;
        evaluate_formula(&formula, &sheet, &mut err);
        assert_eq!(err, 1);
    }

    #[test]
    fn test_stdev_valid_calculation() {
        let vals = vec![2, 4, 4, 4, 5, 5, 7, 9];
        assert_eq!(stdev(&vals), 2); // Actual stdev ≈ 2.138
    }

    #[test]
    fn test_sleep_with_error_cell() {
        let mut sheet = initialize_sheet(2, 2);
        sheet[0][0].err = 1;
        let formula = parse_formula("SLEEP(A1)").unwrap();
        let mut err = 0;
        evaluate_formula(&formula, &sheet, &mut err);
        assert_eq!(err, 1);
    }

    #[test]
    fn test_invalid_range_parsing() {
        let mut sheet = initialize_sheet(2, 2);
        let mut graph = initialize_graph(2, 2);
        let mut row_offset = 0;
        let mut col_offset = 0;
        let mut output_enabled = 1;

        process_input(
            "A1=SUM(B2:A1)",
            2,
            2,
            &mut sheet,
            &mut graph,
            &mut row_offset,
            &mut col_offset,
            &mut output_enabled,
        );

        unsafe {
            assert!(inval_r); // Should detect invalid range
        }
    }

    #[test]
    fn test_viewport_scrolling_limits() {
        let mut sheet = initialize_sheet(20, 20);
        let mut graph = initialize_graph(20, 20);
        let mut row_offset = 15;
        let mut col_offset = 15;
        let mut output_enabled = 1;

        // Try to scroll beyond bottom-right
        process_input(
            "s",
            20,
            20,
            &mut sheet,
            &mut graph,
            &mut row_offset,
            &mut col_offset,
            &mut output_enabled,
        );
        assert_eq!(row_offset, 10); // Max 20-10=10

        process_input(
            "d",
            20,
            20,
            &mut sheet,
            &mut graph,
            &mut row_offset,
            &mut col_offset,
            &mut output_enabled,
        );
        assert_eq!(col_offset, 10);
    }

    #[test]
    fn test_output_disabled_behavior() {
        let sheet = initialize_sheet(1, 1);
        let mut output_enabled = 0;
        // Should produce no output
        print_sheet(1, 1, &sheet, 0, 0, output_enabled);
        // Verify by absence of output (test would need output capture)
    }

    #[test]
    fn test_viewport_rendering() {
        let sheet = initialize_sheet(5, 5);
        // Test partial viewport rendering
        print_sheet(5, 5, &sheet, 3, 3, 1);
        // Should render 2x2 grid (rows 4-5, cols D-E)
    }

    #[test]
    fn test_indirect_circular_dependency() {
        let mut sheet = initialize_sheet(2, 2);
        let mut graph = initialize_graph(2, 2);
        let mut row_offset = 0; // Add missing parameters
        let mut col_offset = 0;
        let mut output = 1;

        process_input(
            "A1=B1",
            2,
            2,
            &mut sheet,
            &mut graph,
            &mut row_offset,
            &mut col_offset,
            &mut output,
        );

        process_input(
            "B1=A1",
            2,
            2,
            &mut sheet,
            &mut graph,
            &mut row_offset,
            &mut col_offset,
            &mut output,
        );

        unsafe {
            assert!(cycle_detected);
        }
    }

    #[test]
    fn test_scroll_to_valid_cell() {
        let mut sheet = initialize_sheet(10, 10);
        let mut graph = initialize_graph(10, 10);
        let mut row_offset = 0;
        let mut col_offset = 0;
        let mut output_enabled = 1;

        // Scroll to B3 (row 3, col 1)
        process_input(
            "scroll_to B3",
            10,
            10,
            &mut sheet,
            &mut graph,
            &mut row_offset,
            &mut col_offset,
            &mut output_enabled,
        );
        assert_eq!(row_offset, 2); // 3 - 1 = 2
        assert_eq!(col_offset, 1); // B = 1
    }

    #[test]
    fn test_dependency_removal_on_formula_update() {
        let mut sheet = initialize_sheet(5, 5);
        let mut graph = initialize_graph(5, 5);
        let mut row_offset = 0;
        let mut col_offset = 0;
        let mut output_enabled = 1;

        // Setup initial dependency: A1 depends on B2
        process_input(
            "A1=B2",
            5,
            5,
            &mut sheet,
            &mut graph,
            &mut row_offset,
            &mut col_offset,
            &mut output_enabled,
        );

        // Update A1 to new formula without B2 dependency
        process_input(
            "A1=5",
            5,
            5,
            &mut sheet,
            &mut graph,
            &mut row_offset,
            &mut col_offset,
            &mut output_enabled,
        );

        // Verify B2's dependents list is empty
        let b2_index = (1 * 5 + 1) as usize;
        assert!(graph[b2_index].as_ref().unwrap().dependents.is_empty());
    }

    #[test]
    fn test_dependency_evaluation_order() {
        let mut sheet = initialize_sheet(3, 3);
        let mut graph = initialize_graph(3, 3);

        // Create dependency chain: A1 -> B2 -> C3
        process_input(
            "A1=B2", 3, 3, &mut sheet, &mut graph, &mut 0, &mut 0, &mut 1,
        );
        process_input(
            "B2=C3", 3, 3, &mut sheet, &mut graph, &mut 0, &mut 0, &mut 1,
        );
        process_input("C3=5", 3, 3, &mut sheet, &mut graph, &mut 0, &mut 0, &mut 1);

        let mut evaluated = vec![false; 9];
        evaluate_cell(0, 0, &mut sheet, &graph, &mut evaluated, 3, 3);

        // Verify all cells in chain were evaluated
        assert!(evaluated[0]); // A1
        assert!(evaluated[4]); // B2 (index 1*3 + 1 = 4)
        assert!(evaluated[8]); // C3 (index 2*3 + 2 = 8)
        assert_eq!(sheet[0][0].val, 5);
    }

    #[test]
    fn test_evaluate_range_avg() {
        let mut sheet = initialize_sheet(3, 3);
        // Setup values: 2, 4, 6 in A1-C1
        for (i, val) in [2, 4, 6].iter().enumerate() {
            sheet[0][i].val = *val;
        }
        let formula = Formula::Range {
            func: "AVG".to_string(),
            start: CellRef { row: 0, col: 0 },
            end: CellRef { row: 0, col: 2 },
        };
        let mut err = 0;
        let result = evaluate_formula(&formula, &sheet, &mut err);
        assert_eq!(result, 4); // (2+4+6)/3 = 4
        assert_eq!(err, 0);
    }

    #[test]
    fn test_evaluate_range_min_max() {
        let mut sheet = initialize_sheet(2, 2);
        sheet[0][0].val = -5;
        sheet[0][1].val = 10;
        sheet[1][0].val = 3;
        sheet[1][1].val = 7;

        // Test MIN
        let min_formula = Formula::Range {
            func: "MIN".to_string(),
            start: CellRef { row: 0, col: 0 },
            end: CellRef { row: 1, col: 1 },
        };
        let mut err = 0;
        assert_eq!(evaluate_formula(&min_formula, &sheet, &mut err), -5);

        // Test MAX
        let max_formula = Formula::Range {
            func: "MAX".to_string(),
            start: CellRef { row: 0, col: 0 },
            end: CellRef { row: 1, col: 1 },
        };
        assert_eq!(evaluate_formula(&max_formula, &sheet, &mut err), 10);
    }

    #[test]
    fn test_evaluate_range_stdev_edge_cases() {
        // Single value case (n=1)
        let mut sheet = initialize_sheet(1, 1);
        sheet[0][0].val = 5;
        let formula = Formula::Range {
            func: "STDEV".to_string(),
            start: CellRef { row: 0, col: 0 },
            end: CellRef { row: 0, col: 0 },
        };
        let mut err = 0;
        assert_eq!(evaluate_formula(&formula, &sheet, &mut err), 0);

        // Two values
        let mut sheet = initialize_sheet(1, 2);
        sheet[0][0].val = 2;
        sheet[0][1].val = 4;
        let formula = Formula::Range {
            func: "STDEV".to_string(),
            start: CellRef { row: 0, col: 0 },
            end: CellRef { row: 0, col: 1 },
        };
        assert_eq!(evaluate_formula(&formula, &sheet, &mut err), 1); // sqrt(2) ≈ 1.414 → rounded to 1
    }

    #[test]
    fn test_evaluate_range_stdev_large_sample() {
        let mut sheet = initialize_sheet(1, 5);
        let values = [1, 2, 3, 4, 5];
        for (i, &val) in values.iter().enumerate() {
            sheet[0][i].val = val;
        }
        let formula = Formula::Range {
            func: "STDEV".to_string(),
            start: CellRef { row: 0, col: 0 },
            end: CellRef { row: 0, col: 4 },
        };
        let mut err = 0;
        // Population stddev would be sqrt(2), but we're using sample stddev (n-1)
        // Variance = (10)/4 = 2.5 → stddev ≈ 1.581 → rounded to 2
        assert_eq!(evaluate_formula(&formula, &sheet, &mut err), 2);
    }

    #[test]
    fn test_range_function_error_propagation() {
        let mut sheet = initialize_sheet(2, 2);
        sheet[0][0].err = 1; // Error in first cell
        let formula = Formula::Range {
            func: "SUM".to_string(),
            start: CellRef { row: 0, col: 0 },
            end: CellRef { row: 1, col: 1 },
        };
        let mut err = 0;
        evaluate_formula(&formula, &sheet, &mut err);
        assert_eq!(err, 1);
    }

    #[test]
    fn test_range_dependencies_multiple_cells() {
        let formula = Formula::Range {
            func: "SUM".to_string(),
            start: CellRef { row: 1, col: 1 }, // B2
            end: CellRef { row: 3, col: 3 },   // D4
        };

        let deps = get_dependencies_from_formula(&formula);
        assert_eq!(deps.len(), 9); // 3x3 grid (rows 2-4, cols B-D)
        assert!(deps.contains(&CellRef { row: 1, col: 1 }));
        assert!(deps.contains(&CellRef { row: 3, col: 3 }));
    }

    #[test]
    fn test_single_cell_range_dependencies() {
        let formula = Formula::Range {
            func: "MIN".to_string(),
            start: CellRef { row: 0, col: 0 }, // A1
            end: CellRef { row: 0, col: 0 },   // A1
        };

        let deps = get_dependencies_from_formula(&formula);
        assert_eq!(deps, vec![CellRef { row: 0, col: 0 }]);
    }

    #[test]
    fn test_sleep_literal_no_dependencies() {
        let formula = Formula::SleepLiteral(5);
        let deps = get_dependencies_from_formula(&formula);
        assert!(deps.is_empty());
    }

    #[test]
    fn test_sleep_cell_dependency() {
        let formula = Formula::SleepCell(CellRef { row: 2, col: 4 }); // E3
        let deps = get_dependencies_from_formula(&formula);
        assert_eq!(deps, vec![CellRef { row: 2, col: 4 }]);
    }

    #[test]
    fn test_remove_from_list_head() {
        // Create list: A1 -> B2 -> C3
        let list = create_node(vec![
            CellRef { row: 0, col: 0 },
            CellRef { row: 1, col: 1 },
            CellRef { row: 2, col: 2 },
        ]);

        // Remove head (A1)
        let result = remove_from_list(list, CellRef { row: 0, col: 0 });
        assert_eq!(
            list_to_vec(&result),
            vec![CellRef { row: 1, col: 1 }, CellRef { row: 2, col: 2 }]
        );
    }

    #[test]
    fn test_remove_from_list_middle() {
        // Create list: A1 -> B2 -> C3
        let list = create_node(vec![
            CellRef { row: 0, col: 0 },
            CellRef { row: 1, col: 1 },
            CellRef { row: 2, col: 2 },
        ]);

        // Remove middle node (B2)
        let result = remove_from_list(list, CellRef { row: 1, col: 1 });
        assert_eq!(
            list_to_vec(&result),
            vec![CellRef { row: 0, col: 0 }, CellRef { row: 2, col: 2 }]
        );
    }

    // Helper functions
    fn create_node(cells: Vec<CellRef>) -> Option<Box<Node>> {
        let mut head = None;
        for cell in cells.into_iter().rev() {
            head = Some(Box::new(Node {
                cell,
                next: head.take(),
            }));
        }
        head
    }

    fn list_to_vec(list: &Option<Box<Node>>) -> Vec<CellRef> {
        let mut result = Vec::new();
        let mut current = list.as_ref();
        while let Some(node) = current {
            result.push(node.cell);
            current = node.next.as_ref();
        }
        result
    }

    #[test]
    fn test_evaluate_range_sum_basic() {
        let sheet = vec![
            vec![
                cell {
                    val: 2,
                    formula: None,
                    err: 0,
                },
                cell {
                    val: 3,
                    formula: None,
                    err: 0,
                },
            ],
            vec![
                cell {
                    val: 4,
                    formula: None,
                    err: 0,
                },
                cell {
                    val: 1,
                    formula: None,
                    err: 0,
                },
            ],
        ];
        let mut err = 0;
        let result = evaluate_range("A1:B2", 2, 2, &sheet, "SUM", &mut err);
        assert_eq!(result, 10); // 2+3+4+1=10
        assert_eq!(err, 0);
    }

    #[test]
    fn test_evaluate_range_avg_basic() {
        let sheet = vec![
            vec![
                cell {
                    val: 6,
                    formula: None,
                    err: 0,
                },
                cell {
                    val: 3,
                    formula: None,
                    err: 0,
                },
            ],
            vec![
                cell {
                    val: 9,
                    formula: None,
                    err: 0,
                },
                cell {
                    val: 0,
                    formula: None,
                    err: 0,
                },
            ],
        ];
        let mut err = 0;
        let result = evaluate_range("A1:B2", 2, 2, &sheet, "AVG", &mut err);
        assert_eq!(result, 4); // (6+3+9+0)/4=4
        assert_eq!(err, 0);
    }
}

// Helper function for main argument testing
fn main_with_args(args: Vec<String>) {
    let args: Vec<String> = env::args().collect();
    // Rest of main logic
}
