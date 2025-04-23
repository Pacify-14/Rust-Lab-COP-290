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

.PHONY: all build run clean vimmode vimmode-run

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
	@echo "Cleaned build files"

# Build the binary in the vimversion directory
vimmode:
	cd $(VIM_DIR) && $(CARGO) build --release
	@echo "Built vim version: $(VIM_BIN)"

# Run the vimversion binary
vimmode-run: vimmode
	env -u WAYLAND_DISPLAY $(VIM_BIN) --vim 100 100

