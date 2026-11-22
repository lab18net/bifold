# Plan

## M0 — walking skeleton (done, v0.1.0)

- [x] Workspace: `core` / `tui` / `gui`, one `Session`, one job engine
- [x] Listing, sort (name/size/time/kind), dirs-first, hidden toggle, glob filter
- [x] Multi-select, cursor/page navigation, history (back/forward in the TUI)
- [x] Copy / move with progress, cancel, collision-safe naming (`report 2.pdf`)
- [x] Delete to trash (default) or permanently, with confirmation
- [x] New folder / new file / rename, text viewer, `.env`-style config
- [x] 75 tests: core semantics, TUI key routing + rendered layout, headless GUI render
- [x] Zero clippy warnings, rustfmt clean, CI workflow

## M1 — daily driver (next)

- [ ] **Directory size column** (background `du`, cached per path, invalidated on change)
- [ ] **Open with the system handler** (`open`/`xdg-open`) next to the built-in viewer
- [ ] **Search inside the tree** (`/` filters one directory today; add recursive find with progress + cancel)
- [ ] **Tabs per pane** — several directories per side without losing the two-pane model
- [ ] **Drag & drop** in the GUI, including drag from Finder (`egui` dropped files)
- [ ] **Compare/sync the two panes** — "show only differences", "copy newer"
- [ ] **Batch rename** (pattern + counter) with a dry-run preview
- [ ] **Permissions / owner column** toggle (config field already reserved)
- [ ] **Persist per-pane history and marks** across restarts
- [x] Keybinding config (`[keys]` / `[viewer_keys]`, ADR-003)
- [x] Binary files: hex dump + lf-style `previewer`
- [x] Tab completion in every text input (paths, folders, programs on `PATH`, shell quoting)
- [x] Enter on an executable shows an editable run line; Enter runs it; Ctrl+O shows its output (ADR-005)
- [x] Themes: herdr palettes (terminal, nord, gruvbox, solarized + light variants) and user theme files (ADR-004)
- [ ] Follow the host's light/dark appearance (herdr's `auto_switch`) instead of a fixed theme
- [ ] Bold/underline/background tokens in themes (today: colours only)
- [ ] Run the previewer off the UI thread (job runner) and cache its output
- [ ] Live preview column (lf's main view) driven by the same previewer
- [ ] Image previews (`cleaner`, kitty/sixel) in the TUI, texture in the GUI

## M2 — beyond the local disk

- [ ] Archive browsing (zip/tar read-only) so a `.zip` behaves like a folder
- [ ] Remote panes: SFTP/SSH and S3 behind a `Backend` trait next to `fsops`
- [ ] Duplicate finder (size + hash) with a review step
- [ ] Command bar (`:cp`, `:mv` with globs) for scripted moves
- [ ] File preview for images/PDF in the GUI; hex view for binary in the TUI

## M3 — shipping

- [x] macOS `.app` bundle + icon, ad-hoc signed (`scripts/package-macos.sh`, `make app`)
- [x] Promo site on GitHub Pages (`site/`)
- [x] Homebrew cask `bifold-app` for `Bifold.app` (quarantine cleared; tap only)
- [ ] Notarisation (needs an Apple Developer ID); then drop the quarantine step
- [ ] Homebrew tap (`brew install lab18net/tap/bifold`) — formulae, man pages, release workflow and `scripts/release.sh` are done and verified locally (ADR-006); remaining: create `lab18net/homebrew-tap`, tag `v0.1.0`, publish the first formulae
- [ ] Linux builds in CI (X11/Wayland deps) and a Windows smoke build
- [x] Release automation: `scripts/release.sh` (version bump, changelog, tag), `release.yml` (GitHub release + macOS tarball), `check-release-metadata.sh` in CI
- [ ] Landing page on the Lab18 site with the two-pane screenshot that sells it

## Deliberately out of scope (for now)

- Cloud sync, sharing, or anything account-based
- A plugin system (the `Backend` trait is the extension point if one is needed)
- Editor features inside the viewer — it reads, it does not write
