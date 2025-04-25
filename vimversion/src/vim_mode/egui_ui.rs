//! egui-based UI for the Vim-like interface.

use eframe::{App, Frame, NativeOptions};
use egui::viewport::ViewportCommand;
use egui::{Color32, FontId, RichText, Ui, Vec2};
use std::sync::{Arc, Mutex};

use crate::vim_mode::commands::{execute_command, find_next_match};
use crate::vim_mode::editor::{
    column_name, formula_to_string, parse_cell_reference, ClipboardContent, EditorState, Mode,
};
use crate::{cell, evaluate_cell, parse_formula, Formula, HashSet};

const CELL_SIZE: f32 = 80.0;
const HEADER_SIZE: f32 = 30.0;
const ROW_HEADER_WIDTH: f32 = 50.0;

pub struct SpreadsheetApp {
    state: EditorState,
    sheet: Arc<Mutex<Vec<Vec<cell>>>>,
    rows: i32,
    cols: i32,
    visible_rows: usize,
    visible_cols: usize,
    formula_editor: String,
    command_editor: String,
    should_exit: bool,
    numeric_prefix: String,
}

impl SpreadsheetApp {
    pub fn new(rows: i32, cols: i32, sheet: Arc<Mutex<Vec<Vec<cell>>>>) -> Self {
        Self {
            state: EditorState::new(),
            sheet,
            rows,
            cols,
            visible_rows: 20,
            visible_cols: 10,
            formula_editor: String::new(),
            command_editor: String::new(),
            should_exit: false,
            numeric_prefix: String::new(),
        }
    }
    fn render_title_bar(&self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.heading("Vim Spreadsheet");

            ui.add_space(20.0);

            // Add a prominent mode indicator
            ui.label("Mode:");
            match self.state.mode {
                Mode::Normal => {
                    ui.label(
                        RichText::new(" NORMAL ")
                            .strong()
                            .size(16.0)
                            .background_color(Color32::DARK_BLUE)
                            .color(Color32::WHITE),
                    );
                }
                Mode::Insert => {
                    ui.label(
                        RichText::new(" INSERT ")
                            .strong()
                            .size(16.0)
                            .background_color(Color32::DARK_GREEN)
                            .color(Color32::WHITE),
                    );
                }
                Mode::Command => {
                    ui.label(
                        RichText::new(" COMMAND ")
                            .strong()
                            .size(16.0)
                            .background_color(Color32::DARK_RED)
                            .color(Color32::WHITE),
                    );
                }
                Mode::Visual { .. } => {
                    ui.label(
                        RichText::new(" VISUAL ")
                            .strong()
                            .size(16.0)
                            .background_color(Color32::DARK_GREEN)
                            .color(Color32::WHITE),
                    );
                }
            }
        });
        ui.separator();
    }
    fn handle_keyboard_input(&mut self, ctx: &egui::Context) {
        // First, handle operations that need to modify the sheet directly
        let mut need_to_delete_row = false;
        let mut need_to_copy_row = false;
        let mut need_to_delete_column = false;
        let mut need_to_copy_column = false;

        if ctx.input(|i| i.key_pressed(egui::Key::R) && i.modifiers.ctrl) {
            need_to_delete_row = true;
        }

        if ctx.input(|i| i.key_pressed(egui::Key::M) && i.modifiers.ctrl) {
            // Set a flag or timer to detect if R is pressed next

            need_to_copy_row = true;
        }

        // Check for Ctrl+C to delete current column
        if ctx.input(|i| i.key_pressed(egui::Key::C) && i.modifiers.ctrl) {
            need_to_delete_column = true;
        }

        // Check for Ctrl+S to yank (copy) current column
        if ctx.input(|i| i.key_pressed(egui::Key::S) && i.modifiers.ctrl) {
            need_to_copy_column = true;
        }

        // Execute row/column operations before locking the sheet
        if need_to_delete_row {
            self.delete_row(self.state.cursor_row, 1);
            self.state.status_message = format!("Deleted row {}", self.state.cursor_row + 1);
        } else if need_to_copy_row {
            self.copy_row(self.state.cursor_row, 1);
            self.state.status_message = format!("Yanked row {}", self.state.cursor_row + 1);
        } else if need_to_delete_column {
            self.delete_column(self.state.cursor_col, 1);
            self.state.status_message =
                format!("Deleted column {}", column_name(self.state.cursor_col));
        } else if need_to_copy_column {
            self.copy_column(self.state.cursor_col, 1);
            self.state.status_message =
                format!("Yanked column {}", column_name(self.state.cursor_col));
        }

        // Now handle the rest of the input with the sheet locked
        let mut sheet = self.sheet.lock().unwrap();

        // Handle key events based on current mode
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            match self.state.mode {
                Mode::Insert => {
                    self.state.apply_edit(&mut sheet);
                    self.state.enter_normal_mode();
                }
                Mode::Command => {
                    self.state.enter_normal_mode();
                }
                Mode::Visual { .. } => {
                    self.state.enter_normal_mode();
                }
                _ => {}
            }
        }

        if let Mode::Normal = self.state.mode {
            if ctx.input(|i| {
                for event in &i.events {
                    if let egui::Event::Text(text) = event {
                        if text == "/" || text == "?" {
                            return true;
                        }
                    }
                }
                false
            }) {
                let is_forward = ctx.input(|i| {
                    for event in &i.events {
                        if let egui::Event::Text(text) = event {
                            return text == "/";
                        }
                    }
                    false
                });

                self.state.enter_command_mode();
                if is_forward {
                    self.command_editor = "/".to_string();
                } else {
                    self.command_editor = "?".to_string();
                }
                self.state.command_buffer = self.command_editor.clone();
            }

            // Handle 'n' and 'N' for next/previous search match
            if ctx.input(|i| i.key_pressed(egui::Key::N)) {
                let forward = !ctx.input(|i| i.modifiers.shift);

                if self.state.search_pattern.is_none() {
                    self.state.status_message = "No previous search".to_string();
                } else {
                    if !find_next_match(&mut self.state, forward) {
                        self.state.status_message = "Pattern not found".to_string();
                    }
                }
            }
        }

        // Normal mode key handling
        if let Mode::Normal = self.state.mode {
            if ctx.input(|i| i.key_pressed(egui::Key::I)) {
                self.state.enter_insert_mode(&sheet);
                self.formula_editor = self.state.edit_buffer.clone();
            } else if ctx.input(|i| {
                for event in &i.events {
                    if let egui::Event::Text(text) = event {
                        if text == ":" {
                            return true;
                        }
                    }
                }
                false
            }) {
                self.state.enter_command_mode();
                self.command_editor = self.state.command_buffer.clone();
            } else if ctx.input(|i| i.key_pressed(egui::Key::V)) {
                self.state.enter_visual_mode();
                self.state.status_message = String::from("-- VISUAL --");
            } else if ctx.input(|i| i.key_pressed(egui::Key::H) && i.modifiers.ctrl) {
                let visible_cols = self.visible_cols;
                self.state.col_offset = self.state.col_offset.saturating_sub(visible_cols);
                self.state.cursor_col = self.state.col_offset;

                // Update status message
                self.state.status_message = String::from("Page left");
            } else if ctx.input(|i| i.key_pressed(egui::Key::H)) {
                if self.state.cursor_col > 0 {
                    self.state.cursor_col -= 1;
                    if self.state.cursor_col < self.state.col_offset {
                        self.state.col_offset = self.state.cursor_col;
                    }
                }
            } else if ctx.input(|i| i.key_pressed(egui::Key::L) && i.modifiers.ctrl) {
                let visible_cols = self.visible_cols;
                let new_offset =
                    (self.state.col_offset + visible_cols).min(self.cols as usize - visible_cols);

                self.state.col_offset = new_offset;
                self.state.cursor_col = self.state.col_offset;

                // Update status message
                self.state.status_message = String::from("Page right");
            } else if ctx.input(|i| i.key_pressed(egui::Key::L)) {
                if self.state.cursor_col + 1 < self.cols as usize {
                    self.state.cursor_col += 1;
                    if self.state.cursor_col >= self.state.col_offset + self.visible_cols {
                        self.state.col_offset += 1;
                    }
                }
            } else if ctx.input(|i| i.key_pressed(egui::Key::J)) {
                if self.state.cursor_row + 1 < self.rows as usize {
                    self.state.cursor_row += 1;
                    if self.state.cursor_row >= self.state.row_offset + self.visible_rows {
                        self.state.row_offset += 1;
                    }
                }
            } else if ctx.input(|i| i.key_pressed(egui::Key::K)) {
                if self.state.cursor_row > 0 {
                    self.state.cursor_row -= 1;
                    if self.state.cursor_row < self.state.row_offset {
                        self.state.row_offset = self.state.cursor_row;
                    }
                }
            } else if ctx.input(|i| i.key_pressed(egui::Key::Y) && !i.modifiers.ctrl) {
                // Yank (copy) current cell - only if not part of Ctrl+Y combination
                let formula_str = if let Some(ref formula) =
                    sheet[self.state.cursor_row][self.state.cursor_col].formula
                {
                    formula_to_string(formula)
                } else {
                    sheet[self.state.cursor_row][self.state.cursor_col]
                        .val
                        .to_string()
                };

                self.state.clipboard = Some(ClipboardContent::Cell {
                    row: self.state.cursor_row,
                    col: self.state.cursor_col,
                    value: formula_str,
                });

                self.state.status_message = format!(
                    "Yanked cell {}{}",
                    column_name(self.state.cursor_col),
                    self.state.cursor_row + 1
                );
            } else if ctx.input(|i| i.key_pressed(egui::Key::P)) {
                // Paste from clipboard
                if let Some(ref content) = self.state.clipboard {
                    match content {
                        ClipboardContent::Cell { value, .. } => {
                            // Try to parse the formula string
                            match parse_formula(value) {
                                Ok(formula) => {
                                    sheet[self.state.cursor_row][self.state.cursor_col].formula =
                                        Some(formula);
                                    sheet[self.state.cursor_row][self.state.cursor_col].err = 0;
                                    self.state.status_message = format!(
                                        "Pasted to {}{}",
                                        column_name(self.state.cursor_col),
                                        self.state.cursor_row + 1
                                    );
                                }
                                Err(_) => {
                                    // If parsing fails, try to interpret as a number
                                    if let Ok(val) = value.parse::<i32>() {
                                        sheet[self.state.cursor_row][self.state.cursor_col]
                                            .formula = None;
                                        sheet[self.state.cursor_row][self.state.cursor_col].val =
                                            val;
                                        sheet[self.state.cursor_row][self.state.cursor_col].err = 0;
                                    } else {
                                        // Set as error
                                        sheet[self.state.cursor_row][self.state.cursor_col]
                                            .formula = None;
                                        sheet[self.state.cursor_row][self.state.cursor_col].val = 0;
                                        sheet[self.state.cursor_row][self.state.cursor_col].err = 1;
                                    }
                                }
                            }
                        }
                        ClipboardContent::Range {
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
                                    let target_row = self.state.cursor_row + i;
                                    let target_col = self.state.cursor_col + j;

                                    if target_row < self.rows as usize
                                        && target_col < self.cols as usize
                                    {
                                        match parse_formula(&data[i][j]) {
                                            Ok(formula) => {
                                                sheet[target_row][target_col].formula =
                                                    Some(formula);
                                                sheet[target_row][target_col].err = 0;
                                            }
                                            Err(_) => {
                                                if let Ok(val) = data[i][j].parse::<i32>() {
                                                    sheet[target_row][target_col].formula = None;
                                                    sheet[target_row][target_col].val = val;
                                                    sheet[target_row][target_col].err = 0;
                                                } else {
                                                    sheet[target_row][target_col].formula = None;
                                                    sheet[target_row][target_col].val = 0;
                                                    sheet[target_row][target_col].err = 1;
                                                }
                                            }
                                        }
                                    }
                                }
                            }

                            self.state.status_message = format!(
                                "Pasted range to {}{}",
                                column_name(self.state.cursor_col),
                                self.state.cursor_row + 1
                            );
                        }
                        ClipboardContent::Row { row: _, data } => {
                            // Paste row data
                            let cols = self.cols as usize;

                            for (j, value) in data.iter().enumerate() {
                                if j < cols {
                                    match parse_formula(value) {
                                        Ok(formula) => {
                                            sheet[self.state.cursor_row][j].formula = Some(formula);
                                            sheet[self.state.cursor_row][j].err = 0;
                                        }
                                        Err(_) => {
                                            if let Ok(val) = value.parse::<i32>() {
                                                sheet[self.state.cursor_row][j].formula = None;
                                                sheet[self.state.cursor_row][j].val = val;
                                                sheet[self.state.cursor_row][j].err = 0;
                                            } else {
                                                sheet[self.state.cursor_row][j].formula = None;
                                                sheet[self.state.cursor_row][j].val = 0;
                                                sheet[self.state.cursor_row][j].err = 1;
                                            }
                                        }
                                    }
                                }
                            }

                            self.state.status_message =
                                format!("Pasted row to row {}", self.state.cursor_row + 1);
                        }
                        ClipboardContent::Column { col: _, data } => {
                            // Paste column data
                            let rows = self.rows as usize;

                            for (i, value) in data.iter().enumerate() {
                                if i < rows {
                                    match parse_formula(value) {
                                        Ok(formula) => {
                                            sheet[i][self.state.cursor_col].formula = Some(formula);
                                            sheet[i][self.state.cursor_col].err = 0;
                                        }
                                        Err(_) => {
                                            if let Ok(val) = value.parse::<i32>() {
                                                sheet[i][self.state.cursor_col].formula = None;
                                                sheet[i][self.state.cursor_col].val = val;
                                                sheet[i][self.state.cursor_col].err = 0;
                                            } else {
                                                sheet[i][self.state.cursor_col].formula = None;
                                                sheet[i][self.state.cursor_col].val = 0;
                                                sheet[i][self.state.cursor_col].err = 1;
                                            }
                                        }
                                    }
                                }
                            }

                            self.state.status_message = format!(
                                "Pasted column to column {}",
                                column_name(self.state.cursor_col)
                            );
                        }
                    }
                }
            } else if ctx.input(|i| i.key_pressed(egui::Key::D) && !i.modifiers.ctrl) {
                // Delete current cell - only if not part of Ctrl+D combination
                sheet[self.state.cursor_row][self.state.cursor_col].formula = None;
                sheet[self.state.cursor_row][self.state.cursor_col].val = 0;
                sheet[self.state.cursor_row][self.state.cursor_col].err = 0;
                self.state.status_message = format!(
                    "Deleted cell {}{}",
                    column_name(self.state.cursor_col),
                    self.state.cursor_row + 1
                );
            } else if ctx.input(|i| i.key_pressed(egui::Key::Z)) {
                // Go to last row
                self.state.cursor_row = (self.rows - 1) as usize;
                if self.state.cursor_row >= self.state.row_offset + self.visible_rows {
                    self.state.row_offset =
                        self.state.cursor_row.saturating_sub(self.visible_rows / 2);
                }
            } else if ctx.input(|i| i.key_pressed(egui::Key::Num0)) {
                // Go to first column
                self.state.cursor_col = 0;
                self.state.col_offset = 0;
            } else if ctx.input(|i| i.key_pressed(egui::Key::Num4) && i.modifiers.shift) {
                // Go to last column ($ key)
                self.state.cursor_col = (self.cols - 1) as usize;
                if self.state.cursor_col >= self.state.col_offset + self.visible_cols {
                    self.state.col_offset =
                        self.state.cursor_col.saturating_sub(self.visible_cols / 2);
                }
            } else if ctx.input(|i| i.key_pressed(egui::Key::G)) {
                // Go to first row
                self.state.cursor_row = 0;
                self.state.row_offset = 0;
            } else if ctx.input(|i| i.key_pressed(egui::Key::F) && i.modifiers.ctrl) {
                let visible_rows = self.visible_rows;
                let new_offset =
                    (self.state.row_offset + visible_rows).min(self.rows as usize - visible_rows);

                self.state.row_offset = new_offset;
                self.state.cursor_row = self.state.row_offset;

                // Update status message
                self.state.status_message = String::from("Page down");
            }
            // Page up (Ctrl+B in Vim)
            else if ctx.input(|i| i.key_pressed(egui::Key::B) && i.modifiers.ctrl) {
                let visible_rows = self.visible_rows;
                self.state.row_offset = self.state.row_offset.saturating_sub(visible_rows);
                self.state.cursor_row = self.state.row_offset;

                // Update status message
                self.state.status_message = String::from("Page up");
            } else if ctx.input(|i| i.key_pressed(egui::Key::L) && i.modifiers.ctrl) {
                let visible_cols = self.visible_cols;
                let new_offset =
                    (self.state.col_offset + visible_cols).min(self.cols as usize - visible_cols);

                self.state.col_offset = new_offset;
                self.state.cursor_col = self.state.col_offset;

                // Update status message
                self.state.status_message = String::from("Page right");
            }

            // Page left (Ctrl+H)
        }

        // Insert mode key handling
        if let Mode::Insert = self.state.mode {
            // The text editing is handled by the egui::TextEdit widget
            self.state.edit_buffer = self.formula_editor.clone();

            if ctx.input(|i| i.key_pressed(egui::Key::Enter)) {
                // Apply the edit, move to the next row, and stay in insert mode
                self.state.apply_edit(&mut sheet);
                if self.state.cursor_row + 1 < self.rows as usize {
                    self.state.cursor_row += 1;
                    if self.state.cursor_row >= self.state.row_offset + self.visible_rows {
                        self.state.row_offset += 1;
                    }
                    self.state.enter_insert_mode(&sheet);
                    self.formula_editor = self.state.edit_buffer.clone();
                } else {
                    self.state.enter_normal_mode();
                }
            } else if ctx.input(|i| i.key_pressed(egui::Key::Tab)) {
                // Apply the edit, move to the next column, and stay in insert mode
                self.state.apply_edit(&mut sheet);
                if self.state.cursor_col + 1 < self.cols as usize {
                    self.state.cursor_col += 1;
                    if self.state.cursor_col >= self.state.col_offset + self.visible_cols {
                        self.state.col_offset += 1;
                    }
                    self.state.enter_insert_mode(&sheet);
                    self.formula_editor = self.state.edit_buffer.clone();
                } else {
                    self.state.enter_normal_mode();
                }
            }
        }

        // Command mode key handling
        if let Mode::Command = self.state.mode {
            self.state.command_buffer = self.command_editor.clone();

            if ctx.input(|i| i.key_pressed(egui::Key::Enter)) {
                let command = self.state.command_buffer.clone();
                self.state.enter_normal_mode();

                match execute_command(&command, &mut self.state, &mut sheet, self.rows, self.cols) {
                    Ok(()) => {}
                    Err(message) => {
                        if message == "quit" {
                            self.should_exit = true;
                        }
                        self.state.status_message = message;
                    }
                }
            }
        }

        // Visual mode key handling
        if let Mode::Visual { .. } = self.state.mode {
            if ctx.input(|i| i.key_pressed(egui::Key::H) && i.modifiers.ctrl) {
                let visible_cols = self.visible_cols;
                self.state.col_offset = self.state.col_offset.saturating_sub(visible_cols);
                self.state.cursor_col = self.state.col_offset;

                // Update status message
                self.state.status_message = String::from("Page left");
            } else if ctx.input(|i| i.key_pressed(egui::Key::H)) {
                if self.state.cursor_col > 0 {
                    self.state.cursor_col -= 1;
                    if self.state.cursor_col < self.state.col_offset {
                        self.state.col_offset = self.state.cursor_col;
                    }
                }
            } else if ctx.input(|i| i.key_pressed(egui::Key::J)) {
                if self.state.cursor_row + 1 < self.rows as usize {
                    self.state.cursor_row += 1;
                    if self.state.cursor_row >= self.state.row_offset + self.visible_rows {
                        self.state.row_offset += 1;
                    }
                }
            } else if ctx.input(|i| i.key_pressed(egui::Key::K)) {
                if self.state.cursor_row > 0 {
                    self.state.cursor_row -= 1;
                    if self.state.cursor_row < self.state.row_offset {
                        self.state.row_offset = self.state.cursor_row;
                    }
                }
            } else if ctx.input(|i| i.key_pressed(egui::Key::L) && i.modifiers.ctrl) {
                let visible_cols = self.visible_cols;
                let new_offset =
                    (self.state.col_offset + visible_cols).min(self.cols as usize - visible_cols);

                self.state.col_offset = new_offset;
                self.state.cursor_col = self.state.col_offset;

                // Update status message
                self.state.status_message = String::from("Page right");
            } else if ctx.input(|i| i.key_pressed(egui::Key::L)) {
                if self.state.cursor_col + 1 < self.cols as usize {
                    self.state.cursor_col += 1;
                    if self.state.cursor_col >= self.state.col_offset + self.visible_cols {
                        self.state.col_offset += 1;
                    }
                }
            } else if ctx.input(|i| i.key_pressed(egui::Key::Y)) {
                // Yank (copy) selected range
                if let Some((min_row, min_col, max_row, max_col)) =
                    self.state.get_visual_selection()
                {
                    let height = max_row - min_row + 1;
                    let width = max_col - min_col + 1;

                    let mut data = Vec::with_capacity(height);
                    for i in 0..height {
                        let mut row_data = Vec::with_capacity(width);
                        for j in 0..width {
                            let formula_str = if let Some(ref formula) =
                                sheet[min_row + i][min_col + j].formula
                            {
                                formula_to_string(formula)
                            } else {
                                sheet[min_row + i][min_col + j].val.to_string()
                            };
                            row_data.push(formula_str);
                        }
                        data.push(row_data);
                    }

                    self.state.clipboard = Some(ClipboardContent::Range {
                        start_row: min_row,
                        start_col: min_col,
                        end_row: max_row,
                        end_col: max_col,
                        data,
                    });

                    self.state.status_message = format!(
                        "Yanked range {}{}:{}{}",
                        column_name(min_col),
                        min_row + 1,
                        column_name(max_col),
                        max_row + 1
                    );

                    self.state.enter_normal_mode();
                }
            } else if ctx.input(|i| i.key_pressed(egui::Key::D)) {
                // Delete selected range
                if let Some((min_row, min_col, max_row, max_col)) =
                    self.state.get_visual_selection()
                {
                    for i in min_row..=max_row {
                        for j in min_col..=max_col {
                            sheet[i][j].formula = None;
                            sheet[i][j].val = 0;
                            sheet[i][j].err = 0;
                        }
                    }

                    self.state.status_message = format!(
                        "Deleted range {}{}:{}{}",
                        column_name(min_col),
                        min_row + 1,
                        column_name(max_col),
                        max_row + 1
                    );
                    self.state.enter_normal_mode();
                }
            }
        }

        // Evaluate cells after changes
        let total_cells = (self.rows * self.cols) as usize;
        let mut evaluated = vec![false; total_cells];

        // Create a temporary graph for dependency tracking
        let mut graph = Vec::with_capacity(total_cells);
        for _ in 0..total_cells {
            graph.push(Some(Box::new(crate::DAGNode {
                in_degree: 0,
                dependents: HashSet::new(),
                dependencies: HashSet::new(),
            })));
        }

        // Evaluate cells that need updating
        for r in 0..self.rows {
            for c in 0..self.cols {
                if sheet[r as usize][c as usize].formula.is_some() {
                    evaluate_cell(
                        r,
                        c,
                        &mut sheet,
                        &graph,
                        &mut evaluated,
                        self.rows,
                        self.cols,
                    );
                }
            }
        }
    }

    fn render_spreadsheet(&mut self, ui: &mut Ui) {
        let sheet = self.sheet.lock().unwrap();

        egui::ScrollArea::both().show(ui, |ui| {
            // Create a grid for the spreadsheet
            egui::Grid::new("spreadsheet_grid")
                .spacing([1.0, 1.0])
                .show(ui, |ui| {
                    // Empty cell in top-left corner
                    ui.add_sized([ROW_HEADER_WIDTH, HEADER_SIZE], egui::Label::new(""));

                    // Column headers
                    for col in self.state.col_offset
                        ..(self.state.col_offset
                            + self
                                .visible_cols
                                .min(self.cols as usize - self.state.col_offset))
                    {
                        let col_name = column_name(col);
                        ui.add_sized(
                            [CELL_SIZE, HEADER_SIZE],
                            egui::Label::new(
                                RichText::new(col_name)
                                    .strong()
                                    .color(Color32::WHITE)
                                    .background_color(Color32::DARK_BLUE),
                            ),
                        );
                    }
                    ui.end_row();

                    // Rows
                    for row in self.state.row_offset
                        ..(self.state.row_offset
                            + self
                                .visible_rows
                                .min(self.rows as usize - self.state.row_offset))
                    {
                        // Row header
                        ui.add_sized(
                            [ROW_HEADER_WIDTH, CELL_SIZE],
                            egui::Label::new(
                                RichText::new(format!("{}", row + 1))
                                    .strong()
                                    .color(Color32::WHITE)
                                    .background_color(Color32::DARK_BLUE),
                            ),
                        );

                        // Cells in this row
                        for col in self.state.col_offset
                            ..(self.state.col_offset
                                + self
                                    .visible_cols
                                    .min(self.cols as usize - self.state.col_offset))
                        {
                            let is_cursor =
                                row == self.state.cursor_row && col == self.state.cursor_col;
                            let is_selected = match self.state.get_visual_selection() {
                                Some((min_row, min_col, max_row, max_col)) => {
                                    row >= min_row
                                        && row <= max_row
                                        && col >= min_col
                                        && col <= max_col
                                }
                                None => false,
                            };

                            // Check if this cell is a search match
                            let is_search_match =
                                if let Some(ref pattern) = self.state.search_pattern {
                                    let cell_value =
                                        if let Some(ref formula) = sheet[row][col].formula {
                                            formula_to_string(formula)
                                        } else {
                                            sheet[row][col].val.to_string()
                                        };

                                    cell_value.contains(pattern)
                                } else {
                                    false
                                };

                            // Check if this is the current search match
                            let is_current_match = if let Some(idx) = self.state.current_match {
                                if idx < self.state.search_matches.len() {
                                    let (match_row, match_col) = self.state.search_matches[idx];
                                    row == match_row && col == match_col
                                } else {
                                    false
                                }
                            } else {
                                false
                            };

                            let cell_value = if sheet[row][col].err != 0 {
                                "ERR".to_string()
                            } else {
                                sheet[row][col].val.to_string()
                            };

                            let mut text = RichText::new(cell_value);

                            if is_cursor {
                                text = text.background_color(Color32::LIGHT_BLUE);
                            } else if is_current_match {
                                text = text.background_color(Color32::GOLD);
                            } else if is_search_match {
                                text = text.background_color(Color32::LIGHT_YELLOW);
                            } else if is_selected {
                                text = text.background_color(Color32::LIGHT_GREEN);
                            }

                            let response =
                                ui.add_sized([CELL_SIZE, CELL_SIZE], egui::Label::new(text));

                            // Handle cell click
                            if response.clicked() {
                                self.state.cursor_row = row;
                                self.state.cursor_col = col;
                            } else if response.double_clicked() {
                                self.state.cursor_row = row;
                                self.state.cursor_col = col;
                                self.state.enter_insert_mode(&sheet);
                                self.formula_editor = self.state.edit_buffer.clone();
                            }
                        }
                        ui.end_row();
                    }
                });
        });
    }

    fn render_status_bar(&self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            let current_cell = format!(
                "{}{}",
                column_name(self.state.cursor_col),
                self.state.cursor_row + 1
            );

            // Add a prominent mode indicator with distinct colors
            match self.state.mode {
                Mode::Normal => {
                    ui.add(egui::Label::new(
                        RichText::new(" NORMAL ")
                            .strong()
                            .background_color(Color32::DARK_BLUE)
                            .color(Color32::WHITE),
                    ));
                }
                Mode::Insert => {
                    ui.add(egui::Label::new(
                        RichText::new(" INSERT ")
                            .strong()
                            .background_color(Color32::DARK_GREEN)
                            .color(Color32::WHITE),
                    ));
                }
                Mode::Command => {
                    ui.add(egui::Label::new(
                        RichText::new(" COMMAND ")
                            .strong()
                            .background_color(Color32::DARK_RED)
                            .color(Color32::WHITE),
                    ));
                }
                Mode::Visual { .. } => {
                    ui.add(egui::Label::new(
                        RichText::new(" VISUAL ")
                            .strong()
                            .background_color(Color32::DARK_GREEN)
                            .color(Color32::WHITE),
                    ));
                }
            }

            ui.add_space(10.0); // Add some spacing
            ui.label(format!("Cell: {}", current_cell));
            ui.add_space(10.0); // Add some spacing

            // Show status message
            ui.label(self.state.status_message.clone());

            // Show formula of current cell
            let sheet = self.sheet.lock().unwrap();
            let formula = match self.state.mode {
                Mode::Insert => self.formula_editor.clone(),
                _ => {
                    if let Some(ref formula) =
                        sheet[self.state.cursor_row][self.state.cursor_col].formula
                    {
                        formula_to_string(formula)
                    } else {
                        "".to_string()
                    }
                }
            };

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(format!("Formula: {}", formula));
            });
        });
    }

    fn render_formula_editor(&mut self, ui: &mut Ui) {
        if let Mode::Insert = self.state.mode {
            ui.horizontal(|ui| {
                ui.label(format!(
                    "Edit {}{}:",
                    column_name(self.state.cursor_col),
                    self.state.cursor_row + 1
                ));

                let response = ui.add(
                    egui::TextEdit::singleline(&mut self.formula_editor)
                        .desired_width(f32::INFINITY)
                        .hint_text("Enter formula or value..."),
                );

                if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    let mut sheet = self.sheet.lock().unwrap();
                    self.state.edit_buffer = self.formula_editor.clone();
                    self.state.apply_edit(&mut sheet);
                    self.state.enter_normal_mode();
                }
            });
        }
    }

    fn render_command_editor(&mut self, ui: &mut Ui) {
        if let Mode::Command = self.state.mode {
            ui.horizontal(|ui| {
                let response = ui.add(
                    egui::TextEdit::singleline(&mut self.command_editor)
                        .desired_width(f32::INFINITY)
                        .hint_text("Enter command..."),
                );

                if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    let mut sheet = self.sheet.lock().unwrap();

                    // Clone the command buffer first
                    let command = self.command_editor.clone();
                    self.state.command_buffer = command.clone();
                    self.state.enter_normal_mode();

                    // Now use the cloned command
                    match execute_command(
                        &command,
                        &mut self.state,
                        &mut sheet,
                        self.rows,
                        self.cols,
                    ) {
                        Ok(()) => {}
                        Err(message) => {
                            if message == "quit" {
                                self.should_exit = true;
                            }
                            self.state.status_message = message;
                        }
                    }
                }
            });
        }
    }

    fn delete_row(&mut self, row_index: usize, count: usize) {
        let mut sheet = self.sheet.lock().unwrap();
        let rows = self.rows as usize;
        let cols = self.cols as usize;

        // Store the deleted rows for potential formula adjustments
        let mut deleted_rows = Vec::new();
        for i in 0..count {
            if row_index + i < rows {
                deleted_rows.push(sheet[row_index + i].clone());
            }
        }

        // Shift rows up
        for i in row_index..rows - count {
            if i + count < rows {
                sheet[i] = sheet[i + count].clone();
            }
        }

        // Clear the bottom rows
        for i in rows - count..rows {
            for j in 0..cols {
                sheet[i][j].val = 0;
                sheet[i][j].formula = None;
                sheet[i][j].err = 0;
            }
        }

        // Adjust formulas in the remaining cells
        // Adjust formulas in the remaining cells
        self.adjust_formulas_after_row_deletion(&mut sheet, row_index, count);
    }

    fn copy_row(&mut self, row_index: usize, count: usize) {
        let sheet = self.sheet.lock().unwrap();
        let rows = self.rows as usize;
        let cols = self.cols as usize;

        if row_index >= rows {
            return;
        }

        // Create a vector to store the row data
        let mut data = Vec::new();

        // Copy the specified rows
        for i in 0..count {
            if row_index + i < rows {
                let mut row_data = Vec::new();
                for j in 0..cols {
                    let formula_str = if let Some(ref formula) = sheet[row_index + i][j].formula {
                        formula_to_string(formula)
                    } else {
                        sheet[row_index + i][j].val.to_string()
                    };
                    row_data.push(formula_str);
                }
                data.push(row_data);
            }
        }

        // Store in clipboard
        self.state.clipboard = Some(ClipboardContent::Row {
            row: row_index,
            data: data.into_iter().flatten().collect(),
        });
    }

    // Column operations
    fn delete_column(&mut self, col_index: usize, count: usize) {
        let mut sheet = self.sheet.lock().unwrap();
        let rows = self.rows as usize;
        let cols = self.cols as usize;

        // Shift columns left
        for i in 0..rows {
            for j in col_index..cols - count {
                if j + count < cols {
                    sheet[i][j] = sheet[i][j + count].clone();
                }
            }

            // Clear the rightmost columns
            for j in cols - count..cols {
                sheet[i][j].val = 0;
                sheet[i][j].formula = None;
                sheet[i][j].err = 0;
            }
        }

        // Adjust formulas in the remaining cells
        self.adjust_formulas_after_column_deletion(&mut sheet, col_index, count);
    }

    fn copy_column(&mut self, col_index: usize, count: usize) {
        let sheet = self.sheet.lock().unwrap();
        let rows = self.rows as usize;
        let cols = self.cols as usize;

        if col_index >= cols {
            return;
        }

        // Create a vector to store the column data
        let mut data = Vec::new();

        // Copy the specified columns
        for j in 0..count {
            if col_index + j < cols {
                let mut col_data = Vec::new();
                for i in 0..rows {
                    let formula_str = if let Some(ref formula) = sheet[i][col_index + j].formula {
                        formula_to_string(formula)
                    } else {
                        sheet[i][col_index + j].val.to_string()
                    };
                    col_data.push(formula_str);
                }
                data.push(col_data);
            }
        }

        // Store in clipboard
        self.state.clipboard = Some(ClipboardContent::Column {
            col: col_index,
            data: data.into_iter().flatten().collect(),
        });
    }

    // Formula adjustment methods
    fn adjust_formulas_after_row_deletion(
        &self,
        sheet: &mut Vec<Vec<cell>>,
        row_index: usize,
        count: usize,
    ) {
        let rows = self.rows as usize;
        let cols = self.cols as usize;

        // Iterate through all cells
        for i in 0..rows {
            for j in 0..cols {
                if let Some(ref formula) = sheet[i][j].formula.clone() {
                    // Adjust the formula based on row deletion
                    // This is a complex operation that would need to traverse the AST
                    // and adjust any cell references that point to rows that have moved

                    // For now, we'll just leave the formula as is
                    // In a real implementation, you would need to update cell references
                }
            }
        }
    }

    fn adjust_formulas_after_column_deletion(
        &self,
        sheet: &mut Vec<Vec<cell>>,
        col_index: usize,
        count: usize,
    ) {
        let rows = self.rows as usize;
        let cols = self.cols as usize;

        // Iterate through all cells
        for i in 0..rows {
            for j in 0..cols {
                if let Some(ref formula) = sheet[i][j].formula.clone() {
                    // Adjust the formula based on column deletion
                    // This is a complex operation that would need to traverse the AST
                    // and adjust any cell references that point to columns that have moved

                    // For now, we'll just leave the formula as is
                    // In a real implementation, you would need to update cell references
                }
            }
        }
    }

    fn adjust_cell_references_for_row_deletion(
        &self,
        formula: &Formula,
        row_index: usize,
        count: usize,
    ) -> Formula {
        // This would need to traverse the Formula AST and adjust any cell references
        // For now, we'll just return the original formula
        formula.clone()
    }

    fn adjust_cell_references_for_column_deletion(
        &self,
        formula: &Formula,
        col_index: usize,
        count: usize,
    ) -> Formula {
        // This would need to traverse the Formula AST and adjust any cell references
        // For now, we'll just return the original formula
        formula.clone()
    }

    fn find_next_match(&mut self, forward: bool) -> bool {
        if self.state.search_matches.is_empty() {
            return false;
        }

        let (current_row, current_col) = (self.state.cursor_row, self.state.cursor_col);

        if forward {
            // Find the next match after current position
            let next_match = self.state.search_matches.iter().position(|&(row, col)| {
                (row > current_row) || (row == current_row && col > current_col)
            });

            if let Some(idx) = next_match {
                self.state.current_match = Some(idx);
            } else if !self.state.search_matches.is_empty() {
                // Wrap around to the first match
                self.state.current_match = Some(0);
            }
        } else {
            // Find the previous match before current position
            let prev_matches: Vec<_> = self
                .state
                .search_matches
                .iter()
                .enumerate()
                .filter(|&(_, &(row, col))| {
                    (row < current_row) || (row == current_row && col < current_col)
                })
                .collect();

            if !prev_matches.is_empty() {
                // Get the last match before current position
                self.state.current_match = Some(prev_matches.last().unwrap().0);
            } else if !self.state.search_matches.is_empty() {
                // Wrap around to the last match
                self.state.current_match = Some(self.state.search_matches.len() - 1);
            }
        }

        if let Some(idx) = self.state.current_match {
            let (row, col) = self.state.search_matches[idx];
            self.state.cursor_row = row;
            self.state.cursor_col = col;

            // Ensure the cursor is visible
            if self.state.cursor_row < self.state.row_offset {
                self.state.row_offset = self.state.cursor_row;
            } else if self.state.cursor_row >= self.state.row_offset + self.visible_rows {
                self.state.row_offset = self.state.cursor_row.saturating_sub(self.visible_rows / 2);
            }

            if self.state.cursor_col < self.state.col_offset {
                self.state.col_offset = self.state.cursor_col;
            } else if self.state.cursor_col >= self.state.col_offset + self.visible_cols {
                self.state.col_offset = self.state.cursor_col.saturating_sub(self.visible_cols / 2);
            }

            return true;
        }

        false
    }
}

// Helper function to convert column name to index
fn column_name_to_index(name: &str) -> usize {
    let mut index = 0;
    for c in name.chars() {
        index = index * 26 + (c as usize - 'A' as usize + 1);
    }
    index - 1 // Convert to 0-indexed
}

impl App for SpreadsheetApp {
    fn update(&mut self, ctx: &egui::Context, frame: &mut Frame) {
        // Check if we should exit
        if self.should_exit {
            std::process::exit(0);
            return;
        }

        // Handle keyboard input
        self.handle_keyboard_input(ctx);

        // Main UI layout
        egui::CentralPanel::default().show(ctx, |ui| {
            // Add the title bar with mode indicator
            self.render_title_bar(ui);

            // Top area for formula editing
            self.render_formula_editor(ui);

            // Command area
            self.render_command_editor(ui);

            // Main spreadsheet area
            self.render_spreadsheet(ui);

            // Status bar at the bottom
            ui.separator();
            self.render_status_bar(ui);
        });

        // Request continuous repainting for smooth interaction
        ctx.request_repaint();
    }
}
