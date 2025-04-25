# Project name
BIN_NAME := spreadsheet

# Cargo paths
CARGO := cargo
BUILD_DIR := target/release
RELEASE_BIN := $(BUILD_DIR)/$(BIN_NAME)

# Vim version paths
VIM_DIR := vimversion
VIM_BUILD_DIR := $(VIM_DIR)/target/release
VIM_BIN := $(VIM_BUILD_DIR)/$(BIN_NAME)

.PHONY: all build run clean vimmode vimmode-run ext1 ext1-run docs coverage test

all: clean build

# Build the release binary
build:
	$(CARGO) build --release
	@cp $(BUILD_DIR)/main $(RELEASE_BIN) 2>/dev/null || true
	@echo "Built $(RELEASE_BIN)"

# Run the binary with default args
run: build
	$(RELEASE_BIN) 999 18278

# Remove build artifacts
clean:
	$(CARGO) clean
	cd $(VIM_DIR) && $(CARGO) clean
	@rm -f *.aux *.log *.toc *.out *.fls *.fdb_latexmk
	@rm -rf tarpaulin-report.html index.html
	@echo "Cleaned build files"

# Build the binary in the vimversion directory
vimmode: clean
	cd $(VIM_DIR) && $(CARGO) build --release
	@echo "Built vim version: $(VIM_BIN)"

# Run the vimversion binary
vimmode-run: vimmode
	env -u WAYLAND_DISPLAY $(VIM_BIN) --vim 100 100

# Extension target
ext1: clean
	cd $(VIM_DIR) && $(CARGO) build --release
	env -u WAYLAND_DISPLAY $(VIM_BIN) --vim 999 1000

ext1-run: ext1
	env -u WAYLAND_DISPLAY $(VIM_BIN) --vim 999 1000

# Compile the LaTeX report
docs:
	$(CARGO) doc --open
	cp target/doc/spreadsheet/index.html ./index.html
	pdflatex report.tex
	pdflatex report.tex
	@echo "PDF report generated: report.pdf"

# Run code coverage
coverage: clean
	$(CARGO) tarpaulin --out Html

# Run tests
test: clean
	$(CARGO) test

# Check clippy and fmt
lit:
	$(CARGO) fmt
	$(CARGO) clippy
