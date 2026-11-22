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
| Viewer: text, hex dump for binary files, lf-style external `previewer` | ✅ | ✅ |
| Colour themes: `terminal`, `nord`, `gruvbox`, `gruvbox-light`, `solarized`, `solarized-light` (herdr palettes) + your own | ✅ | ✅ |
| Key bindings configurable in `config.toml` (one keymap, both front ends) | ✅ | ✅ |
| Persistent preferences (`~/.config/bifold/config.toml`) | ✅ | ✅ |

Not there yet: file icons/thumbnails, drag & drop, tabs, archive browsing,
remote protocols, macOS `.app` bundle. See [docs/PLAN.md](docs/PLAN.md).

## Install

**Homebrew** (macOS), from the Lab18 tap:

```bash
brew install lab18net/tap/bifold        # terminal UI, with man page
brew install lab18net/tap/bifold-gui    # desktop window (macOS)
```

**Prebuilt macOS binaries** (Apple silicon) are attached to each
[GitHub release](https://github.com/lab18net/bifold/releases), with a checksum.

**From source** with Rust 1.88+: `cargo install --git
https://github.com/lab18net/bifold --locked bifold-tui` (add `bifold-gui` for the
desktop window). Cargo does not install the man pages; they are in `man/`.

**macOS app via Homebrew** (Apple silicon): `brew install --cask lab18net/tap/bifold-app`
installs `Bifold.app` into /Applications and clears the quarantine flag, since the
app is not notarised yet.

**macOS app, by hand:** each release also has `Bifold-<version>-aarch64.app.zip`. The app
is signed ad hoc and not notarised, so on first launch use right-click → Open
(or `xattr -dr com.apple.quarantine Bifold.app`). Build it yourself with
`make app` (writes `dist/Bifold.app`).

Release process, the Homebrew tap and how to try a formula before it is
published: [packaging/homebrew/README.md](packaging/homebrew/README.md).

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
| `Tab` | switch pane | `Enter` / `→` / `l` | enter folder, view file, or run line for an executable |
| `↑` `↓` / `j` `k` | move cursor | `←` / `Backspace` | parent folder |
| `PgUp` `PgDn` `Home` `End` | jump | `Space` / `Insert` | mark and step down |
| `F2` | rename | `F3` / `F4` | view file / hex dump |
| `F5` | copy to other pane | `F6` | move to other pane |
| `F7` | new folder | `F8` | delete |
| `Ctrl+N` | new file | `Ctrl+A` / `Ctrl+D` | mark all / clear marks |
| `Ctrl+O` | show the terminal (desktop: last output) | `H` | show / hide the key-hint line |
| `.` | hidden files | `s` / `o` | cycle sort key / flip order |
| `/` | filter | `g` | go to path |
| `r` | refresh | `b` / `f` | back / forward |
| `h` | help (fixed) | `t` | cycle colour theme |
| `q` / `Ctrl+C` | quit | | |

Esc closes dialogs, clears the filter and the selection. Prompts accept
`~/…` paths; copy and move are pre-filled with the other pane's directory.

These are the defaults. `bifold --dump-keys` prints the live bindings as TOML
you can paste into your config.

**`h` opens the help**: one screen, in tabs — *Keys* (the main bindings, taken
from your current keymap), *Files* (panes, marks, trash, filter), *Viewer*
(hex, previewer), *Setup* (config, key rebinding, themes with the paths on
this machine) and *All keys* (every action). `←`/`→`/`Tab` or `1`–`5` switch
tabs; `h`, `Esc`, `q` or `Enter` close it. `h` is hard-bound: it works
whatever `[keys]` says, and trying to bind it in `[keys]` is reported as a
config warning. `?` (the configurable `help` action) opens the same screen.

In the viewer: `↑↓`/`j k` and `PgUp PgDn`/`Space` scroll, `g`/`G` jump to the
start/end, `x` (or `F4`) toggles hex, `w` wraps lines (terminal), `Esc`/`q` closes.

## Desktop keys

The same keymap as the terminal — a binding changed in `config.toml` changes
in both. `Ctrl` and `Cmd` are interchangeable (so `ctrl+a` is also `Cmd+A`).
The viewer scrolls with the same keys and the mouse wheel; `w` (wrap) is
terminal-only, the desktop viewer scrolls sideways. Mouse: click selects,
`Cmd`/`Shift`-click marks,
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
show_hints = true      # the key-hint line at the bottom of the terminal UI (H toggles)
theme = "bifold"       # see Themes below
run_executables = true # Enter on an executable shows a run line
run_timeout_ms = 10000 # desktop: leave a still-running command after this long
```

`--no-config` starts from defaults and writes nothing.

**The file is yours.** Bifold edits it in place on exit and writes only the
settings you changed during the session (sort, hidden files, theme, key-hint
line, and the pane folders when you moved). Comments, unknown keys, ordering,
`[keys]`/`[theme_custom]` tables and inline comments stay as you wrote them;
edits you make to the file while Bifold is running are kept too, unless Bifold
changed that same setting. A one-run `--theme` / `--path` is not written back.
A file that does not parse is **never overwritten**: Bifold starts with
defaults, says so in the status line, and leaves the file alone. A symlinked
config (dotfiles repo) is written through the link. With no file yet, a full
default one is created.

### Key bindings

Overrides go in two tables; everything not mentioned keeps its default.
Value `"none"` unbinds a key. Unknown keys or actions are reported in the
status line at startup and skipped.

```toml
[keys]
"ctrl+y" = "copy"        # add a binding
"f7"     = "none"        # remove one
"m"      = "mkdir"

[viewer_keys]
"h" = "toggle_hex"
```

Key syntax: a character (`q`, `G`, `?`, `.`), or a name — `enter esc tab space
backspace delete insert up down left right home end pageup pagedown f1`…`f24`
— with optional `ctrl+` / `alt+` / `shift+` prefixes (`shift+tab`).

Actions (`[keys]`): `quit switch_pane up down page_up page_down home end open
parent back forward toggle_mark mark_all clear_marks view view_hex rename copy
move mkdir new_file delete toggle_hidden cycle_sort toggle_sort_order filter
goto refresh help cancel cycle_theme toggle_hints last_output`.
Actions (`[viewer_keys]`): `close up down page_up page_down top bottom
toggle_wrap toggle_hex`.

Dialog keys (`y`/`n` in confirmations, `Esc`/`Enter`/`Tab` in prompts), `h`, and
the tab keys inside the help screen are not configurable.

### Tab completion in inputs

`Tab` completes in every text input, in both front ends (`Shift+Tab` goes
back):

| Input | Completes |
|---|---|
| Go to, copy/move destination | folders |
| New file / folder, rename, filter | files and folders of the active folder |
| Run line (Enter on an executable) | the last word as a path with shell quoting (`my\ file`); the first word as a program on `PATH` |

Bash-like: a unique match is filled in (a folder gets `/`; in the run line a
file gets a space and an open quote is closed). With several matches the text
grows to their common prefix and the candidates are listed; `Tab` again, with
nothing left to add, cycles through them. Hidden entries are offered only after
you type a dot, `~` expands to your home folder, and paths may be absolute or
relative to the active folder (`../` included). Typing anything ends the cycle.

### Viewer and previewer

The viewer follows lf's model. Without configuration it shows text as text and
**binary files as a hex dump** (offset, 16 bytes, ASCII gutter); `F4` / `x`
forces hex for any file. Set `previewer` and Bifold runs it instead:

```toml
previewer = "~/.config/bifold/previewer"   # or "pistol", "ctpv", …
previewer_timeout_ms = 3000
```

The command is run as `previewer <file> <width> <height> <x> <y>` — lf's
argument order, so lf previewer scripts work unchanged — and its stdout becomes
the viewer content. ANSI colours (16, 256 and truecolor foreground, bold) are
rendered in both front ends; other escape sequences are dropped. Output is
capped at 4 MiB. If the previewer is missing, prints nothing, or exceeds the
timeout (it is killed), the built-in viewer is used and the title says why.
`x`/`F4` bypasses the previewer and shows hex.

Not implemented from lf: image previews (`cleaner`, sixel/kitty), the live
preview column next to the file list, and output caching.

Example previewer (`hexyl` for binaries, `bat` for the rest):

```sh
#!/bin/sh
case "$(file --mime-encoding -b "$1")" in
  binary) hexyl --border none "$1" ;;
  *)      bat --color=always --style=plain --paging=never "$1" ;;
