# Rust-Lab-COP-290
# Vim Spreadsheet Editor

A powerful spreadsheet editor with Vim-like keybindings and commands, offering a familiar interface for Vim users while providing robust spreadsheet functionality.

## Features

### Navigation

- **Vim-style Movement**: Navigate using `h`, `j`, `k`, `l` keys
- **Jump Commands**: 
  - `g` - Go to first row
  - `Z` - Go to last row
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
  - Statistical functions: 
    - `=AVG(A1:A10)` - Calculate average
    - `=MAX(A1:B2)` - Find maximum value
    - `=MIN(A1:B2)` - Find minimum value
    - `=STDDEV(B1:B10)` - Calculate standard deviation
    
  - Batch formula assignment: `:i in 1..10: Ai = Bi + 1`

### Clipboard Operations

- **Copy/Paste**:
  - `y` - Yank (copy) current cell
  - `p` - Paste to current cell
  - Visual mode + `y` - Copy selected range
  - Visual mode + `d` - Delete selected range

- **Row/Column Operations**:
  - `Ctrl+R` - Delete current row
  - `Ctrl+M` - Yank (copy) current row
  - `Ctrl+C` - Delete current column
  - `Ctrl+S` - Yank (copy) current column

### Search

- `/pattern` - Search forward for pattern
- `?pattern` - Search for pattern
- `n` - Go to next match


### File Operations

- `:w [filename]` - Save spreadsheet (default: spreadsheet.ss)
- `:w filename.csv` - Export as CSV
- `:w filename.tsv` - Export as TSV
- `:e [filename]` - Open a file
- `:wq` - Save and quit

### Other Commands

- `:q` or `:quit` - Quit the application


## Command Reference

### Cell Navigation Commands

| Command | Description |
|---------|-------------|
| `h`, `j`, `k`, `l` | Move left, down, up, right |
| `g` | Go to first row |
| `Z` | Go to last row |
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
| `=SUM(A1:A10)` | Calculate sum of range |
| `=AVG(A1:A10)` | Calculate average of range |
| `=MAX(A1:B2)` | Find maximum value in range |
| `=MIN(A1:B2)` | Find minimum value in range |
| `=STDDEV(B1:B10)` | Calculate standard deviation of range |


### Batch Formula Commands

| Command | Description |
|---------|-------------|
| `:i in 1..10: Ai = Bi + 1` | Set formulas for cells A1 to A10 |
| `:i in 1..10: Di = AVG(Ai:Ci)` | Computing Average and other functionalities |
| `:i in 1..10: Ei = Ai + Di` | Computing Sum across rows |

### Search Commands

| Command | Description |
|---------|-------------|
| `/pattern` | Search forward for pattern |
| `?pattern` | Search backward for pattern |
| `n` | Go to next match |


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

## Using make 

| Command | Description |
|---------|-------------|
| `make` | Clean and build normal spreadsheet binary |
| `./target/release/spreadsheet <rows> <columns>` | After building, run the binary manually |
| `make run` | Build and run normal spreadsheet with size 999x18278 |
| `make vimmode` | Build Vim-like spreadsheet binary in `vimversion` directory |
| `env -u WAYLAND_DISPLAY ./vimversion/target/release/spreadsheet --vim <rows> <cols>` or `./vimversion/target/release/spreadsheet --vim <rows> <cols>` | Run Vim-like Spreadsheet Binary |
| `make vimmode-run` | Build and run Vim-like spreadsheet automatically with default size 100x100 |
| `make clean` | Remove all build artifacts |
| `make coverage` | Testing coverage |
| `make test` | Running tests |
| `make docs` | Generating rust doc and report.pdf  |
| `make ext1` | Running extensions |

## For make docs
`sudo apt install texlive-latex-base`
`sudo apt install texlive-latex-extra`
`sudo apt install etoolbox`

## License

[MIT License](LICENSE)

