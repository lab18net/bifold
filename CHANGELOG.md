# Changelog

All notable changes to Bifold are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/). `scripts/release.sh` turns the
`Unreleased` section into a dated release.

## [Unreleased]

### Added

- Project site at <https://lab18net.github.io/bifold/> (`site/`, deployed with GitHub
  Pages); the formulae and the cask now use it as their homepage. Its terminal
  screenshots are captured from the real TUI by `scripts/site-screens.py`.

### Fixed

- The release-metadata check no longer fails CI on man-page style warnings from the
  runner's `mandoc`; only real errors fail it, and the lint output is printed.

## [0.1.0] - 2026-11-22

First public release.

### Fixed

- Running the test suite rewrote the developer's real `~/.config/bifold/config.toml`
  (theme, sort, directories); config saving is now off unless the binary turns it on.

### Added

- Homebrew cask `bifold-app` that installs the prebuilt `Bifold.app` from the
  GitHub release (`brew install --cask lab18net/tap/bifold-app`).
- `Bifold.app` for macOS: `scripts/package-macos.sh` (or `make app`) bundles the
  desktop UI with its icon into an ad-hoc signed app and a zip; release builds
  attach the zip to the GitHub release.
- Two-pane file manager with two front ends on one shared core: a full-screen
  terminal UI (`bifold`) and a native desktop window (`bifold-gui`).
- Independent panes, navigation with history, sorting (name / size / time /
  kind, folders first, reversible), hidden-file toggle, quick filter with `*`
  and `?` globs, multi-select.
- Copy and move between panes in a background thread with progress and cancel;
  one transfer at a time; a name clash never overwrites; symlinks are copied as
  links and never followed.
- Delete goes to the trash (`~/.Trash`, or `$BIFOLD_TRASH_DIR`) by default,
  with confirmation; permanent deletion only when the config says so.
- Viewer in the lf style: text as text, binary files as a hex dump, or an
  external `previewer` command (same arguments as lf) with a time limit and
  output cap.
- Key bindings are data: one keymap for both front ends, rebindable in
  `[keys]` / `[viewer_keys]` of `~/.config/bifold/config.toml`
  (`--dump-keys` prints the effective map).
- Colour themes: `bifold`, `terminal`, `nord`, `gruvbox`, `gruvbox-light`,
  `solarized`, `solarized-light` (herdr palettes), plus user themes in
  `~/.config/bifold/themes/` with `extends`.
- `h` opens a tabbed one-screen help generated from the live keymap.
- Enter on an executable opens an editable run line; `Ctrl+O` switches to the
  real terminal (desktop: shows the last command's output).
- Tab completion in every text input (folders, names, paths with shell
  quoting, programs on `PATH`); `Shift+Tab` goes back.
- `H` hides the key-hint line (`show_hints` in the config).
- The config file is edited in place on exit: comments, unknown keys and
  ordering are preserved, only changed settings are written, and a file that
  does not parse is never overwritten.
- Man pages `bifold(1)` and `bifold-gui(1)`; Homebrew formulae for a tap.

[Unreleased]: https://github.com/lab18net/bifold/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/lab18net/bifold/releases/tag/v0.1.0