esac
```

### Running programs

`Enter` on an executable file (any execute bit) opens an editable **run line**
pre-filled with `./name` — add arguments, press `Enter` to run, `Esc` to
cancel. It runs through `sh -c` in the active pane's folder. `F3`/`F4` still
view the file, and `run_executables = false` makes `Enter` view it as before.

- **Terminal:** Bifold steps aside and gives the program your real terminal
  (interactive and full-screen programs work, the window can be resized, and
  `Ctrl+C` stops the program, not Bifold). After it exits you see its exit code
  and press `Enter` to come back; both panes are reloaded.
- **Desktop:** the command runs in the background (no stdin), and its output
  and exit code open in the viewer. If it is still running after
  `run_timeout_ms` it is left running and the output so far is shown. One
  command at a time.

**`Ctrl+O`** (action `last_output`). In the **terminal** it switches to your
real terminal, exactly as it is — whatever the last command printed is still
on it, nothing is redrawn — and `Ctrl+O`, `Enter` or `Esc` brings you back
(like Midnight Commander). It works at any time, not only after a command. The
**desktop** has no terminal to switch to, so there it opens the last command's
output in the viewer (capped at 4 MiB).

### Themes

Pick one with `theme = "nord"` in the config, `--theme NAME` for one run, the
`t` key (cycles through all of them and remembers the choice on exit), or the
dropdown in the desktop toolbar. `--list-themes` shows what is available.

| Theme | |
|---|---|
| `bifold` | default: teal on your terminal's own background |
| `terminal` | only your terminal's ANSI palette and default colours |
| `nord`, `gruvbox`, `solarized` | dark |
| `gruvbox-light`, `solarized-light` | light (the desktop switches egui to its light base) |

The built-in palettes (except `bifold`) are taken from herdr 0.9.5 and use
herdr's token names, so a herdr `[theme.custom]` block pastes straight in.

**Your own themes** are files in `~/.config/bifold/themes/<name>.toml`
(`$XDG_CONFIG_HOME/bifold/themes`), selected like any built-in. A file may
`extends` another theme, built-in or yours, and set only what it changes:

```toml
# ~/.config/bifold/themes/sunset.toml
extends = "nord"          # default: "bifold"
accent = "#ff8800"
panel_bg = "#201018"
text = "reset"            # reset = the terminal's own colour
```

A file named like a built-in (`nord.toml`) replaces it; `extends = "nord"`
inside it still reaches the built-in. For quick tweaks without a file, put
tokens in `[theme_custom]` in `config.toml`; they apply on top of whichever
theme is selected.

Colours: `#rrggbb`, `#rgb`, `rgb(r,g,b)`, `ansi:N` (0–255), an ANSI name
(`red`, `lightblue`, `darkgray`, `white`, …) or `reset`. A bad value or unknown
token is reported in the status line (`config: theme 'sunset': …`) and skipped;
an unknown theme name falls back to `bifold`.

