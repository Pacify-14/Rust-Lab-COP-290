# Rust-Lab-COP-290
# Vim Spreadsheet Editor

A powerful spreadsheet editor with Vim-like keybindings and commands, offering a familiar interface for Vim users while providing robust spreadsheet functionality.

## Features

### Navigation

- **Vim-style Movement**: Navigate using `h`, `j`, `k`, `l` keys
- **Jump Commands**: 
  - `g` - Go to first row
  - `G` - Go to last row
  - `0` - Go to first column
  - `$` - Go to last column
- **Page Navigation**:
  - `Ctrl+F` - Page down
  - `Ctrl+B` - Page up
  - `Ctrl+L` - Page right
  - `Ctrl+H` - Page left
- **Cell Jump**: Type `:A1` to jump directly to cell A1

### Editing

- **Modes**:
  - **Normal Mode**: For navigation and commands
  - **Insert Mode**: For editing cell content (enter with `i`)
  - **Command Mode**: For executing commands (enter with `:`)
  - **Visual Mode**: For selecting ranges (enter with `v`)

- **Cell Editing**:
  - `i` - Enter insert mode to edit current cell
  - `Enter` in insert mode - Apply edit and move to next row
  - `Tab` in insert mode - Apply edit and move to next column
  - `Escape` - Return to normal mode

- **Formulas**:
  - Basic arithmetic: `=A1+B1`, `=C1*D1`, etc.
  - Statistical functions: `=AVERAGE(A1:A10)`, `=STDDEV(B1:B10)`, `=PERCENTILE(C1:C10,0.75)`
  - Batch formula assignment: `:i in 1..10: Ai = Bi + 1`

### Clipboard Operations

- **Copy/Paste**:
  - `y` - Yank (copy) current cell
  - `p` - Paste to current cell
  - Visual mode + `y` - Copy selected range
  - Visual mode + `d` - Delete selected range

- **Row/Column Operations**:
  - `Ctrl+R` - Delete current row
  - `Ctrl+Y` then `R` - Yank (copy) current row
  - `Ctrl+C` - Delete current column
  - `Ctrl+S` - Yank (copy) current column

### Search and Replace

- `/pattern` - Search forward for pattern
- `?pattern` - Search backward for pattern
- `n` - Go to next match
- `N` - Go to previous match
- `:s/old/new/` - Replace first occurrence in current cell
- `:s/old/new/g` - Replace all occurrences in current cell
- `:%s/old/new/g` - Replace all occurrences in all cells

### File Operations

- `:w [filename]` - Save spreadsheet (default: spreadsheet.ss)
- `:w filename.csv` - Export as CSV
- `:w filename.tsv` - Export as TSV
- `:e [filename]` - Open a file
- `:wq` - Save and quit

### Other Commands

- `:q` or `:quit` - Quit the application
- `:help` - Display help information

## Command Reference

### Cell Navigation Commands

| Command | Description |
|---------|-------------|
| `h`, `j`, `k`, `l` | Move left, down, up, right |
| `gg` | Go to first row |
| `G` | Go to last row |
| `0` | Go to first column |
| `$` | Go to last column |
| `:A1` | Jump to cell A1 |

### Editing Commands

| Command | Description |
|---------|-------------|
| `i` | Enter insert mode |
| `Escape` | Return to normal mode |
| `v` | Enter visual mode for selection |
| `d` | Delete current cell |

### Formula Commands

| Formula | Description |
|---------|-------------|
| `=A1+B1` | Basic arithmetic |
| `=AVERAGE(A1:A10)` | Calculate average of range |
| `=STDDEV(B1:B10)` | Calculate standard deviation of range |
| `=PERCENTILE(C1:C10,0.75)` | Calculate 75th percentile of range |

### Batch Formula Commands

| Command | Description |
|---------|-------------|
| `:i in 1..10: Ai = Bi + 1` | Set formulas for cells A1 to A10 |
| `:i,j in 1..5,1..5: Ci,j = Ai,j + Bi,j` | Set formulas for a 2D range |

### Search Commands

| Command | Description |
|---------|-------------|
| `/pattern` | Search forward for pattern |
| `?pattern` | Search backward for pattern |
| `n` | Go to next match |
| `N` | Go to previous match |
| `:s/old/new/` | Replace in current cell |
| `:s/old/new/g` | Replace all in current cell |
| `:%s/old/new/g` | Replace all in all cells |

### File Commands

| Command | Description |
|---------|-------------|
| `:w [filename]` | Save spreadsheet |
| `:e [filename]` | Open a file |
| `:wq` | Save and quit |
| `:q` or `:quit` | Quit |

## Implementation Details

- Built with Rust and egui for a responsive UI
- Supports both terminal and GUI interfaces
- Implements a custom formula evaluation engine
- Provides Vim-like modal editing experience
- Handles various file formats (custom, CSV, TSV)

## Getting Started

1. Clone the repository
2. Build with `cargo build --release`
3. Run with `cargo run --release`
4. Press `:help` for command reference

## License

[MIT License](LICENSE)
