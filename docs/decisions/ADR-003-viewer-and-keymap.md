# ADR-003: lf-style viewer and a configurable keymap

- Status: accepted
- Date: 2026-11-19
- Deciders: Andrey Abramov (solo)

## Context

v0.1.0 refused to show binary files ("binary file — no preview") and hard-coded
every key twice, once in each front end. Two requests followed: view binary
content, and make key bindings configurable.

## Decision

### Viewer: lf's previewer model, plus a built-in hex fallback

`core::preview::load` returns a `Preview` of `StyledLine`s (spans with optional
colour and bold — no UI types). Resolution order:

1. `previewer` from config, run as `previewer [args] <file> <w> <h> <x> <y>`
   with stdout captured (ANSI SGR parsed in `core::style`), a timeout
   (`previewer_timeout_ms`, default 3000) and a 4 MiB cap.
2. Failure, empty output or timeout falls back to the built-in viewer and records
   a `notice` shown in the title.
3. Built-in: valid UTF-8 text as text; a NUL byte or invalid UTF-8 in the first
   8 KiB means hex (`offset  16 bytes  |ascii|`). `force_hex` skips 1 and 2.

lf's argument order is kept so existing lf previewers work unchanged. Image
previews, a live preview column and caching are left out on purpose.

### Keymap: one vocabulary, in core

`KeySpec` (key + ctrl/alt/shift) and `Action`/`ViewerAction` enums live in
`core::keys`. `Keymap` holds an ordered binding list (the first key of an
action is the one shown in hints), built from defaults plus `[keys]` and
`[viewer_keys]` overrides; `"none"` unbinds; bad entries become startup
warnings, not failures. Each front end only converts its native event to a
`KeySpec`. Actions that touch pane state run in `Session::apply`, so
navigation, marks and sort behave identically; the UI crates handle dialogs,
the viewer and quit.

## Consequences

- Help screens and footer hints are generated from the keymap, so they cannot
  disagree with the config.
- Previewers run synchronously on the UI thread, bounded by the timeout. A slow
  previewer therefore freezes the UI for up to that long; moving it onto the job
  runner is the next step if that bites.
- The `Ctrl`/`Cmd` distinction does not exist: the desktop treats both as
  `ctrl`. A binding that needs Cmd alone is not expressible.
- Terminals cannot distinguish some combinations (`ctrl+i` is `tab`,
  `ctrl+shift+letter` equals `ctrl+letter`), so those are not reliable bindings.
- Dialog-internal keys are fixed (see README).
