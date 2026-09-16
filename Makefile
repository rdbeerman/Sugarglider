# Sugarglider Makefile
# Builds both Rust and Swift components

.PHONY: all build build-swift build-rust clean release install uninstall

# Default target
all: build

# Build everything (debug)
build: build-swift build-rust

# Build Swift UI library
build-swift:
	@echo "Building Swift UI library..."
	cd SugargliderUI && swift build

# Build Swift UI library (release)
build-swift-release:
	@echo "Building Swift UI library (release)..."
	cd SugargliderUI && swift build -c release

# Build Rust components (debug)
build-rust: build-swift
	@echo "Building Rust components..."
	cargo build

# Build everything (release)
release: build-swift-release
	@echo "Building Rust components (release)..."
	cargo build --release

# Run the window manager (debug)
run: build
	cargo run

# Run tests
test:
	cd SugargliderUI && swift test
	cargo test

# Clean all build artifacts
clean:
	cd SugargliderUI && swift package clean
	cargo clean

# Format code
fmt:
	cd SugargliderUI && swift format --in-place --recursive Sources/
	cargo +nightly fmt

# Check formatting
fmt-check:
	cd SugargliderUI && swift format --lint --recursive Sources/
	cargo +nightly fmt --check

# Install to /usr/local/bin
install: release
	@echo "Installing Sugarglider..."
	install -d /usr/local/bin
	install -m 755 target/release/sugarglider /usr/local/bin/
	install -m 755 target/release/sugarglider_server /usr/local/bin/

# Uninstall
uninstall:
	@echo "Uninstalling Sugarglider..."
	rm -f /usr/local/bin/sugarglider
	rm -f /usr/local/bin/sugarglider_server

# Create app bundle
bundle: release
	@echo "Creating app bundle..."
	cargo packager --release

# Development: watch and rebuild
dev:
	cargo watch -x build

# Show help
help:
	@echo "Sugarglider Build System"
	@echo ""
	@echo "Targets:"
	@echo "  build          - Build debug version (Swift + Rust)"
	@echo "  release        - Build release version"
	@echo "  run            - Build and run (debug)"
	@echo "  test           - Run all tests"
	@echo "  clean          - Remove build artifacts"
	@echo "  fmt            - Format all code"
	@echo "  install        - Install to /usr/local/bin"
	@echo "  uninstall      - Remove installation"
	@echo "  bundle         - Create macOS app bundle"
	@echo "  help           - Show this help"
