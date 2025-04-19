use regex::Regex;
use std::io::{self, BufRead, Write};
use std::process;
use std::str;
use std::time::Instant;
use std::collections::{HashMap, HashSet, VecDeque};
use std::hash::{Hash, Hasher};
mod vim_mode;

#[macro_use]
extern crate lazy_static;
use clap::{Arg, Command};
use libc;

const CLOCKS_PER_SEC: i64 = 1_000_000;
const VIEWPORT_SIZE: i32 = 10;

static mut INVAL_R: bool = false;
static mut UNREC_CMD: bool = false;
static mut SLEEPTIMETOTAL: f64 = 0.0;
static mut CYCLE_DETECTED: bool = false;

#[derive(Clone)]
pub struct cell {
    pub val: i32,
    pub formula: Option<String>,
    pub err: i32, // 1 if the cell contains an error, 0 otherwise
}

#[derive(Copy, Clone)]
pub struct CellUpdate {
    pub row: i32,
    pub col: i32,
    pub is_updated: bool,
}

static mut LAST_UPDATE: CellUpdate = CellUpdate {
    row: -1,
    col: -1,
    is_updated: false,
};

#[derive(Copy, Clone, Eq, PartialEq)]
pub struct CellRef {
    pub row: i32,
    pub col: i32,
}

impl Hash for CellRef {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.row.hash(state);
        self.col.hash(state);
    }
}

// New dependency graph structure using hash maps
pub struct DependencyGraph {
    // Map from cell index to its dependents (cells that depend on it)
    dependents: HashMap<CellRef, Vec<CellRef>>,
    // Track in-degree for topological sorting
    in_degree: HashMap<CellRef, i32>,
}

impl DependencyGraph {
    fn new() -> Self {
        DependencyGraph {
            dependents: HashMap::new(),
            in_degree: HashMap::new(),
        }
    }

    fn add_dependency(&mut self, reference: CellRef, dependent: CellRef) -> bool {
        // Check for cycles before adding dependency
        if self.is_reachable(dependent, reference) {
            unsafe { CYCLE_DETECTED = true; }
            return false;
        }

        // Add dependent to reference's dependents list
        self.dependents.entry(reference).or_insert_with(Vec::new).push(dependent);
        
        // Increment in-degree of dependent
        *self.in_degree.entry(dependent).or_insert(0) += 1;
        
        true
    }

    fn is_reachable(&self, start: CellRef, target: CellRef) -> bool {
        let mut visited = HashSet::new();
        let mut queue = VecDeque::new();
        
        queue.push_back(start);
        visited.insert(start);
        
        while let Some(current) = queue.pop_front() {
            if current == target {
                return true;
            }
            
            if let Some(deps) = self.dependents.get(&current) {
                for &next in deps {
                    if !visited.contains(&next) {
                        visited.insert(next);
                        queue.push_back(next);
                    }
                }
            }
        }
        
        false
    }
}
lazy_static! {
    static ref COLUMN_MAP: HashMap<String, i32> = {
        let mut m = HashMap::new();
        // Single letter columns (A-Z)
        for i in 0..26 {
            m.insert(((b'A' + i) as char).to_string(), i as i32);
        }
        // Two letter columns (AA-ZZ)
        for i in 0..26 {
            for j in 0..26 {
                let col = format!("{}{}", (b'A' + i) as char, (b'A' + j) as char);
                m.insert(col, 26 + i as i32 * 26 + j as i32);
            }
        }
        // Three letter columns (AAA-ZZZ)
        for i in 0..26 {
            for j in 0..26 {
                for k in 0..26 {
                    let col = format!("{}{}{}", 
                        (b'A' + i) as char, 
                        (b'A' + j) as char,
                        (b'A' + k) as char);
                    m.insert(col, 26*26 + (i as i32)*26*26 + (j as i32)*26 + k as i32);
                }
            }
        }
        m
    };
}

// Cache for range function results
struct RangeCache {
    cache: HashMap<String, i32>,
}

impl RangeCache {
    fn new() -> Self {
        RangeCache {
            cache: HashMap::new(),
        }
    }

