# ┌──────────────────────────────────────────────────────────────┐
# │  Compass — native local-first knowledge graph engine        │
# │  Makefile: build · test · install · release · dev · ci       │
# └──────────────────────────────────────────────────────────────┘

# ── Configuration ──────────────────────────────────────────────────

CARGO        ?= cargo
RUSTC        ?= rustc
RUSTUP       ?= rustup

BIN_NAME     := compass
PACKAGE      := compass-cli

BUILD_FLAGS  ?=
TEST_FLAGS   ?=
TARGET       ?=
DESTDIR      ?=
DIST_DIR     ?= dist

# Destination precedence:
#   1. explicit BINDIR
#   2. explicit PREFIX/bin
#   3. existing ~/.cargo/bin
#   4. ~/.local/bin
ifeq ($(origin BINDIR), undefined)
ifneq ($(strip $(PREFIX)),)
BINDIR := $(PREFIX)/bin
else ifneq ($(wildcard $(HOME)/.cargo/bin),)
BINDIR := $(HOME)/.cargo/bin
else
BINDIR := $(HOME)/.local/bin
endif
endif

VERSION      := $(shell sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
TOOLCHAIN    := $(shell sed -n 's/^channel = "\(.*\)"/\1/p' rust-toolchain.toml | head -1)
UNAME_S      := $(shell uname -s)
UNAME_M      := $(shell uname -m)

TARGET_ARG      = $(if $(strip $(TARGET)),--target $(TARGET),)
TARGET_SUBDIR   = $(if $(strip $(TARGET)),$(TARGET)/,)
BIN_SUFFIX      = $(if $(findstring windows,$(TARGET)),.exe,)
INSTALL_NAME    = $(BIN_NAME)$(BIN_SUFFIX)
DEBUG_INSTALL_NAME = $(BIN_NAME)-debug$(BIN_SUFFIX)
DEBUG_BINARY    = target/$(TARGET_SUBDIR)debug/$(BIN_NAME)$(BIN_SUFFIX)
RELEASE_BINARY  = target/$(TARGET_SUBDIR)release/$(BIN_NAME)$(BIN_SUFFIX)

ifeq ($(UNAME_S),Darwin)
ifneq ($(filter arm64 aarch64,$(UNAME_M)),)
DIST_TARGET := aarch64-apple-darwin
else ifneq ($(filter x86_64 amd64,$(UNAME_M)),)
DIST_TARGET := x86_64-apple-darwin
endif
endif

BOLD         := \033[1m
GREEN        := \033[32m
CYAN         := \033[36m
RESET        := \033[0m
CHECK        := $(GREEN)✓$(RESET)

# ── Help ───────────────────────────────────────────────────────────

.PHONY: help
help: ## Show this help
	@printf "$(BOLD)Compass $(VERSION)$(RESET) — Makefile targets\n\n"
	@grep -E '^[a-zA-Z0-9_-]+.*:.*?## .*$$' $(MAKEFILE_LIST) \
	  | sort \
	  | awk 'BEGIN {FS = ":.*?## "}; {printf "  $(CYAN)%-24s$(RESET) %s\n", $$1, $$2}'

# ── Build ──────────────────────────────────────────────────────────

.PHONY: build
build: ## Build the debug Compass binary
	@printf "$(BOLD)Building $(BIN_NAME) (debug)...$(RESET)\n"
	$(CARGO) build --locked -p $(PACKAGE) --bin $(BIN_NAME) $(TARGET_ARG) $(BUILD_FLAGS)

.PHONY: release
release: ## Build the optimized Compass binary
	@printf "$(BOLD)Building $(BIN_NAME) (release)...$(RESET)\n"
	$(CARGO) build --locked -p $(PACKAGE) --bin $(BIN_NAME) --release $(TARGET_ARG) $(BUILD_FLAGS)

.PHONY: all
all: build test ## Build and run the self-contained test suite

.PHONY: check
check: ## Fast workspace compile check
	$(CARGO) check --workspace --locked $(TARGET_ARG) $(BUILD_FLAGS)

.PHONY: check-release
check-release: ## Fast release-mode workspace compile check
	$(CARGO) check --workspace --release --locked $(TARGET_ARG) $(BUILD_FLAGS)

# ── Test ───────────────────────────────────────────────────────────

.PHONY: test
test: ## Run self-contained workspace tests
	@printf "$(BOLD)Running native workspace tests...$(RESET)\n"
	$(CARGO) test --workspace --exclude compass-parity --lib --bins --locked $(TEST_FLAGS)

.PHONY: test-all
test-all: ## Run all tests (requires the documented Python oracle setup)
	$(CARGO) test --workspace --all-targets --all-features --locked $(TEST_FLAGS)

.PHONY: test-release
test-release: ## Run self-contained tests in release mode
	$(CARGO) test --workspace --exclude compass-parity --lib --bins --release --locked $(TEST_FLAGS)

.PHONY: test-product
test-product: ## Run the Compass CLI product contract
	$(CARGO) test -p $(PACKAGE) --test compass_product --locked $(TEST_FLAGS)

.PHONY: test-release-scripts
test-release-scripts: ## Test packaging and download installer scripts
	sh scripts/test_release_scripts.sh

# ── Lint & Format ──────────────────────────────────────────────────

.PHONY: fmt
fmt: ## Format all Rust source
	$(CARGO) fmt --all

.PHONY: fmt-check
fmt-check: ## Check Rust formatting
	$(CARGO) fmt --all -- --check

.PHONY: clippy
clippy: ## Run Clippy across the complete workspace
	$(CARGO) clippy --workspace --all-targets --all-features --locked -- -D warnings

.PHONY: lint
lint: fmt-check clippy ## Run formatting and Clippy checks

# ── Benchmarks & Docs ──────────────────────────────────────────────

.PHONY: bench
bench: ## Run workspace benchmarks
	$(CARGO) bench --workspace --locked

.PHONY: bench-compassql
bench-compassql: ## Run the CompassQL benchmark script
	sh scripts/benchmark_compassql.sh

.PHONY: docs
docs: ## Generate workspace Rust documentation
	$(CARGO) doc --workspace --no-deps --locked

.PHONY: docs-open
docs-open: docs ## Generate and open Rust documentation
	@open target/doc/compass_cli/index.html 2>/dev/null \
	  || xdg-open target/doc/compass_cli/index.html 2>/dev/null \
	  || printf "Open target/doc/compass_cli/index.html manually\n"

# ── Install / Uninstall ────────────────────────────────────────────

.PHONY: install
install: release ## Install Compass (existing ~/.cargo/bin, else ~/.local/bin)
	@test -n "$(BINDIR)" || { printf "error: BINDIR must not be empty\n" >&2; exit 2; }
	@printf "$(BOLD)Installing $(BIN_NAME) $(VERSION) to $(BINDIR)...$(RESET)\n"
	install -d "$(DESTDIR)$(BINDIR)"
	install -m 0755 "$(RELEASE_BINARY)" "$(DESTDIR)$(BINDIR)/$(INSTALL_NAME)"
	@printf "  $(CHECK) $(DESTDIR)$(BINDIR)/$(INSTALL_NAME)\n"
	@case ":$$PATH:" in \
	  *":$(BINDIR):"*) ;; \
	  *) printf "  Add %s to PATH before running $(BIN_NAME).\n" "$(BINDIR)" ;; \
	esac

.PHONY: install-debug
install-debug: build ## Install the debug binary as compass-debug
	@test -n "$(BINDIR)" || { printf "error: BINDIR must not be empty\n" >&2; exit 2; }
	@printf "$(BOLD)Installing $(BIN_NAME) (debug) to $(BINDIR)...$(RESET)\n"
	install -d "$(DESTDIR)$(BINDIR)"
	install -m 0755 "$(DEBUG_BINARY)" "$(DESTDIR)$(BINDIR)/$(DEBUG_INSTALL_NAME)"
	@printf "  $(CHECK) $(DESTDIR)$(BINDIR)/$(DEBUG_INSTALL_NAME)\n"

.PHONY: uninstall
uninstall: ## Remove the installed Compass binary
	@test -n "$(BINDIR)" || { printf "error: BINDIR must not be empty\n" >&2; exit 2; }
	@printf "$(BOLD)Uninstalling $(BIN_NAME) from $(BINDIR)...$(RESET)\n"
	rm -f "$(DESTDIR)$(BINDIR)/$(INSTALL_NAME)"
	@printf "  $(CHECK) removed $(DESTDIR)$(BINDIR)/$(INSTALL_NAME)\n"

# ── Release Packaging ──────────────────────────────────────────────

.PHONY: dist
dist: ## Build and package Compass for the current macOS host
	@test -n "$(DIST_TARGET)" || { \
	  printf "error: dist supports native macOS hosts; use an explicit cross-build target\n" >&2; \
	  exit 2; \
	}
	@$(MAKE) --no-print-directory release TARGET="$(DIST_TARGET)"
	scripts/package_macos.sh "$(DIST_TARGET)" \
	  "target/$(DIST_TARGET)/release/$(BIN_NAME)" "$(DIST_DIR)"

.PHONY: dist-macos-arm
dist-macos-arm: ## Build and package the Apple Silicon macOS release
	@$(MAKE) --no-print-directory release TARGET=aarch64-apple-darwin
	scripts/package_macos.sh aarch64-apple-darwin \
	  target/aarch64-apple-darwin/release/$(BIN_NAME) "$(DIST_DIR)"

.PHONY: dist-macos-x86
dist-macos-x86: ## Build and package the Intel macOS release
	@$(MAKE) --no-print-directory release TARGET=x86_64-apple-darwin
	scripts/package_macos.sh x86_64-apple-darwin \
	  target/x86_64-apple-darwin/release/$(BIN_NAME) "$(DIST_DIR)"

.PHONY: dist-clean
dist-clean: ## Remove packaged release artifacts
	rm -rf "$(DIST_DIR)"

.PHONY: release-check
release-check: fmt-check clippy test test-product test-release-scripts release ## Verify release readiness
	@actual="$$(./$(RELEASE_BINARY) --version)"; \
	  expected="$(BIN_NAME) $(VERSION)"; \
	  test "$$actual" = "$$expected" \
	  && printf "  $(CHECK) version output matches $(VERSION)\n" \
	  || { printf "  ✗ expected '%s', got '%s'\n" "$$expected" "$$actual"; exit 1; }
	@printf "$(BOLD)$(GREEN)Compass $(VERSION) is release-ready.$(RESET)\n"

# ── Cross-Compilation ──────────────────────────────────────────────

.PHONY: build-linux-x86
build-linux-x86: ## Build release for x86_64 Linux
	@$(MAKE) --no-print-directory release TARGET=x86_64-unknown-linux-gnu

.PHONY: build-linux-arm
build-linux-arm: ## Build release for ARM64 Linux
	@$(MAKE) --no-print-directory release TARGET=aarch64-unknown-linux-gnu

.PHONY: build-macos-x86
build-macos-x86: ## Build release for Intel macOS
	@$(MAKE) --no-print-directory release TARGET=x86_64-apple-darwin

.PHONY: build-macos-arm
build-macos-arm: ## Build release for Apple Silicon macOS
	@$(MAKE) --no-print-directory release TARGET=aarch64-apple-darwin

.PHONY: build-windows-x86
build-windows-x86: ## Build release for x86_64 Windows
	@$(MAKE) --no-print-directory release TARGET=x86_64-pc-windows-msvc

.PHONY: build-windows-arm
build-windows-arm: ## Build release for ARM64 Windows
	@$(MAKE) --no-print-directory release TARGET=aarch64-pc-windows-msvc

# ── Dev Workflow ───────────────────────────────────────────────────

.PHONY: watch
watch: ## Recheck and retest on source changes (requires cargo-watch)
	$(CARGO) watch -x "check --workspace --locked" \
	  -x "test --workspace --exclude compass-parity --lib --bins --locked"

.PHONY: run
run: ## Build and run Compass (override with ARGS="...")
	$(CARGO) run --locked -p $(PACKAGE) --bin $(BIN_NAME) -- $(or $(ARGS),--help)

.PHONY: run-release
run-release: ## Build and run optimized Compass
	$(CARGO) run --locked -p $(PACKAGE) --bin $(BIN_NAME) --release -- $(or $(ARGS),--help)

# ── Dependencies ───────────────────────────────────────────────────

.PHONY: deps-update
deps-update: ## Update locked Rust dependencies
	$(CARGO) update

.PHONY: deps-outdated
deps-outdated: ## Report outdated dependencies (requires cargo-outdated)
	@$(CARGO) outdated || { \
	  printf "Install cargo-outdated with: cargo install cargo-outdated\n" >&2; \
	  exit 1; \
	}

.PHONY: deps-audit
deps-audit: ## Audit dependencies (requires cargo-audit)
	@$(CARGO) audit || { \
	  printf "Install cargo-audit with: cargo install cargo-audit\n" >&2; \
	  exit 1; \
	}

.PHONY: deps-policy
deps-policy: ## Check licenses, bans, advisories, and sources (requires cargo-deny)
	$(CARGO) deny --all-features check

.PHONY: deps-tree
deps-tree: ## Print the dependency tree
	$(CARGO) tree --locked

# ── Clean & Info ───────────────────────────────────────────────────

.PHONY: clean
clean: ## Remove Cargo build artifacts
	$(CARGO) clean

.PHONY: clean-all
clean-all: clean dist-clean ## Remove build and distribution artifacts
	@printf "  $(CHECK) build and distribution artifacts removed\n"

.PHONY: version
version: ## Print the Compass workspace version
	@printf "$(BIN_NAME) $(VERSION)\n"

.PHONY: info
info: ## Show build and installation settings
	@printf "$(BOLD)Compass $(VERSION)$(RESET)\n\n"
	@printf "  Rust toolchain:  %s\n" "$$($(RUSTC) --version 2>/dev/null || printf 'not found')"
	@printf "  Cargo:           %s\n" "$$($(CARGO) --version 2>/dev/null || printf 'not found')"
	@printf "  Pinned channel:  %s\n" "$(TOOLCHAIN)"
	@printf "  Target:          %s\n" "$(or $(TARGET),host)"
	@printf "  BINDIR:          %s\n" "$(BINDIR)"
	@printf "  OS:              %s (%s)\n" "$(UNAME_S)" "$(UNAME_M)"

.PHONY: toolchain
toolchain: ## Install the pinned Rust toolchain and components
	$(RUSTUP) toolchain install "$(TOOLCHAIN)" --component clippy --component rustfmt

# ── CI ─────────────────────────────────────────────────────────────

.PHONY: ci-fast
ci-fast: fmt-check check test test-product test-release-scripts ## Run the fast local CI subset

.PHONY: ci
ci: release-check ## Run the complete self-contained release gate

.DEFAULT_GOAL := help