| Token | Used for |
|---|---|
| `accent` | active pane border, titles, cursor in prompts |
| `panel_bg` | background (`reset` keeps the terminal's) |
| `active_row_bg` / `surface0` | cursor row in the active / inactive pane |
| `selection_bg` | marked rows |
| `surface1` / `surface_dim` | inactive pane border / progress track |
| `text` / `subtext0` / `overlay0` | normal / secondary / faint text |
| `overlay1` / `teal` | hex dump offsets / ASCII column |
| `blue` / `mauve` / `yellow` | folders / symlinks / marks and filter |
| `red` / `peach` / `green` | errors and delete / config warnings / progress bar |

On the desktop, `reset` tokens fall back to a neutral dark colour because
there is no terminal to inherit from.

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
[ADR-002 shared core](docs/decisions/ADR-002-shared-core.md),
[ADR-003 viewer and keymap](docs/decisions/ADR-003-viewer-and-keymap.md),
[ADR-004 themes](docs/decisions/ADR-004-themes.md).
Naming candidates and their collision checks: [docs/naming.md](docs/naming.md).

## Tests

```bash
cargo test --workspace      # 221 tests: core behaviour, themes, help, command runner, previewer subprocess, TUI keys + rendered colours, headless egui
cargo clippy --workspace --all-targets
cargo fmt --all -- --check
```

The TUI is tested by rendering into `ratatui::backend::TestBackend` at several
terminal sizes, so layout regressions fail in CI without a terminal. The GUI is
driven through a headless `egui::Context` in the same way.

## License

MIT — see [LICENSE](LICENSE).
