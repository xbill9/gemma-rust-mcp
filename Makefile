# Makefile for gemma-rust-mcp

# Variables
BIN := gemma-rust-mcp
PROMPT ?= In one sentence, what is a TPU?
LOCAL_RIG := $(HOME)/gemma4-dev/local-llamacpp-1650ti-2b-q4_0
CLOUD_RIG := $(HOME)/gemma4-dev/gpu-2B-cloudrun-devops-agent

.PHONY: all build debug prod release run run-debug run-cloud chat chat-cloud status status-cloud rig-help rig-help-cloud clean lint clippy fmt format fmt-check check test ci help

# The default target
all: debug

# Build the project for development
debug:
	@echo "Building debug..."
	@cargo build

build: debug

# Build the project for release
prod:
	@echo "Building release..."
	@cargo build --release
	@echo "Binary: target/release/$(BIN)"

release: prod

# Ask through the local llama.cpp rig's MCP server (release build)
run:
	@echo "Asking through the local rig's MCP server..."
	@cargo run --release -- --rig local "$(PROMPT)"

# Ask through the local llama.cpp rig's MCP server (debug build)
run-debug:
	@echo "Asking through the local rig's MCP server (debug build)..."
	@cargo run -- --rig local "$(PROMPT)"

# Ask through the Cloud Run rig's MCP server
run-cloud:
	@echo "Asking through the Cloud Run rig's MCP server (first request may take minutes on a cold start)..."
	@cargo run --release -- --rig cloudrun "$(PROMPT)"

# Interactive session through the local rig's MCP server
chat:
	@cargo run --release -- --rig local --interactive

# Interactive session through the Cloud Run rig's MCP server
chat-cloud:
	@cargo run --release -- --rig cloudrun --interactive

# The local rig's status tools: GPU and memory, model server, model info
status:
	@cargo run -q --release -- --rig local --status

# The Cloud Run rig's status tools: deployment, system status, model details
status-cloud:
	@cargo run -q --release -- --rig cloudrun --status

# The local rig's own help tool: the tools it exposes
rig-help:
	@cargo run -q --release -- --rig local --rig-help

# The Cloud Run rig's own help tool: its configuration and tools
rig-help-cloud:
	@cargo run -q --release -- --rig cloudrun --rig-help

# Clean the project
clean:
	@echo "Cleaning the project..."
	@cargo clean

# Lint the code: clippy on every target, then the format check
lint:
	@echo "Linting code..."
	@cargo clippy --all-targets -- -D warnings
	@cargo fmt --all -- --check

clippy:
	@echo "Running clippy..."
	@cargo clippy --all-targets -- -D warnings

# Format the code
fmt:
	@echo "Formatting code..."
	@cargo fmt --all

format: fmt

# Check formatting without changing files
fmt-check:
	@echo "Checking formatting..."
	@cargo fmt --all -- --check

# Check the code
check:
	@echo "Checking the code..."
	@cargo check --all-targets

# Run tests
test:
	@echo "Running tests..."
	@cargo test

# Everything a CI run would do
ci: lint test prod

help:
	@echo "Makefile for gemma-rust-mcp"
	@echo ""
	@echo "Usage:"
	@echo "    make <target> [PROMPT=\"...\"]"
	@echo ""
	@echo "Targets:"
	@echo "    all          (default) same as 'debug'"
	@echo "    debug        Build for development (alias: build)"
	@echo "    prod         Build optimised release binary (alias: release)"
	@echo "    run          Ask through the local rig's MCP server (release build)"
	@echo "    run-debug    Ask through the local rig's MCP server (debug build)"
	@echo "    run-cloud    Ask through the Cloud Run rig's MCP server"
	@echo "    chat         Interactive session through the local rig's MCP server"
	@echo "    chat-cloud   Interactive session through the Cloud Run rig's MCP server"
	@echo "    status       Local rig's status tools: GPU memory, model server, model info"
	@echo "    status-cloud Cloud Run rig's status tools: deployment, system status, model"
	@echo "    rig-help     Local rig's own help tool: the tools it exposes"
	@echo "    rig-help-cloud Cloud Run rig's own help tool: configuration and tools"
	@echo "    clean        Remove build artefacts"
	@echo "    lint         clippy -D warnings + format check"
	@echo "    clippy       clippy only"
	@echo "    fmt          Format the code (alias: format)"
	@echo "    fmt-check    Check formatting without changing files"
	@echo "    check        cargo check"
	@echo "    test         Run tests"
	@echo "    ci           lint + test + prod"
	@echo ""
	@echo "The CLI launches each rig's MCP server itself; model servers come from the rigs:"
	@echo "    make -C $(LOCAL_RIG) serve"
	@echo "    make -C $(CLOUD_RIG) status"
