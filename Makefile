# Bifold — task runner. `make help` lists targets.
CARGO ?= cargo
DIRS ?= $(HOME)

.PHONY: help build release test lint fmt check app man-lint release-check run-tui run-gui dump clean

help:
	@echo "build     debug build of both front ends"
	@echo "release   optimised build"
	@echo "test      cargo test --workspace"
	@echo "lint      clippy, warnings are errors"
	@echo "fmt       rustfmt the workspace"
	@echo "check     fmt --check + lint + test"
	@echo "app        build dist/Bifold.app + zip (macOS)"
	@echo "man-lint  lint the man pages (mandoc)"
	@echo "release-check  check + man pages + release metadata (what a release needs)"
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

app: ## build dist/Bifold.app and its zip (macOS)
	scripts/package-macos.sh

man-lint:
	mandoc -Tlint -Wwarning man/*.1

release-check: check
	scripts/check-release-metadata.sh

run-tui: build
	./target/debug/bifold --path "$(DIRS)"

run-gui: build
	./target/debug/bifold-gui --path "$(DIRS)"

dump: build
	./target/debug/bifold --path "$(DIRS)" --dump

clean:
	$(CARGO) clean
