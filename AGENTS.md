# AGENTS.md — Bifold

Instructions for anyone (human or agent) changing this repo. Product context
lives in `README.md`; this file is about how the code is arranged and what must
stay true.

## Layout

```
Cargo.toml            workspace: crates/core, crates/tui, crates/gui
crates/core/          bifold-core — no UI dependencies, no terminal types
crates/tui/           bifold-tui  — binary `bifold`
crates/gui/           bifold-gui  — binary `bifold-gui`
docs/decisions/       ADRs (Context → Decision → Consequences)
docs/PLAN.md          milestones
docs/naming.md        product-name candidates + collision notes
scripts/              release.sh, update-formula.sh, check-release-metadata.sh,
                      changelog-section.sh, package-macos.sh, site-screens.py, rename-product.sh
man/                  bifold.1, bifold-gui.1 (every CLI flag must be documented)
site/                 promo site (GitHub Pages, deployed by .github/workflows/pages.yml);
                      screens.js is generated from the real TUI by scripts/site-screens.py
packaging/macos/      Info.plist.in + Bifold.icns for scripts/package-macos.sh (Bifold.app)
packaging/homebrew/   formula templates (*.rb.in), tap starter files, publishing guide
CHANGELOG.md          Keep a Changelog; the [Unreleased] section becomes the release notes
```

## Commands

```bash
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets      # must stay at zero warnings
cargo fmt --all
cargo run -p bifold-tui -- --path ~/src
cargo run -p bifold-gui -- --path ~/src
make release-check                           # check + man lint + release metadata
scripts/release.sh X.Y.Z --dry-run           # then without --dry-run / with --push
```

`make check` runs fmt + clippy + tests together.

## Invariants

1. **`bifold-core` is UI-agnostic.** No `ratatui`, `crossterm`, `egui` or
   `eframe` dependency in `crates/core`. If a rule needs to apply to both front
   ends (sorting, filtering, confirmation, transfer semantics), it belongs in
   core.
2. **Parity by construction.** Both front ends render `Session` and call
   `Session`/`fsops` methods. Do not implement a file operation inside a UI
   crate; add it to core and call it from both.
3. **One transfer at a time.** `JobRunner::start` refuses to run concurrently;
   UI code must handle `false`/`Err` instead of ignoring it.
4. **Deletes are reversible by default.** `DeleteMode::Trash` moves into
   `~/.Trash` (or `$BIFOLD_TRASH_DIR`); permanent deletion requires the config
   to say so.
5. **Never follow symlinks while copying, measuring or deleting.** Copy
   recreates the link; `measure`/`remove_tree` use `symlink_metadata`.
6. **Transfers must be cancellable.** Long loops call `ctx.check()?` and report
   via `ctx.add_bytes` / `ctx.add_file`.
7. **UI state lives in the UI crate.** Dialogs, viewer scroll, prompt buffers
   stay in `tui::app` / `gui::main`; core knows nothing about them.
8. **No `unwrap()` on filesystem results outside tests.**
9. **Keys are data.** Front ends translate native key events into
   `core::keys::KeySpec` and run the `Action` the `Keymap` returns — never
   match raw keys against behaviour in a UI crate. Pane-state actions live in
   `Session::apply`; the UI crate handles only dialogs, viewer, quit. A new
   action needs an entry in the `actions!` macro, a default binding and a README
   mention (a test fails if an action is unbound by default). Exactly three
   things are fixed and handled outside the keymap: `h` (help, `help::is_help_key`),
   dialog keys (y/n, Enter, Esc, Tab for completion, Up/Down for command history in
   the run line) and tab keys inside the
   help screen. Help
   content lives in `core::help` and is generated from the live keymap — add
   notes there once, never in a UI crate.
10. **The viewer is `core::preview`.** Front ends render `Preview.lines`
   (`StyledLine`s) and never read files themselves. Previewer processes must
   stay time-boxed and output-capped.
