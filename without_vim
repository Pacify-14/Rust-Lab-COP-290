#![allow(warnings)]
use regex::Regex;
use std::cell::Ref;
use std::cell::RefCell;
use std::env;
use std::io::{self, BufRead, Write};
use std::process;
use std::str;
use std::time::Instant;

use libc;

const _CLOCKS_PER_SEC: i64 = 1_000_000;
const VIEWPORT_SIZE: i32 = 10;
static mut inval_r: bool = false;
static mut unrec_cmd: bool = false;
static mut sleeptimetotal: f64 = 0.0;

#[derive(Clone)]
#[allow(non_camel_case_types)]
pub struct cell {
    pub val: i32,
    pub formula: Option<String>,
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

#[derive(Copy, Clone)]
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
    pub dependents: Option<Box<Node>>,
    pub dependencies: Option<Box<Node>>, // Added to track cells this cell depends on
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

fn is_valid_formula(formula: &str) -> bool {
    let re_arith = Regex::new(r"^\s*([A-Z]+\d+|\d+)\s*([\+\-\*/])\s*([A-Z]+\d+|\d+)\s*$").unwrap();
    if re_arith.is_match(formula) {
        return true;
    }
    let re_range_func = Regex::new(r"^(SUM|AVG|MIN|MAX|STDEV)\([A-Z]+\d+:[A-Z]+\d+\)$").unwrap();
    if re_range_func.is_match(formula) {
        return true;
    }
    let re_sleep = Regex::new(r"^SLEEP\((\d+|[A-Z]+\d+)\)$").unwrap();
    if re_sleep.is_match(formula) {
        return true;
    }
    let re_cell = Regex::new(r"^[A-Z]+\d+$").unwrap();
    if re_cell.is_match(formula) {
        return true;
    }
    formula.trim().parse::<i32>().is_ok()
}

fn process_input(
    input: &str,
    R: i32,
    C: i32,
    sheet: &mut Vec<Vec<cell>>,
    row_offset: &mut i32,
    col_offset: &mut i32,
    output_enabled: &mut i32,
) {
    unsafe {
        sleeptimetotal = 0.0;
    }
    if input == "q" {
        process::exit(0);
    } else if input == "w" {
        *row_offset = if *row_offset >= 10 {
            *row_offset - 10
        } else {
            0
        };
        return;
    } else if input == "s" {
        *row_offset = if *row_offset + 10 < R {
            *row_offset + 10
        } else {
            *row_offset
        };
        return;
    } else if input == "a" {
        *col_offset = if *col_offset >= 10 {
            *col_offset - 10
        } else {
            0
        };
        return;
    } else if input == "d" {
        *col_offset = if *col_offset + 10 < C {
            *col_offset + 10
        } else {
            *col_offset
        };
        return;
    } else if input.starts_with("scroll_to") {
        let parts: Vec<&str> = input[10..].trim().split_whitespace().collect();
        if parts.len() >= 1 {
            let col_str: String = parts[0].chars().take_while(|c| c.is_alphabetic()).collect();
            let row_str: String = parts[0].chars().skip_while(|c| c.is_alphabetic()).collect();
            if let Ok(row) = row_str.parse::<i32>() {
                let colIndex = get_col_index(&col_str);
                if colIndex >= 0 && colIndex < C && row >= 1 && row <= R {
                    *row_offset = row - 1;
                    *col_offset = colIndex;
                }
            }
        }
        return;
    } else if input == "disable_output" {
        *output_enabled = 0;
        return;
    } else if input == "enable_output" {
        *output_enabled = 1;
        return;
    }
    let mut col = String::new();
    let mut formula = String::new();
    let mut row: i32 = 0;
    {
        let mut chars = input.chars();
        while let Some(c) = chars.clone().next() {
            if c.is_ascii_alphabetic() {
                col.push(c);
                chars.next();
            } else {
                break;
            }
        }
        let rest: String = chars.collect();
        let parts: Vec<&str> = rest.split('=').collect();
        if parts.len() == 2 {
            if let Ok(r) = parts[0].trim().parse::<i32>() {
                row = r;
                formula = parts[1].trim().to_string();
            }
        }
    }
    if !col.is_empty() && row != 0 && !formula.is_empty() {
        let colIndex = get_col_index(&col);
        if colIndex > C || row > R {
            unsafe {
                inval_r = true;
            }
            return;
        }
        if colIndex >= 0 && colIndex < C && row >= 1 && row <= R {
            if check_invalid_range(&formula, row - 1, colIndex) != 0 {
                unsafe {
                    inval_r = true;
                }
                return;
            }
            if !is_valid_formula(&formula) {
                unsafe {
                    unrec_cmd = true;
                }
                return;
            }
            let r_index = (row - 1) as usize;
            let c_index = colIndex as usize;
            sheet[r_index][c_index].formula = Some(formula);
            unsafe {
                last_update.row = row - 1;
                last_update.col = colIndex;
                last_update.is_updated = true;
                unrec_cmd = false;
            }
            return;
        }
    }
    unsafe {
        last_update.is_updated = false;
        unrec_cmd = true;
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
    let idx = (row * C + col) as usize;
    if evaluated[idx] {
        return;
    }
    if let Some(ref dag) = graph[idx] {
        let mut dep = dag.dependencies.as_ref();
        while let Some(node) = dep {
            evaluate_cell(node.cell.row, node.cell.col, sheet, graph, evaluated, R, C);
            dep = node.next.as_ref().map(|b| b as &Box<Node>);
        }
    }
    if let Some(ref formula) = sheet[row as usize][col as usize].formula {
        let mut error_flag = 0;
        let val = evaluate_formula(formula, R, C, sheet, &mut error_flag);
        sheet[row as usize][col as usize].val = val;
        sheet[row as usize][col as usize].err = error_flag;
    }
    evaluated[idx] = true;
    if let Some(ref dag) = graph[idx] {
        let mut dependent = dag.dependents.as_ref();
        while let Some(node) = dependent {
            evaluate_cell(node.cell.row, node.cell.col, sheet, graph, evaluated, R, C);
            dependent = node.next.as_ref().map(|b| b as &Box<Node>);
        }
    }
}

fn evaluate_formula(
    formula: &str,
    R: i32,
    C: i32,
    sheet: &Vec<Vec<cell>>,
    error_flag: &mut i32,
) -> i32 {
    if formula.contains("(") && formula.contains(":") && formula.contains(")") {
        let open_paren = formula.find('(').unwrap();
        let func_name = &formula[0..open_paren];
        let close_paren = formula.find(')').unwrap();
        let range = &formula[open_paren + 1..close_paren];
        if func_name == "MAX"
            || func_name == "MIN"
            || func_name == "AVG"
            || func_name == "SUM"
            || func_name == "STDEV"
        {
            return evaluate_range(range, R, C, sheet, func_name, error_flag);
        }
    }
    if formula.starts_with("SLEEP(") {
        let inner = &formula[6..formula.len() - 1];
        if let Ok(value) = inner.trim().parse::<i32>() {
            let sleep_start = Instant::now();
            std::thread::sleep(std::time::Duration::from_secs(value as u64));
            let duration = sleep_start.elapsed().as_secs_f64();
            unsafe {
                sleeptimetotal += duration;
            }
            return value;
        } else {
            let mut col = String::new();
            let mut row_str = String::new();
            for c in inner.chars() {
                if c.is_ascii_alphabetic() {
                    col.push(c);
                } else {
                    break;
                }
            }
            row_str = inner[col.len()..].trim().to_string();
            if let Ok(row) = row_str.parse::<i32>() {
                let col_idx = get_col_index(&col);
                let row_idx = row - 1;
                if col_idx >= 0 && row_idx >= 0 && row_idx < R {
                    if sheet[row_idx as usize][col_idx as usize].err != 0 {
                        *error_flag = 1;
                        return 0;
                    }
                    let value = sheet[row_idx as usize][col_idx as usize].val;
                    thread_local! {
                        static PREV_SLEEP_FORMULA: RefCell<Option<String>> = RefCell::new(None);
                        static PREV_SLEEP_VALUE: RefCell<i32> = RefCell::new(-1);
                    }

                    // In your function:
                    let formula_changed = PREV_SLEEP_FORMULA.with(|prev| {
                        let prev = prev.borrow();
                        prev.as_ref().map_or(true, |p| p != formula)
                    });
                    let value_changed = PREV_SLEEP_VALUE.with(|prev_val| {
                        let prev_val = prev_val.borrow();
                        *prev_val != value
                    });

                    if formula_changed {
                        PREV_SLEEP_FORMULA.with(|prev| {
                            *prev.borrow_mut() = Some(formula.to_string());
                        });
                        PREV_SLEEP_VALUE.with(|prev_val| {
                            *prev_val.borrow_mut() = value;
                        });
                        let sleep_start = Instant::now();
                        std::thread::sleep(std::time::Duration::from_secs(value as u64));
                        let sleep_end = Instant::now();
                        let duration = sleep_end.duration_since(sleep_start).as_secs_f64();
                        // Assuming sleeptimetotal is another static, handle it separately if needed
                        unsafe {
                            sleeptimetotal += duration;
                        } // Only if sleeptimetotal is still a static mut
                    } else if value_changed {
                        PREV_SLEEP_VALUE.with(|prev_val| {
                            *prev_val.borrow_mut() = value;
                        });
                        let sleep_start = Instant::now();
                        std::thread::sleep(std::time::Duration::from_secs(value as u64));
                        let sleep_end = Instant::now();
                        let duration = sleep_end.duration_since(sleep_start).as_secs_f64();
                        unsafe {
                            sleeptimetotal += duration;
                        } // Adjust as needed
                    }
                    return value;
                }
            }
        }
    }
    let re = Regex::new(r"^\s*([A-Z]+\d+|\d+)\s*([\+\-\*/])\s*([A-Z]+\d+|\d+)\s*$").unwrap();
    if let Some(caps) = re.captures(formula) {
        let left = &caps[1];
        let op = &caps[2];
        let right = &caps[3];
        let mut left_val = 0;
        let mut right_val = 0;
        let mut left_err = 0;
        let mut right_err = 0;
        if let Ok(num) = left.parse::<i32>() {
            left_val = num;
        } else {
            left_val = get_value_from_formula(left, R, C, sheet, &mut left_err);
        }
        if let Ok(num) = right.parse::<i32>() {
            right_val = num;
        } else {
            right_val = get_value_from_formula(right, R, C, sheet, &mut right_err);
        }
        if left_err != 0 || right_err != 0 {
            *error_flag = 1;
            return 0;
        }
        return match op {
            "+" => left_val + right_val,
            "-" => left_val - right_val,
            "*" => left_val * right_val,
            "/" => {
                if right_val == 0 {
                    *error_flag = 1;
                    0
                } else {
                    left_val / right_val
                }
            }
            _ => {
                *error_flag = 1;
                0
            }
        };
    }
    if let Ok(num) = formula.trim().parse::<i32>() {
        return num;
    }
    let mut col = String::new();
    let mut row = 0;
    let mut found_letter = false;
    for c in formula.chars() {
        if c.is_ascii_alphabetic() {
            col.push(c);
            found_letter = true;
        } else if c.is_digit(10) && found_letter {
            row = row * 10 + c.to_digit(10).unwrap() as i32;
        }
    }
    if !col.is_empty() && row > 0 {
        let col_index = get_col_index(&col);
        if col_index < 0 || col_index >= C || row < 1 || row > R {
            *error_flag = 1;
            return 0;
        }
        if sheet[(row - 1) as usize][col_index as usize].err != 0 {
            *error_flag = 1;
            return 0;
        }
        return sheet[(row - 1) as usize][col_index as usize].val;
    }
    *error_flag = 1;
    0
}

fn get_dependencies(formula: &str, R: i32, C: i32) -> Vec<CellRef> {
    let mut deps = Vec::new();
    if formula.contains("(") && formula.contains(":") && formula.contains(")") {
        let open_paren = formula.find('(').unwrap();
        let close_paren = formula.find(')').unwrap();
        let range = &formula[open_paren + 1..close_paren];
        let parts: Vec<&str> = range.split(':').collect();
        if parts.len() == 2 {
            if let (Some(start), Some(end)) = (parse_cell_ref(parts[0]), parse_cell_ref(parts[1])) {
                for row in start.row..=end.row {
                    for col in start.col..=end.col {
                        if row >= 0 && row < R && col >= 0 && col < C {
                            deps.push(CellRef { row, col });
                        }
                    }
                }
            }
        }
    } else if formula.starts_with("SLEEP(") {
        let inner = &formula[6..formula.len() - 1];
        if let Some(cell_ref) = parse_cell_ref(inner) {
            if cell_ref.row >= 0 && cell_ref.row < R && cell_ref.col >= 0 && cell_ref.col < C {
                deps.push(cell_ref);
            }
        }
    } else {
        let re = Regex::new(r"([A-Z]+\d+)").unwrap();
        for cap in re.captures_iter(formula) {
            if let Some(cell_ref) = parse_cell_ref(&cap[1]) {
                if cell_ref.row >= 0 && cell_ref.row < R && cell_ref.col >= 0 && cell_ref.col < C {
                    deps.push(cell_ref);
                }
            }
        }
    }
    deps
}

fn parse_cell_ref(s: &str) -> Option<CellRef> {
    let mut col = String::new();
    let mut row = 0;
    for c in s.chars() {
        if c.is_ascii_alphabetic() {
            col.push(c);
        } else if c.is_digit(10) {
            row = row * 10 + c.to_digit(10).unwrap() as i32;
        }
    }
    if col.is_empty() || row == 0 {
        None
    } else {
        let col_index = get_col_index(&col);
        if col_index >= 0 {
            Some(CellRef {
                row: row - 1,
                col: col_index,
            })
        } else {
            None
        }
    }
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
    let new_node = Box::new(Node {
        cell: CellRef {
            row: dep_row,
            col: dep_col,
        },
        next: None,
    });
    if let Some(ref mut dag) = graph[reference_index] {
        let prev = dag.dependents.take();
        let mut node_to_insert = new_node;
        node_to_insert.next = prev;
        dag.dependents = Some(node_to_insert);
    }
    let new_dep_node = Box::new(Node {
        cell: CellRef {
            row: ref_row,
            col: ref_col,
        },
        next: None,
    });
    if let Some(ref mut dag) = graph[dependent_index] {
        let prev_deps = dag.dependencies.take();
        let mut dep_node_to_insert = new_dep_node;
        dep_node_to_insert.next = prev_deps;
        dag.dependencies = Some(dep_node_to_insert);
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
        dag.dependents = remove_from_list(
            dag.dependents.take(),
            CellRef {
                row: dep_row,
                col: dep_col,
            },
        );
    }
    if let Some(ref mut dag) = graph[dependent_index] {
        dag.dependencies = remove_from_list(
            dag.dependencies.take(),
            CellRef {
                row: ref_row,
                col: ref_col,
            },
        );
        dag.in_degree -= 1;
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
        let mut node_opt = dag.dependents.as_ref();
        while let Some(node) = node_opt {
            let next = (node.cell.row * C + node.cell.col) as usize;
            if next < total && dfs(graph, R, C, next, target, visited) {
                return true;
            }
            node_opt = node.next.as_ref().map(|b| b as &Box<Node>);
        }
    }
    false
}

fn is_reachable(
    graph: &Vec<Option<Box<DAGNode>>>,
    R: i32,
    C: i32,
    src: usize,
    target: usize,
) -> bool {
    let total = (R * C) as usize;
    if src >= total || target >= total {
        return false;
    }
    let mut visited = vec![false; total];
    dfs(graph, R, C, src, target, &mut visited)
}

fn check_invalid_range(formula: &str, current_row: i32, current_col: i32) -> i32 {
    if formula.contains("(") && formula.contains(":") && formula.contains(")") {
        let open_paren = formula.find('(').unwrap();
        let close_paren = formula.find(')').unwrap();
        let inner = &formula[open_paren + 1..close_paren];
        let parts: Vec<&str> = inner.split(':').collect();
        if parts.len() == 2 {
            let start = parse_cell_ref(parts[0]);
            let end = parse_cell_ref(parts[1]);
            if let (Some(s), Some(e)) = (start, end) {
                if s.col > e.col || s.row > e.row {
                    return 1;
                }
                if current_row >= s.row
                    && current_row <= e.row
                    && current_col >= s.col
                    && current_col <= e.col
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
        if let (Some(start), Some(end)) = (parse_cell_ref(parts[0]), parse_cell_ref(parts[1])) {
            *start_row = start.row;
            *end_row = end.row;
            *start_col = start.col;
            *end_col = end.col;
            if *start_row > *end_row || *start_col > *end_col {
                unsafe {
                    inval_r = true;
                }
                return -1;
            }
            return 0;
        }
    }
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
    if let Some(cell_ref) = parse_cell_ref(formula) {
        if cell_ref.col < 0 || cell_ref.col >= C || cell_ref.row < 0 || cell_ref.row >= R {
            *error_flag = 1;
            return 0;
        }
        if sheet[cell_ref.row as usize][cell_ref.col as usize].err != 0 {
            *error_flag = 1;
            return 0;
        }
        return sheet[cell_ref.row as usize][cell_ref.col as usize].val;
    }
    if let Ok(value) = formula.trim().parse::<i32>() {
        return value;
    }
    *error_flag = 1;
    0
}

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
    let total_cells = (R * C) as usize;
    let mut graph: Vec<Option<Box<DAGNode>>> = (0..total_cells)
        .map(|_| {
            Some(Box::new(DAGNode {
                in_degree: 0,
                dependents: None,
                dependencies: None,
            }))
        })
        .collect();
    for i in 0..total_cells {
        graph[i] = Some(Box::new(DAGNode {
            in_degree: 0,
            dependents: None,
            dependencies: None,
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
        let mut backup_formula: Option<String> = None;
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
                    let deps_to_remove: Vec<CellRef> = {
                        let mut deps = Vec::new();
                        let mut dep = dag.dependencies.take();
                        while let Some(mut node) = dep {
                            deps.push(node.cell);
                            dep = node.next.take();
                        }
                        deps
                    };
                    drop(dag); // Explicitly drop dag to release the borrow
                    for dep in deps_to_remove {
                        remove_dependency(&mut graph, row, col, dep.row, dep.col, R, C);
                    }
                    if let Some(ref mut dag) = graph[idx] {
                        dag.in_degree = 0;
                    }
                }
                let new_formula = sheet[row as usize][col as usize].formula.as_ref().unwrap();
                let new_deps = get_dependencies(new_formula, R, C);
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
