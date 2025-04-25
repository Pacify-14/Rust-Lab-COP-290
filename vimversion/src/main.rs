#![allow(warnings)]
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
mod vim_mode;

use clap::{Arg, Command};
use libc;
/// An AST for all the ways you can compute a cell
/// Your AST for formulas
#[derive(Clone, Debug)]
pub enum Formula {
    Literal(i32),
    Cell(CellRef),
    Inc {
        base: CellRef,
        offset: i32,
    },
    Arith {
        op: Op,
        left: Box<Formula>,
        right: Box<Formula>,
    },
    Range {
        func: String,
        start: CellRef,
        end: CellRef,
    },
    SleepLiteral(i32),
    SleepCell(CellRef),
}

/// Your operator enum
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    Add,
    Sub,
    Mul,
    Div,
}

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

#[derive(Clone)]
#[allow(non_camel_case_types)]
pub struct cell {
    pub val: i32,
    pub formula: Option<Formula>,
    pub err: i32,
}

#[derive(Copy, Clone)]
pub struct CellUpdate {
    pub row: i32,
    pub col: i32,
    pub is_updated: bool,
}

static mut last_update: CellUpdate = CellUpdate {
    row: -1,
    col: -1,
    is_updated: false,
};

#[derive(Debug, Copy, Clone, PartialEq)]
pub struct CellRef {
    pub row: i32,
    pub col: i32,
}

pub struct Node {
    pub cell: CellRef,
    pub next: Option<Box<Node>>,
}

pub struct DAGNode {
    pub in_degree: i32,
    pub dependents: HashSet<(i32, i32)>, // Replacing linked list
    pub dependencies: HashSet<(i32, i32)>, // Replacing linked list
}

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

fn main() {
    let matches = Command::new("Hacker Spreadsheet")
        .version("1.0")
        .author("Your Name")
        .about("A vim-like spreadsheet editor for the terminal")
        .arg(
            Arg::new("vim")
                .long("vim")
                .help("Enable vim-like interface")
                .action(clap::ArgAction::SetTrue),
        )
        .arg(
            Arg::new("rows")
                .short('r')
                .long("rows")
                .help("Number of rows")
                .value_name("ROWS"),
        )
        .arg(
            Arg::new("cols")
                .short('c')
                .long("cols")
                .help("Number of columns")
                .value_name("COLS"),
        )
        .arg(Arg::new("R").help("Number of rows (positional)").index(1))
        .arg(
            Arg::new("C")
                .help("Number of columns (positional)")
                .index(2),
        )
        .get_matches();

    let vim_mode = matches.get_flag("vim");

    // Get rows and columns from either named or positional arguments
    let R = if let Some(r) = matches.get_one::<String>("rows") {
        r.parse::<i32>().unwrap_or(20)
    } else if let Some(r) = matches.get_one::<String>("R") {
        r.parse::<i32>().unwrap_or(20)
    } else {
        20 // Default
    };

    let C = if let Some(c) = matches.get_one::<String>("cols") {
        c.parse::<i32>().unwrap_or(20)
    } else if let Some(c) = matches.get_one::<String>("C") {
        c.parse::<i32>().unwrap_or(20)
    } else {
        20 // Default
    };

    // Validate dimensions
    if R < 1 || R > 100000 || C < 1 || C > (26 * 26 * 26 + 26 * 26 + 26) {
        println!("Invalid grid size.");
        process::exit(1);
    }

    // If vim mode is enabled, run the vim interface
    if vim_mode {
        vim_mode::editor::run_vim_interface(R, C);
        return;
    }

    let args: Vec<String> = env::args().collect();
    let (R, C) = if args.len() >= 3 {
        // Use command-line arguments
        let r = args[1].parse::<i32>().unwrap_or(20);
        let c = args[2].parse::<i32>().unwrap_or(20);
        (r, c)
    } else {
        // Use the values from clap
        (R, C)
    };

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
