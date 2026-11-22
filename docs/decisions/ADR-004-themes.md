# ADR-004: Themes

- Status: accepted
- Date: 2026-11-19
- Deciders: Andrey Abramov (solo)

## Context

Colours were constants in each front end. Requested: the palettes herdr ships
(`terminal`, `nord`, `gruvbox`, `gruvbox-light`, `solarized`,
`solarized-light`) and user-defined themes, in both the terminal and desktop
front ends.

## Decision

`core::theme` owns the model: 18 colour tokens named exactly like herdr's
(`accent`, `panel_bg`, `text`, `surface0`, …). Using herdr's vocabulary keeps
its palettes copyable and lets a herdr `[theme.custom]` block be pasted into a
Bifold theme. A token is a colour or `None`, "the terminal's own default"
(`reset` in files).

- **Built-ins are data, not code.** `theme_builtin.rs` holds each theme as TOML
  text in the same format users write; the six herdr themes were extracted from
  herdr 0.9.5 (`src/app/state.rs`), plus `bifold`, the app's own default (the
  old look), so existing users see no change.
- **User themes** are `<config dir>/themes/<name>.toml` with an optional
  `extends` (built-in or another user theme; depth-limited, cycles reported).
  A user file may shadow a built-in and still extend it.
- **Overrides:** `[theme_custom]` in `config.toml` applies on top of any theme.
- **Failure is soft:** bad colours, unknown tokens, unreadable files and
  unknown names become status-line warnings; the previous or default theme
  stays. Names are validated against path tricks (`../`).
- **Selection:** `theme` in config, `--theme`, the `cycle_theme` action (`t`),
  and a desktop dropdown. `Session::save_config` persists the choice.
- **Rendering:** each front end maps tokens to its own colour type once per
  frame (`Pal`). The TUI repaints the theme background after popups' `Clear`;
  the desktop derives egui `Visuals` (light or dark base) from the theme.
  Colours emitted by core itself (hex dump offsets and ASCII column) are
  semantic (`Color::Faint`, `Color::Accent`) and resolved via the theme, so
  they follow it without core knowing any palette.

## Consequences

- Adding a theme is a TOML file, not a code change; a new token means a field
  in the `tokens!` macro and a use in both `Pal`s.
- `terminal` stays honest in the TUI (ANSI indices and default colours only).
  On the desktop there is no terminal to inherit from, so `reset` tokens use a
  neutral dark fallback and `terminal` looks like a plain dark theme there.
- Tokens herdr defines for its own chrome that Bifold has no use for would be
  dead weight, so each of the 18 is mapped to something real (README table).
- Palettes are ported by value; if herdr changes a palette, Bifold does not
  follow.
- No automatic light/dark switching with the host appearance (herdr has
  `auto_switch`); themes are chosen explicitly.
