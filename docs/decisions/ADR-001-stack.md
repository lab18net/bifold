# ADR-001: Technology stack

- Status: accepted
- Date: 2026-11-18
- Deciders: Andrey Abramov (solo)

## Context

Bifold is a two-panel file manager with two front ends — a full-screen terminal
UI and a native desktop window — built by one developer as a Lab18 product.

What the problem actually demands:

- **Correct, fast filesystem work**: recursive copy/move with progress and
  cancel, permission and mtime preservation, symlink-aware traversal, trash
  instead of unlink, cross-device fallback.
- **Two renderers over one behaviour model.** The commander workflow (cursor,
  mark, passive pane as destination) is the product; if the terminal and the
  desktop behave differently, one of them is wrong.
- **Comfortable latency at 10k+ entries per directory**, with a UI that stays
  responsive while a transfer runs.
- **Distribution without a runtime**: a single executable for the TUI, a
  signed `.app` for macOS, and later Linux/Windows builds, without shipping a
  browser.
- **One person, part-time.** Boring technology and one toolchain beat a
  polyglot setup that needs two ecosystems updated in lockstep.

## Decision

**Rust**, one Cargo workspace, three crates:

| Crate | Role | Key dependencies |
|---|---|---|
| `bifold-core` | model, pane/session state, fs operations, job engine | `chrono`, `serde`, `toml` |
| `bifold-tui` | terminal UI, binary `bifold` | `ratatui`, `crossterm` |
| `bifold-gui` | desktop UI, binary `bifold-gui` | `eframe` (egui) |

Rationale:

- The core is pure `std::fs` plus threads — no async runtime, no GC pauses, no
  event-loop plumbing to keep two front ends in sync.
- `ratatui` is the mature TUI toolkit in Rust and gives exact cell-level control
  (columns, marks, dialogs) with a `TestBackend` that makes layout testable
  without a terminal.
- `egui`/`eframe` is an immediate-mode GUI: the desktop front end is one
  `update()` function repainting the same `Session`, with no retained widget
  tree to keep consistent. It bundles to a native binary via `cargo bundle`.
- One toolchain (`cargo`) builds, tests, lints and ships both binaries, so CI is
  three commands.

## Alternatives considered

| Option | Why not |
|---|---|
| TypeScript: Ink/React TUI + Tauri GUI | Two rendering models, a Node toolchain plus a Rust shim, and a webview in the desktop app. Rewriting core logic once per front end is the exact failure this ADR avoids. |
| Go: bubbletea + Fyne/Wails | No Go on the dev machine, weaker native-GUI story, and the same "two idioms" problem. |
| Python: textual + Qt/Tk | Distribution pain (interpreter + wheels), slow large-directory scans, and a packaging story that does not survive notarisation cleanly. |
| Electron/Tauri only | A file manager that needs a browser engine to copy a folder is the wrong shape. |
| C++17/Qt | Best-in-class file managers are written this way, but the build/CI/onboarding cost for a solo product is not justified. |

## Consequences

- Compile times are the main tax (egui is a large dependency tree; first build
  is minutes, incremental is seconds).
- `bifold-core` must stay free of UI crates — enforced by review and by the
  absence of those dependencies in `crates/core/Cargo.toml`.
- Platform-specific behaviour (trash location, `.app` packaging, notarisation)
  is quarantined behind `fsops` and packaging scripts, not spread through the
  UI code.
- Two front ends means two places to wire a new feature; ADR-002 makes that a
  parity rule rather than a suggestion.