    fn get(&self, key: &str) -> Option<i32> {
        self.cache.get(key).copied()
    }

    fn set(&mut self, key: String, value: i32) {
        self.cache.insert(key, value);
    }

    fn clear(&mut self) {
        self.cache.clear();
    }
}

// Cache for parsed formulas
#[derive(Clone)]
struct ParsedFormula {
    dependencies: Vec<CellRef>,
    operation: String,
    is_range_function: bool,
}

struct FormulaCache {
    cache: HashMap<String, ParsedFormula>,
}

impl FormulaCache {
    fn new() -> Self {
        FormulaCache {
            cache: HashMap::new(),
        }
    }

    fn get(&self, formula: &str) -> Option<&ParsedFormula> {
        self.cache.get(formula)
    }

    fn set(&mut self, formula: String, parsed: ParsedFormula) {
        self.cache.insert(formula, parsed);
    }

    fn clear(&mut self) {
        self.cache.clear();
    }
}

fn print_sheet(
    r: i32,
    c: i32,
    sheet: &Vec<Vec<cell>>,
    row_offset: i32,
    col_offset: i32,
    output_enabled: i32,
) {
    if output_enabled == 0 {
        return;
    }
    print!("  ");
    print_columns(c, col_offset);
    println!("\n");
    for i in row_offset..(row_offset + VIEWPORT_SIZE) {
        if i >= r {
            break;
        }
        print!("{}\t", i + 1);
        for j in col_offset..(col_offset + VIEWPORT_SIZE) {
            if j >= c {
                break;
            }
            if sheet[i as usize][j as usize].err != 0 {
                print!("ERR\t"); // Display "ERR" for cells with errors
            } else {
                print!("{}\t", sheet[i as usize][j as usize].val);
            }
        }
        println!();
    }
}

fn print_columns(c: i32, col_offset: i32) {
    print!("\t");
    for i in col_offset..(col_offset + VIEWPORT_SIZE) {
        if i >= c {
            break;
        }
        let mut temp = i;
        let mut col = ['\0'; 4];
        col[0] = '\0';
        col[1] = '\0';
        col[2] = '\0';
        col[3] = '\0';
        let mut index = 2;
        while temp >= 0 {
            col[index] = (b'A' + (temp % 26) as u8) as char;
            index -= 1;
            temp = (temp / 26) - 1;
            if index < 0 {
                break;
            }
        }
        // Print starting from col[index+1]
        let s: String = col[(index + 1) as usize..4].iter().collect();
        print!("{}\t", s);
    }
    println!();
}

