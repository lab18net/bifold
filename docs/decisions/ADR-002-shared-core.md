# ADR-002: One core, two front ends

- Status: accepted
- Date: 2026-11-18
- Deciders: Andrey Abramov (solo)

## Context

The product has to feel identical in a terminal and in a desktop window: same
sorting, same filter syntax, same "copy the marked items to the other pane",
same collision naming (`report 2.pdf`), same error wording, same trash-first
delete. Two independent implementations would drift within a week.

## Decision

A single state machine in `bifold-core`, rendered by two thin adapters.

```
                 ┌──────────────────────────────┐
   keys ────────►│ tui::app (Mode, Prompt,      │
                 │ Confirm, Viewer)             │──┐
                 └──────────────────────────────┘  │  calls
                                                    ├──► Session (core)
                 ┌──────────────────────────────┐  │      ├─ panes[2]
   mouse/keys ──►│ gui::GuiApp (Dialog, Viewer) │──┘      ├─ JobRunner
                 └──────────────────────────────┘         └─ Config
```

- `Pane` owns directory listing, cursor, marks, sort, filter, history, and
  `targets()` (marked entries, or the cursor row) — the definition of "what an
  operation acts on" exists once.
- `Session` owns the two panes, the active side, the job runner and the status
  message. `start_copy` / `start_move` / `start_delete` / `make_dir` /
  `make_file` / `rename_cursor` validate, measure totals, spawn the worker and
  set the message; the front ends only decide *when* to call them.
- `JobRunner` runs one job at a time on a `std::thread`, publishing a
  `Progress` snapshot plus `OpStats`; both front ends poll it per frame and draw
  the same numbers.
- UI-only concerns stay in the UI crates: `Mode`/`Prompt`/`Confirm` and key
  mapping in `tui`, `Dialog`/scroll state in `gui`. Core has no idea a dialog
  exists.

## Consequences

- **Parity is testable.** Core tests assert behaviour; front-end tests assert
  rendering and key routing. A new operation needs a core test and two thin
  wirings, not two implementations.
- **Blocking work must not run on the UI thread.** Anything that can take longer
  than a frame goes through `JobRunner`; measuring totals happens synchronously
  today (metadata-only walk) and is the first candidate to move into the worker
  if huge trees become a problem.
- **Refusals are shared.** "Cannot move X inside itself", "a transfer is already
  running", "name cannot be empty" are produced in core, so the terminal footer
  and the desktop status bar show the same text.
- Adding a third front end (web, iOS) means rendering `Session`, not re-deriving
  file semantics.
