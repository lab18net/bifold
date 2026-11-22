# ADR-005: Running programs from the file list

## Context

Enter on an executable file opened the viewer, which shows a script's source
but makes it awkward to *run* it. Orthodox file managers (mc, Norton) show a
command line pre-filled with the file name so the user can add arguments and
press Enter. Bifold has two front ends: the terminal one owns the terminal the
program wants to use, the desktop one has none.

## Decision

1. **Enter on an executable** (a regular file with any execute bit) opens an
   editable run line pre-filled with `./name` (single-quoted when the name has
   shell-special characters). Enter runs it, Esc cancels. `F3`/`F4` still view
   the file. `run_executables = false` restores the old behaviour.
2. **The command runs through `sh -c` in the active pane's folder.** The user
   typed or confirmed the line, so it is a shell line, like a terminal prompt.
3. **One module, two run modes** in `core::runcmd`:
   - `run_inherited` — the TUI leaves the alternate screen and gives the
     command the real terminal, like a shell does: interactive and full-screen
     programs work and the window can be resized. It waits for Enter so the
     output can be read, then restores itself and reloads both panes. Ctrl+C
     while it runs interrupts the command, not Bifold (SIGINT is ignored in
     the parent and reset to default in the child).
   - `run_captured` — the GUI runs it on a worker thread with stdout and
     stderr merged on one pipe (ordered, and shell syntax errors are caught).
     Output is capped at the viewer's 4 MiB and shown in the viewer
     (`PreviewKind::Output`). At `run_timeout_ms` (default 10 s) a command
     that is still running is **left running**, not killed — it may be a GUI
     program the user just launched — and its partial output is shown.
4. One command at a time in the GUI, matching the one-transfer rule.
5. **`Ctrl+O` (`Action::LastOutput`)** shows the terminal. In the TUI that is
   the real terminal, untouched: the command's output is already on its main
   screen from when it ran, so nothing is recorded or replayed (an earlier
   design recorded a pty stream and replayed it, which only duplicated what
   was on screen). It works with or without a previous command. The desktop
   has no terminal, so it shows the last run (`Session::last_run`, filled by
   `run_captured`) in the viewer (`PreviewKind::Output`).

## Consequences

- No interpreter is guessed from the extension or shebang; the shell and the
  kernel do that, so `./x` behaves exactly as in a terminal.
- A GUI command has no stdin (null), so a program that asks questions gets EOF.
  Run it from the terminal front end for that.
- The desktop cannot kill a detached command; it is reaped when it ends.
- Unix only for now (`sh`, execute bits). On other platforms `is_executable`
  is false and Enter keeps opening the viewer.