fn process_input(
    input: &str,
    r: i32,
    c: i32,
    sheet: &mut Vec<Vec<cell>>,
    row_offset: &mut i32,
    col_offset: &mut i32,
    output_enabled: &mut i32,
) {
    unsafe {
        SLEEPTIMETOTAL = 0.0;
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
        *row_offset = if *row_offset + 10 < r {
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
        *col_offset = if *col_offset + 10 < c {
            *col_offset + 10
        } else {
            *col_offset
        };
        return;
    } else if input.starts_with("scroll_to") {
        let parts: Vec<&str> = input[10..].trim().split_whitespace().collect();
        if parts.len() >= 1 {
            // Expect format like "A1"
            let col_str: String = parts[0].chars().take_while(|c| c.is_alphabetic()).collect();
            let row_str: String = parts[0].chars().skip_while(|c| c.is_alphabetic()).collect();
            if let Ok(row) = row_str.parse::<i32>() {
                let col_index = get_col_index(&col_str);
                if col_index >= 0 && col_index < c && row >= 1 && row <= r {
                    *row_offset = row - 1;
                    *col_offset = col_index;
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
    // This replicates: if (sscanf(input, "%[A-Z]%d=%[^\n]", col, &row, formula) == 3)
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
            let row_part = parts[0].trim();
            let formula_part = parts[1].trim();
            if let Ok(r) = row_part.parse::<i32>() {
                row = r;
                formula = formula_part.to_string();
            }
        }
    }
    if !col.is_empty() && row != 0 && !formula.is_empty() {
        let col_index = get_col_index(&col);
        if col_index > c || row > r {
            unsafe {
                INVAL_R = true;
            }
            return;
        }
        if col_index >= 0 && col_index < c && row >= 1 && row <= r {
            // Check for invalid range before storing the formula
            if check_invalid_range(&formula) != 0 {
                unsafe {
                    INVAL_R = true;
                }
                // Don't store the formula if range is invalid
                return;
            }
            // Store the formula only if range is valid
            let r_index = (row - 1) as usize;
            let c_index = col_index as usize;
            sheet[r_index][c_index].formula = Some(formula.to_string());
            unsafe {
                LAST_UPDATE.row = row - 1;
                LAST_UPDATE.col = col_index;
                LAST_UPDATE.is_updated = true;
                UNREC_CMD = false;
            }
            return;
        }
    }
    unsafe {
        LAST_UPDATE.is_updated = false;
        UNREC_CMD = true;
    }
}

fn get_col_index(col: &str) -> i32 {
    // Use the cached column map for O(1) lookup
    if let Some(&index) = COLUMN_MAP.get(col) {
        return index;
    }
    
    // Fallback to calculation if not in cache
    let mut index: i32 = 0;
    for c in col.chars() {
        if c >= 'A' && c <= 'Z' {
            index = index * 26 + (c as i32 - 'A' as i32 + 1);
        } else {
            return -1;
        }
    }
    return index - 1;
}

fn evaluate_sheet(r: i32, c: i32, sheet: &mut Vec<Vec<cell>>) {
    let mut graph = DependencyGraph::new();
    let mut formula_cache = FormulaCache::new();
    
    // Build dependency graph
    build_dependency_graph(r, c, sheet, &mut graph, &mut formula_cache);
    
    // Evaluate cells in topological order
    topological_evaluation(r, c, sheet, &graph);
}

fn evaluate_formula(
    formula: &str,
    r: i32,
    c: i32,
    sheet: &Vec<Vec<cell>>,
    error_flag: &mut i32,
    range_cache: &mut RangeCache,
) -> i32 {
    // First, check if it's a range function like MAX, MIN, AVG, etc.
    if formula.contains("(") && formula.contains(":") && formula.contains(")") {
        let open_paren = formula.find('(').unwrap();
        let func_name = &formula[0..open_paren];
        let close_paren = formula.find(')').unwrap();
        let range = &formula[open_paren + 1..close_paren];

        // Check if it's a valid range function
        if func_name == "MAX"
            || func_name == "MIN"
            || func_name == "AVG"
            || func_name == "SUM"
            || func_name == "STDEV"
        {
            // Check cache first
            let cache_key = format!("{}:{}", range, func_name);
            if let Some(result) = range_cache.get(&cache_key) {
                return result;
            }
            
            let result = evaluate_range(range, r, c, sheet, func_name);
            
            // Cache the result
            range_cache.set(cache_key, result);
            return result;
        }
    }

    if formula.starts_with("SLEEP(") {
        let inner = &formula[6..formula.len() - 1];
        // In evaluate_formula() SLEEP handling:
        if let Ok(value) = inner.trim().parse::<i32>() {
            // Handling SLEEP(value)
            let sleep_start = Instant::now();
            std::thread::sleep(std::time::Duration::from_secs(value as u64));
            let sleep_end = Instant::now();
            let duration = sleep_end.duration_since(sleep_start).as_secs_f64();
            unsafe {
                SLEEPTIMETOTAL += duration; // Update only for SLEEP
            }
            return value;
        } else {
            // For SLEEP with a cell reference, pattern: SLEEP(%[A-Z]%d)
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
                if col_idx >= 0 && row_idx >= 0 && row_idx < r {
                    if sheet[row_idx as usize][col_idx as usize].err != 0 {
                        *error_flag = 1; // Propagate error from referenced cell
                        return 0;
                    }
                    let value = sheet[row_idx as usize][col_idx as usize].val;
                    // Static variables for previous sleep formula and value
                    static mut PREV_SLEEP_FORMULA: Option<String> = None;
                    static mut PREV_SLEEP_VALUE: i32 = -1;
                    unsafe {
                        let formula_changed = if let Some(ref prev) = PREV_SLEEP_FORMULA {
                            prev != formula
                        } else {
                            true
                        };
                        if formula_changed {
                            PREV_SLEEP_FORMULA = Some(formula.to_string());
                            PREV_SLEEP_VALUE = value;
                            let sleep_start = Instant::now();
                            std::thread::sleep(std::time::Duration::from_secs(value as u64));
                            let sleep_end = Instant::now();
                            let duration = sleep_end.duration_since(sleep_start).as_secs_f64();
                            SLEEPTIMETOTAL += duration; // Update only for SLEEP
                        } else {
                            // Same formula as before. If its argument value differs from the last time, then sleep again.
                            if value != PREV_SLEEP_VALUE {
                                PREV_SLEEP_VALUE = value;
                                let sleep_start = Instant::now();
                                std::thread::sleep(std::time::Duration::from_secs(value as u64));
                                let sleep_end = Instant::now();
                                let duration = sleep_end.duration_since(sleep_start).as_secs_f64();
                                SLEEPTIMETOTAL += duration; // Update only for SLEEP
                            }
                        }
                    }
                    return value;
                }
            }
        }
    }
    
    // Regex to match expressions like A1+A2, 2+A3, A4+3, 3+4, etc.
    let re = Regex::new(r"^\s*([A-Z]+\d+|\d+)\s*([\+\-\*/])\s*([A-Z]+\d+|\d+)\s*$").unwrap();

    if let Some(caps) = re.captures(formula) {
        let left = &caps[1];
        let op = &caps[2];
        let right = &caps[3];

        let mut left_val = 0;
        let mut right_val = 0;
        let mut left_err = 0;
        let mut right_err = 0;

        // Parse left operand
        if let Ok(num) = left.parse::<i32>() {
            left_val = num;
        } else {
            left_val = get_value_from_cell_ref(left, r, c, sheet, &mut left_err);
        }

        // Parse right operand
        if let Ok(num) = right.parse::<i32>() {
            right_val = num;
        } else {
            right_val = get_value_from_cell_ref(right, r, c, sheet, &mut right_err);
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

    // For direct number conversion as in case 9: Direct number (e.g., "42") - No dependency
    if let Ok(num) = formula.trim().parse::<i32>() {
        return num;
    }

    // Check if it's a simple cell reference (e.g., "A1")
    let cell_ref = parse_cell_reference(formula);
    if let Some((col, row)) = cell_ref {
        let col_index = get_col_index(&col);
        if col_index < 0 || col_index >= c || row < 1 || row > r {
            *error_flag = 1;
            return 0;
        }

        if sheet[(row - 1) as usize][col_index as usize].err != 0 {
            *error_flag = 1;
            return 0;
        }

        return sheet[(row - 1) as usize][col_index as usize].val;
    }

    // If we get here, it's not a valid formula
    *error_flag = 1;
    0
}
    
    // Helper function to parse cell references more efficiently
    fn parse_cell_reference(reference: &str) -> Option<(String, i32)> {
        let mut col = String::new();
        let mut row_str = String::new();
        let mut found_letter = false;
    
        for c in reference.trim().chars() {
            if c.is_ascii_alphabetic() {
                col.push(c);
                found_letter = true;
            } else if c.is_digit(10) && found_letter {
                row_str.push(c);
            }
        }
    
        if !col.is_empty() && !row_str.is_empty() {
            if let Ok(row) = row_str.parse::<i32>() {
                return Some((col, row));
            }
        }
        
        None
    }
    
    // Get value from a cell reference with error checking
    fn get_value_from_cell_ref(
        reference: &str,
        r: i32,
        c: i32,
        sheet: &Vec<Vec<cell>>,
        error_flag: &mut i32,
    ) -> i32 {
        if let Some((col, row)) = parse_cell_reference(reference) {
            let col_index = get_col_index(&col);
            if col_index < 0 || col_index >= c || row < 1 || row > r {
                *error_flag = 1;
                return 0;
            }
    
            if sheet[(row - 1) as usize][col_index as usize].err != 0 {
                *error_flag = 1;
                return 0;
            }
    
            return sheet[(row - 1) as usize][col_index as usize].val;
        }
    
        // Try to parse as a direct number
        if let Ok(value) = reference.trim().parse::<i32>() {
            return value;
        }
    
        // If we get here, it's not a valid reference
        *error_flag = 1;
        0
    }
    
    fn build_dependency_graph(
        r: i32,
        c: i32,
        sheet: &Vec<Vec<cell>>,
        graph: &mut DependencyGraph,
        formula_cache: &mut FormulaCache,
    ) {
        // Parse formulas to populate graph
        for i in 0..r {
            for j in 0..c {
                if let Some(ref formula) = sheet[i as usize][j as usize].formula {
                    let dependent = CellRef { row: i, col: j };
                    
                    // Check if formula is in cache
                    if let Some(parsed) = formula_cache.get(formula) {
                        // Use cached dependencies
                        for &reference in &parsed.dependencies {
                            graph.add_dependency(reference, dependent);
                        }
                        continue;
                    }
                    
                    // Parse formula and extract dependencies
                    let mut dependencies = Vec::new();
                    
                    // Check for range functions
                    if formula.contains("(") && formula.contains(":") && formula.contains(")") {
                        let open_paren = formula.find('(').unwrap();
                        let func_name = &formula[0..open_paren];
                        let close_paren = formula.find(')').unwrap();
                        let range = &formula[open_paren + 1..close_paren];
                        
                        if func_name == "MAX" || func_name == "MIN" || func_name == "AVG" 
                           || func_name == "SUM" || func_name == "STDEV" {
                            let mut start_row = 0;
                            let mut end_row = 0;
                            let mut start_col = 0;
                            let mut end_col = 0;
                            
                            if parse_range(range, &mut start_row, &mut end_row, &mut start_col, &mut end_col) == 0 {
                                for row in start_row..=end_row {
                                    for col in start_col..=end_col {
                                        let reference = CellRef { row, col };
                                        dependencies.push(reference);
                                        graph.add_dependency(reference, dependent);
                                    }
                                }
                            }
                        }
                    }
                    // Check for SLEEP with cell reference
                    else if formula.starts_with("SLEEP(") {
                        let inner = &formula[6..formula.len() - 1];
                        if let Some((col, row)) = parse_cell_reference(inner) {
                            let col_idx = get_col_index(&col);
                            if col_idx >= 0 && row > 0 && row <= r {
                                let reference = CellRef { row: row - 1, col: col_idx };
                                dependencies.push(reference);
                                graph.add_dependency(reference, dependent);
                            }
                        }
                    }
                    // Check for binary operations
                    else {
                        let re = Regex::new(r"^\s*([A-Z]+\d+|\d+)\s*([\+\-\*/])\s*([A-Z]+\d+|\d+)\s*$").unwrap();
                        if let Some(caps) = re.captures(formula) {
                            let left = &caps[1];
                            let right = &caps[3];
                            
                            // Check if left operand is a cell reference
                            if let Some((col, row)) = parse_cell_reference(left) {
                                let col_idx = get_col_index(&col);
                                if col_idx >= 0 && row > 0 && row <= r {
                                    let reference = CellRef { row: row - 1, col: col_idx };
                                    dependencies.push(reference);
                                    graph.add_dependency(reference, dependent);
                                }
                            }
                            
                            // Check if right operand is a cell reference
                            if let Some((col, row)) = parse_cell_reference(right) {
                                let col_idx = get_col_index(&col);
                                if col_idx >= 0 && row > 0 && row <= r {
                                    let reference = CellRef { row: row - 1, col: col_idx };
                                    dependencies.push(reference);
                                    graph.add_dependency(reference, dependent);
                                }
                            }
                        }
                        // Check for single cell reference
                        else if let Some((col, row)) = parse_cell_reference(formula) {
                            let col_idx = get_col_index(&col);
                            if col_idx >= 0 && row > 0 && row <= r {
                                let reference = CellRef { row: row - 1, col: col_idx };
                                dependencies.push(reference);
                                graph.add_dependency(reference, dependent);
                            }
                        }
                    }
                    
                    // Cache the parsed formula
                    let parsed = ParsedFormula {
                        dependencies,
                        operation: formula.to_string(),
                        is_range_function: formula.contains("(") && formula.contains(":") && formula.contains(")"),
                    };
                    formula_cache.set(formula.clone(), parsed);
                }
            }
        }
    }
    
    fn topological_evaluation(
        r: i32,
        c: i32,
        sheet: &mut Vec<Vec<cell>>,
        graph: &DependencyGraph,
    ) {
        let mut range_cache = RangeCache::new();
        
        // Find cells with no dependencies (in_degree = 0)
        let mut queue = VecDeque::new();
        let mut in_degree = graph.in_degree.clone();
        
        // Initialize queue with cells that have no dependencies
        for i in 0..r {
            for j in 0..c {
                let cell_ref = CellRef { row: i, col: j };
                if in_degree.get(&cell_ref).copied().unwrap_or(0) == 0 {
                    queue.push_back(cell_ref);
                }
            }
        }
        
        // Process cells in topological order
        while let Some(cell_ref) = queue.pop_front() {
            let i = cell_ref.row;
            let j = cell_ref.col;
            
            // Evaluate the cell if it has a formula
            if let Some(ref formula) = sheet[i as usize][j as usize].formula {
                let mut error_flag = 0;
                let val = evaluate_formula(formula, r, c, sheet, &mut error_flag, &mut range_cache);
                sheet[i as usize][j as usize].val = val;
                sheet[i as usize][j as usize].err = error_flag;
            }
            
            // Update dependents
            if let Some(dependents) = graph.dependents.get(&cell_ref) {
                for &dependent in dependents {
                    if let Some(degree) = in_degree.get_mut(&dependent) {
                        *degree -= 1;
                        if *degree == 0 {
                            queue.push_back(dependent);
                        }
                    }
                }
            }
        }
    }
    
    fn check_invalid_range(formula: &str) -> i32 {
        let mut function = String::new();
        let mut col1 = String::new();
        let mut col2 = String::new();
        let mut row1: i32 = 0;
        let mut row2: i32 = 0;
        // Check for range functions (MIN, MAX, AVG, SUM, STDEV)
        if formula.contains("(") && formula.contains(":") && formula.contains(")") {
            let open_paren = formula.find('(').unwrap();
            function = formula[0..open_paren].to_string();
            let close_paren = formula.find(')').unwrap();
            let inner = &formula[open_paren + 1..close_paren];
            let parts: Vec<&str> = inner.split(':').collect();
            if parts.len() == 2 {
                let part1 = parts[0];
                let part2 = parts[1];
                
                if let Some((c1, r1)) = parse_cell_reference(part1) {
                    col1 = c1;
                    row1 = r1;
                }
                
                if let Some((c2, r2)) = parse_cell_reference(part2) {
                    col2 = c2;
                    row2 = r2;
                }
                
                let c1 = get_col_index(&col1);
                let c2 = get_col_index(&col2);
                // Check if range is invalid (C1 > C2 or R1 > R2)
                if c1 > c2 || row1 > row2 {
                    return 1; // Invalid range
                }
            }
        }
        0 // Valid range or not a range formula
    }
    
    fn evaluate_range(range: &str, r: i32, c: i32, sheet: &Vec<Vec<cell>>, func: &str) -> i32 {
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
                INVAL_R = true;
            }
            return 0; // Error in range
        }
    
        let total_cells = (end_row - start_row + 1) * (end_col - start_col + 1);
        let mut values: Vec<i32> = Vec::with_capacity(total_cells as usize);
    
        // Iterate over the range and check for error cells
        for i in start_row..=end_row {
            for j in start_col..=end_col {
                if sheet[i as usize][j as usize].err != 0 {
                    // If any cell is already in error, indicate error for the range
                    unsafe {
                        INVAL_R = true;
                    }
                    return 0;
                }
                values.push(sheet[i as usize][j as usize].val);
            }
        }
    
        let count = values.len();
        let mut result = 0;
    
        if func == "SUM" {
            for val in &values {
                result += val;
            }
        } else if func == "AVG" {
            if count > 0 {
                for val in &values {
                    result += val;
                }
                result /= count as i32;
            }
        } else if func == "MIN" {
            if count > 0 {
                result = std::i32::MAX;
                for val in &values {
                    if *val < result {
                        result = *val;
                    }
                }
            }
        } else if func == "MAX" {
            if count > 0 {
                result = std::i32::MIN;
                for val in &values {
                    if *val > result {
                        result = *val;
                    }
                }
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
        // Check if it's a range (A1:B2 format)
        if let Some(colon_pos) = range.find(':') {
            let (first_part, second_part) = range.split_at(colon_pos);
            let second_part = &second_part[1..]; // Skip the colon
    
            // Parse first part (A1)
            if let Some((col1, row1)) = parse_cell_reference(first_part) {
                *start_row = row1 - 1;
                *start_col = get_col_index(&col1);
            } else {
                unsafe { INVAL_R = true; }
                return -1;
            }
    
            // Parse second part (B2)
            if let Some((col2, row2)) = parse_cell_reference(second_part) {
                *end_row = row2 - 1;
                *end_col = get_col_index(&col2);
            } else {
                unsafe { INVAL_R = true; }
                return -1;
            }
    
            if *start_row > *end_row || *start_col > *end_col {
                unsafe { INVAL_R = true; }
                return -1; // Invalid range
            }
        } else {
            // Single cell reference (A1 format)
            if let Some((col, row)) = parse_cell_reference(range) {
                *start_row = row - 1;
                *end_row = row - 1;
                *start_col = get_col_index(&col);
                *end_col = get_col_index(&col);
            } else {
                unsafe { INVAL_R = true; }
                return -1; // Invalid range
            }
        }
    
        0 // Success
    }
    
    fn stdev(values: &Vec<i32>) -> i32 {
        let count = values.len();
        if count <= 1 {
            // Need at least 2 values for standard deviation
            return 0;
        }
    
        // Calculate mean
        let mut mean = 0.0;
        for val in values {
            mean += *val as f64;
        }
        mean /= count as f64;
    
        // Calculate sum of squared differences from mean
        let mut sum_squared_diff = 0.0;
        for val in values {
            let diff = *val as f64 - mean;
            sum_squared_diff += diff * diff;
        }
    
        // Calculate standard deviation
        // Using population standard deviation formula: sqrt(Σ(x - μ)²/n)
        let stdev = (sum_squared_diff / count as f64).sqrt();
    
        // Round to nearest integer
        (stdev + 0.5) as i32
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
        let rows = if let Some(r) = matches.get_one::<String>("rows") {
            r.parse::<i32>().unwrap_or(20)
        } else if let Some(r) = matches.get_one::<String>("R") {
            r.parse::<i32>().unwrap_or(20)
        } else {
            20 // Default
        };
    
        let cols = if let Some(c) = matches.get_one::<String>("cols") {
            c.parse::<i32>().unwrap_or(20)
        } else if let Some(c) = matches.get_one::<String>("C") {
            c.parse::<i32>().unwrap_or(20)
        } else {
            20 // Default
        };
    
        // Validate dimensions
        if rows < 1 || rows > 100000 || cols < 1 || cols > (26 * 26 * 26 + 26 * 26 + 26) {
            println!("Invalid grid size.");
            process::exit(1);
        }
    
        // If vim mode is enabled, run the vim interface
        if vim_mode {
            vim_mode::editor::run_vim_interface(rows, cols);
            return;
        }
    
        // Original code path for standard mode
        // Allocate sheet as a contiguous 2D vector.
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
    
        let mut row_offset: i32 = 0;
        let mut col_offset: i32 = 0;
        let mut output_enabled: i32 = 1;
        let mut range_cache = RangeCache::new();
        let mut formula_cache = FormulaCache::new();
    
        print_sheet(rows, cols, &sheet, row_offset, col_offset, output_enabled);
    
        let stdin = io::stdin();
        let mut input_line = String::new();
        print!("[0.0] (ok) > ");
        io::stdout().flush().unwrap();
    
        while let Ok(n) = stdin.lock().read_line(&mut input_line) {
            if n == 0 {
                break;
            }
    
            if let Some(pos) = input_line.find('\n') {
                input_line.replace_range(pos..pos + 1, "");
            }
    
            unsafe {
                SLEEPTIMETOTAL = 0.0;
                INVAL_R = false;
                UNREC_CMD = false;
                CYCLE_DETECTED = false;
                LAST_UPDATE.is_updated = false;
            }
    
            // Clear caches when input changes
            range_cache.clear();
    
            // ----- Backup update command info if applicable -----
            // If the command is of the form "A1=..." then store a backup of the cell's old formula.
            let mut updated_row: i32 = -1;
            let mut updated_col: i32 = -1;
            let mut backup_formula = String::new(); // Adjust size as needed.
    
            {
                let mut col_str = String::new();
                let mut new_formula = String::new();
                let mut row: i32 = 0;
                // Parse command of form "A1=..."
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
                    let row_part = parts[0].trim();
                    new_formula = parts[1].trim().to_string();
                    if let Ok(r) = row_part.parse::<i32>() {
                        row = r;
                    }
                }
    
                updated_col = get_col_index(&col_str);
                updated_row = row - 1;
    
                if updated_col >= 0 && updated_col < cols && row >= 1 && row <= rows {
                    if let Some(ref s) = sheet[updated_row as usize][updated_col as usize].formula {
                        backup_formula = s.clone(); // Copy the old formula (if any) into backup_formula.
                    }
                }
            }
    
            let start = Instant::now();
    
            // Process the user input (this may update a cell's formula, adjust scrolling, etc.)
            process_input(
                &input_line,
                rows,
                cols,
                &mut sheet,
                &mut row_offset,
                &mut col_offset,
                &mut output_enabled,
            );
    
            // Build a dependency graph to check for cycles and evaluate cells
            let mut graph = DependencyGraph::new();
            
            build_dependency_graph(rows, cols, &sheet, &mut graph, &mut formula_cache);
    
            unsafe {
                if CYCLE_DETECTED && updated_row != -1 && updated_col != -1 {
                    // Cycle detected - update rejected.
                    sheet[updated_row as usize][updated_col as usize].formula = None;
                    if backup_formula.len() > 0 {
                        sheet[updated_row as usize][updated_col as usize].formula =
                            Some(backup_formula.clone());
                    } else {
                        sheet[updated_row as usize][updated_col as usize].formula = None;
                    }
                } else if !CYCLE_DETECTED {
                    SLEEPTIMETOTAL = 0.0;
                    topological_evaluation(rows, cols, &mut sheet, &graph);
                }
            }
    
            if output_enabled != 0 {
                print_sheet(rows, cols, &sheet, row_offset, col_offset, output_enabled);
            }
    
            let end = Instant::now();
    
            {
                let mut col_str = String::new();
                let mut dummy_formula = String::new();
                let mut row: i32 = 0;
                // Check for invalid range update command
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
                    let row_part = parts[0].trim();
                    dummy_formula = parts[1].trim().to_string();
                    if let Ok(r) = row_part.parse::<i32>() {
                        row = r;
                    }
                }
    
                if unsafe { INVAL_R } && !col_str.is_empty() && row != 0 {
                    let updated_col = get_col_index(&col_str);
                    if updated_col >= 0 && updated_col < cols && row >= 1 && row <= rows {
                        sheet[(row - 1) as usize][updated_col as usize].formula = None;
                    }
                }
            }
    
            unsafe {
                let sleep_time = SLEEPTIMETOTAL; // Copy the value to a local variable
                SLEEPTIMETOTAL = 0.0; // Reset immediately
                print!("[{:.2}]", sleep_time); // Reset after printing
                if UNREC_CMD {
                    print!(" (unrecognized cmd) > ");
                } else if INVAL_R {
                    print!(" (Invalid range) > ");
                } else if CYCLE_DETECTED {
                    print!(" (Cycle Detected, change cmd) > ");
                } else {
                    print!(" (ok) > ");
                }
            }
    
            io::stdout().flush().unwrap();
            input_line.clear();
        }
    }
        
        fn clock() -> i64 {
            unsafe { libc::time(std::ptr::null_mut()) as i64 }
        }

    