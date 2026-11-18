# Bifold — task runner. `make help` lists targets.
CARGO ?= cargo
DIRS ?= $(HOME)

.PHONY: help build release test lint fmt check run-tui run-gui dump clean

help:
	@echo "build     debug build of both front ends"
	@echo "release   optimised build"
	@echo "test      cargo test --workspace"
	@echo "lint      clippy, warnings are errors"
	@echo "fmt       rustfmt the workspace"
	@echo "check     fmt --check + lint + test"
	@echo "run-tui   build and start the terminal UI (DIRS=\$$HOME)"
	@echo "run-gui   build and start the desktop UI"
	@echo "dump      print both panes as text and exit"
	@echo "clean     cargo clean"

build:
	$(CARGO) build --workspace

release:
	$(CARGO) build --workspace --release

test:
	$(CARGO) test --workspace

lint:
	$(CARGO) clippy --workspace --all-targets -- -D warnings

fmt:
	$(CARGO) fmt --all

check:
	$(CARGO) fmt --all -- --check
	$(CARGO) clippy --workspace --all-targets -- -D warnings
	$(CARGO) test --workspace

run-tui: build
	./target/debug/bifold --path "$(DIRS)"

run-gui: build
	./target/debug/bifold-gui --path "$(DIRS)"

dump: build
	./target/debug/bifold --path "$(DIRS)" --dump

clean:
	$(CARGO) clean
