#!/usr/bin/env python3
"""Render real Bifold TUI screens for the promo site (site/screens.js).

    uv run --with pyte python scripts/site-screens.py      # needs target/debug/bifold

It builds a throw-away fixture tree, runs the actual binary in a pty for each
theme, reads the terminal state with pyte and writes colour-preserving HTML.
Nothing is mocked: what the page shows is what the program drew.
"""
import fcntl, json, os, pty, select, shutil, struct, subprocess, sys, tempfile, termios, time
from html import escape
from pathlib import Path

import pyte

ROOT = Path(__file__).resolve().parent.parent
BIN = ROOT / "target" / "debug" / "bifold"
OUT = ROOT / "site" / "screens.js"
THEMES = ["nord", "gruvbox", "solarized", "gruvbox-light", "solarized-light", "terminal"]
COLS, ROWS = 96, 19

# pyte reports ANSI colours by name; map them to the xterm defaults.
ANSI = {"black": "#000000", "red": "#cd0000", "green": "#00cd00", "brown": "#cdcd00",
        "blue": "#0000ee", "magenta": "#cd00cd", "cyan": "#00cdcd", "white": "#e5e5e5",
        "brightblack": "#7f7f7f", "brightred": "#ff0000", "brightgreen": "#00ff00",
        "brightbrown": "#ffff00", "brightblue": "#5c5cff", "brightmagenta": "#ff00ff",
        "brightcyan": "#00ffff", "brightwhite": "#ffffff"}


def css(color):
    if color == "default":
        return None
    if color in ANSI:
        return ANSI[color]
    return "#" + color if len(color) == 6 else None


def fixture(base: Path):
    left, right = base / "src" / "bifold", base / "Downloads"
    for d in ("crates/core", "crates/tui", "crates/gui", "docs", "man", "packaging", "scripts"):
        (left / d).mkdir(parents=True)
    (right / "Photos").mkdir(parents=True)
    now = time.time()
    files = {
        left / "Cargo.toml": 1_204, left / "Cargo.lock": 96_410, left / "README.md": 18_330,
        left / "CHANGELOG.md": 2_615, left / "LICENSE": 1_069, left / "Makefile": 1_880,
        right / "Bifold-0.1.0.app.zip": 4_684_196, right / "notes.txt": 912,
        right / "invoice-2026-11.pdf": 241_870, right / "talk.key": 18_400_233,
        right / "screenshot.png": 861_004, right / "archive.tar.gz": 2_204_815,
    }
    for i, (path, size) in enumerate(files.items()):
        path.write_bytes(b"\0" * 0)
        os.truncate(path, size)
        t = now - 3600 * (7 + 11 * i)
        os.utime(path, (t, t))
    for d in list(left.iterdir()) + list(right.iterdir()):
        if d.is_dir():
            os.utime(d, (now - 86400 * 2, now - 86400 * 2))
    return left, right


def capture(theme: str, left: Path, right: Path, cfg: Path) -> str:
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", ROWS, COLS, 0, 0))
    env = dict(os.environ, TERM="xterm-256color", COLORTERM="truecolor", XDG_CONFIG_HOME=str(cfg), HOME=str(left.parent.parent))
    proc = subprocess.Popen([str(BIN), "--theme", theme, "--left", str(left), "--right", str(right)],
                            stdin=slave, stdout=slave, stderr=subprocess.DEVNULL, env=env,
                            start_new_session=True)
    os.close(slave)
    os.set_blocking(master, False)
    screen = pyte.Screen(COLS, ROWS)
    stream = pyte.ByteStream(screen)

    def pump(sec):
        end = time.time() + sec
        while time.time() < end:
            r, _, _ = select.select([master], [], [], 0.05)
            if r:
                try:
                    chunk = os.read(master, 65536)
                except OSError:
                    return
                if b"\x1b[6n" in chunk:
                    os.write(master, b"\x1b[1;1R")
                stream.feed(chunk)

    pump(1.2)
    os.write(master, b"\x1b[B"); pump(0.3)
    os.write(master, b"\x1b[B"); pump(0.3)
    tidy_titles(screen, os.path.realpath(left.parent.parent))
    html = render(screen)
    os.write(master, b"q"); pump(0.4)
    try:
        proc.wait(timeout=5)
    except subprocess.TimeoutExpired:
        proc.kill()
    return html


def tidy_titles(screen, prefix: str):
    """Show the fixture's home as `~` in the pane titles, keeping the sort badge right-aligned."""
    row = screen.buffer[1]
    cells = [row[x] for x in range(screen.columns)]
    text = "".join(c.data for c in cells)
    pos = 0
    while (i := text.find(prefix, pos)) != -1:
        cut = len(prefix) - 1
        j = text.find("  name", i)
        if j == -1:
            break
        cells[i:i + len(prefix)] = [cells[i]._replace(data="~")]
        j -= cut
        cells[j:j] = [cells[j]._replace(data=" ")] * cut
        text = "".join(c.data for c in cells)
        pos = i + 1
    for x, c in enumerate(cells[: screen.columns]):
        row[x] = c


def render(screen) -> str:
    lines = []
    for y in range(screen.lines):
        row, run, key = [], "", None
        for x in range(screen.columns):
            ch = screen.buffer[y][x]
            fg, bg = css(ch.fg), css(ch.bg)
            if ch.reverse:
                fg, bg = bg or "var(--term-bg)", fg or "var(--term-fg)"
            k = (fg, bg, ch.bold, ch.italics)
            if k != key and run:
                row.append(span(run, key)); run = ""
            key = k
            run += ch.data
        row.append(span(run, key))
        lines.append("".join(row).rstrip())
    while lines and not lines[-1].strip():
        lines.pop()
    return "\n".join(lines)


def span(text, key):
    fg, bg, bold, ital = key
    style = []
    if fg: style.append(f"color:{fg}")
    if bg: style.append(f"background:{bg}")
    if bold: style.append("font-weight:700")
    if ital: style.append("font-style:italic")
    text = escape(text)
    return f'<span style="{";".join(style)}">{text}</span>' if style else text


def main():
    if not BIN.exists():
        sys.exit(f"build first: cargo build -p bifold-tui ({BIN} missing)")
    tmp = Path("/tmp/bfs")  # short, so pane titles are not elided
    shutil.rmtree(tmp, ignore_errors=True)
    tmp.mkdir()
    try:
        left, right = fixture(tmp)
        cfg = tmp / "cfg"
        shutil.rmtree(cfg, ignore_errors=True)
        screens = {t: capture(t, left, right, cfg) for t in THEMES}
    finally:
        shutil.rmtree(tmp, ignore_errors=True)
    OUT.write_text("// Generated by scripts/site-screens.py from the real TUI. Do not edit.\n"
                   "window.SCREENS = " + json.dumps(screens, ensure_ascii=False, indent=1) + ";\n")
    print(f"wrote {OUT} ({OUT.stat().st_size // 1024} KiB, themes: {', '.join(screens)})")


main()