11. **Colours come from `core::theme`.** A front end converts `Theme` tokens
   to its own colour type once (`Pal` in both UI crates) and never hard-codes a
   colour in a draw path. Semantic colours produced by core (`Color::Faint`,
   `Color::Accent`) go through `Theme::resolve`. Tokens keep herdr's names; a
   `None` token means "terminal default" and needs a fallback on the desktop.
12. **Commands run through `core::runcmd`.** A front end never spawns a shell
   itself: the TUI uses `run_inherited` (after leaving the alternate screen;
   `Ctrl+O` just flips back to that terminal), the GUI uses `run_captured` on
   a worker thread and keeps the result in `Session::last_run` (`Ctrl+O` shows
   it in the viewer). Captured output is capped, a slow
   command is detached rather than killed, and `Session::run_suggestion`
   decides when Enter offers a run line.
13. **The config file belongs to the user.** `Config::save_to` (a full
   rewrite) is only for creating a missing file. Everything else goes through
   `Config::save_view_changes_to`, which edits in place with `toml_edit` and
   writes only settings in `ViewSettings` that changed since the session
   started. A new setting the app itself changes at runtime must be added to
   `ViewSettings`; a file that does not parse must never be overwritten. Tests must
never reach the real file either: `App::persist` / `GuiApp::persist` default to
`false` and only `main`/`run` turn them on, and a test that needs to save uses
`save_config_to` with a tempdir path.

## Releasing

- The version lives only in `[workspace.package]` of the root `Cargo.toml`; every
  crate inherits it. Do not edit it by hand: `scripts/release.sh X.Y.Z` bumps it,
  dates `CHANGELOG.md` and the man pages, runs `make check`, commits and tags
  (it pushes only with `--push`). Write user-visible changes under
  `## [Unreleased]` in `CHANGELOG.md` as you make them; the script refuses to
  release an empty section.
- A new or renamed command-line flag must be added to the `--help` text **and**
  to `man/bifold.1` (or `bifold-gui.1`): `scripts/check-release-metadata.sh`,
  which CI runs, fails otherwise.
- `Cargo.lock` stays committed and current (Homebrew builds with `--locked`).
- Homebrew formulae are templates in `packaging/homebrew/`; never commit a real
  URL or checksum there. See `packaging/homebrew/README.md` and ADR-006.

## Adding a feature

1. Model it in core with unit tests (tempdir fixtures; never touch real user
   directories).
2. Expose it in the TUI (`app.rs` key handling → `ui.rs` rendering) and in the
   GUI (`toolbar`/`shortcuts`/`dialog`) — both, or say explicitly why not.
3. Give it an `Action` and a default key in `crates/core/src/keys.rs`; add or update the key table in `README.md` and the `HELP` text in
   `crates/tui/src/main.rs` (a test asserts the help mentions the core keys).
4. Run `make check`.

## Tests worth knowing

- `crates/tui/src/ui.rs` renders into `TestBackend` at 12×4 … 200×60; narrow
  terminals must not panic.
- `crates/gui/src/main.rs` drives a headless `egui::Context` and asserts shapes
  are produced, so a broken panel layout fails without a window.
- `crates/core/src/keys.rs` and `preview.rs` cover key parsing/overrides and the
  previewer subprocess (arguments, ANSI, timeout, fallback) with real scripts.
- `crates/core/src/session.rs` covers copy/move/refusal paths end to end with
  the real job runner (poll until finished, no sleeps in production code).

## Renaming the product

The name appears as `Bifold` (display), `bifold` (binaries, config dir, crate
prefix). `scripts/rename-product.sh <NewName>` rewrites all three consistently;
review the diff and run `make check` afterwards. Directory names (`crates/core`)
are name-independent on purpose — only package names change. The script also
renames the man pages and the Homebrew templates; the GitHub repository, the tap
and existing tags have to be renamed by hand (and the repo slug in
`scripts/check-release-metadata.sh` follows the rename).
