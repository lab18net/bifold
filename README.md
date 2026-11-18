# Bifold

Two panes. One keyboard. Terminal and desktop.

A two-panel file manager in the classic commander tradition — the left pane is
where you are, the right pane is where you are going — with two front ends that
share one core: a full-screen TUI and a native desktop window.

A project of **Andrey Abramov Consulting** (Lab18).

```
╭─ LEFT  ~/src/bifold ────────────╮┌─ RIGHT  ~/Downloads ───────────┐
│› docs/                   <dir> ││  release-notes.md        12K   │
│  crates/                 <dir> ││  screenshot.png         840K   │
│  Cargo.toml                312B ││  archive.zip            2.1M   │
│  README.md                 4.2K ││                                │
╰────────────────────────────────╯└────────────────────────────────╯
 Tab switch  ↑↓ move  ⏎ open  Space mark  F5 copy  F6 move  F7 mkdir  F8 delete
 34 item(s) · dir crates · 2026-11-18 21:44 · drwxr-xr-x
```

## Status

`v0.1.0` — the walking skeleton is real, not a mock-up:

| Capability | TUI | GUI |
|---|---|---|
| Two panes, independent directories, shared state | ✅ | ✅ |
| Navigate (enter / up / back / forward) | ✅ | ✅ (back/forward: TUI only) |
| Sort by name / size / time / kind, dirs first, reversible | ✅ | ✅ |
| Hidden files toggle, quick filter (`*` and `?` globs) | ✅ | ✅ |
| Multi-select, select all, clear | ✅ | ✅ |
| Copy / move between panes, background thread, progress, cancel | ✅ | ✅ |
| Delete to trash (reversible) or permanently, with confirmation | ✅ | ✅ |
| New folder / new file / rename | ✅ | ✅ |
| Text viewer with binary detection and 1 MiB budget | ✅ | ✅ |
| Persistent preferences (`~/.config/bifold/config.toml`) | ✅ | ✅ |

Not there yet: file icons/thumbnails, drag & drop, tabs, archive browsing,
remote protocols, macOS `.app` bundle. See [docs/PLAN.md](docs/PLAN.md).

## Build and run

Requires Rust 1.88+.

```bash
cargo build --release

./target/release/bifold                     # terminal UI, two panes at $HOME
./target/release/bifold --path ~/src        # both panes here
./target/release/bifold --left ~/src --right ~/Downloads
./target/release/bifold --dump ~/src        # print both panes as text, no TUI

./target/release/bifold-gui                 # desktop window
```

`make build`, `make test`, `make lint`, `make run-tui` wrap the same commands.

## Terminal keys

| Key | Action | Key | Action |
|---|---|---|---|
| `Tab` | switch pane | `Enter` / `→` / `l` | enter folder, view file |
| `↑` `↓` / `j` `k` | move cursor | `←` / `Backspace` | parent folder |
| `PgUp` `PgDn` `Home` `End` | jump | `Space` / `Insert` | mark and step down |
| `F2` | rename | `F3` | view file |
| `F5` | copy to other pane | `F6` | move to other pane |
| `F7` | new folder | `F8` | delete |
| `Ctrl+N` | new file | `Ctrl+A` / `Ctrl+D` | mark all / clear marks |
| `.` | hidden files | `s` / `o` | cycle sort key / flip order |
| `/` | filter | `g` | go to path |
| `r` | refresh | `b` / `f` | back / forward |
| `?` | help | `q` / `Ctrl+C` | quit |

Esc closes dialogs, clears the filter and the selection. Prompts accept
`~/…` paths; copy and move are pre-filled with the other pane's directory.

## Desktop keys

Same bindings where they exist (`Tab`, arrows, `Enter`, `Backspace`, `Space`,
`F2`, `F3`, `F5`–`F8`, `Del`, `.`, `/`, `Esc`), plus `Cmd+A` / `Cmd+D` /
`Cmd+R` / `Cmd+N` / `Cmd+G`. Mouse: click selects, `Cmd`/`Shift`-click marks,
double-click enters, the divider drags, and the toolbar mirrors every action.

## Configuration

`~/.config/bifold/config.toml` (honours `$XDG_CONFIG_HOME`), written on exit:

```toml
left = "/Users/you/src"
right = "/Users/you/Downloads"
show_hidden = false
sort = "name"          # name | size | time | kind
sort_desc = false
dirs_first = true
delete_mode = "trash"  # trash | permanent
confirm_delete = true
editor = ""
show_permissions = false
```

`--no-config` starts from defaults and writes nothing.

## How it is built

```
crates/
  core/   bifold-core — filesystem model, pane state, session, job engine
  tui/    bifold-tui  — ratatui + crossterm, binary `bifold`
  gui/    bifold-gui  — egui/eframe, binary `bifold-gui`
```

`bifold-core` never imports a UI type. Both front ends drive the same
`Session` (two `Pane`s + one `JobRunner`), so sorting, filtering, selection,
transfer semantics and error messages cannot drift between terminal and
desktop. Long operations run on one worker thread at a time, report
file/byte progress, and are cancellable; deletes go to the trash unless
configured otherwise.

Decisions: [ADR-001 stack](docs/decisions/ADR-001-stack.md),
[ADR-002 shared core](docs/decisions/ADR-002-shared-core.md).
Naming candidates and their collision checks: [docs/naming.md](docs/naming.md).

## Tests

```bash
cargo test --workspace      # 75 tests: core behaviour, TUI key handling, headless egui render
cargo clippy --workspace --all-targets
cargo fmt --all -- --check
```

The TUI is tested by rendering into `ratatui::backend::TestBackend` at several
terminal sizes, so layout regressions fail in CI without a terminal. The GUI is
driven through a headless `egui::Context` in the same way.

## License

MIT — see [LICENSE](LICENSE).
