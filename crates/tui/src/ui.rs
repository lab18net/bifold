//! Rendering. Commander-shaped: two bordered panes, a status line, and modal
//! overlays for prompts, confirmations, progress, viewer and help.

use bifold_core::format::{ellipsis_left, ellipsis_right, human_size, short_time};
use bifold_core::help::{HelpLine, HelpPage};
use bifold_core::job::Phase;
use bifold_core::keys::{Action, Keymap, ViewerAction};
use bifold_core::session::LEFT;
use bifold_core::style::{Color as CoreColor, StyledLine};
use bifold_core::theme::Theme;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Gauge, Paragraph, Wrap};

use crate::app::{App, Confirm, Mode, Prompt, Viewer};

/// Terminal colours derived from the active theme. `Color::Reset` is "whatever
/// the terminal uses", which is what a theme's `reset` tokens turn into.
#[derive(Clone, Copy)]
struct Pal {
    bg: Color,
    accent: Color,
    on_accent: Color,
    text: Color,
    dim: Color,
    faint: Color,
    dir: Color,
    link: Color,
    mark: Color,
    error: Color,
    warn: Color,
    gauge: Color,
    track: Color,
    border_off: Color,
    cursor_on: Color,
    cursor_off: Color,
    mark_bg: Color,
}

fn conv(color: Option<CoreColor>) -> Color {
    match color {
        Some(CoreColor::Indexed(n)) => Color::Indexed(n),
        Some(CoreColor::Rgb(r, g, b)) => Color::Rgb(r, g, b),
        // Semantic colours are resolved through the theme before they get here.
        Some(CoreColor::Faint) | Some(CoreColor::Accent) | None => Color::Reset,
    }
}

impl Pal {
    fn new(theme: &Theme) -> Self {
        let on_accent = match theme.accent {
            Some(accent) => {
                let (r, g, b) = accent.to_rgb();
                if 0.299 * f32::from(r) + 0.587 * f32::from(g) + 0.114 * f32::from(b) > 140.0 {
                    Color::Black
                } else {
                    Color::White
                }
            }
            None => Color::Black,
        };
        Self {
            bg: conv(theme.panel_bg),
            accent: conv(theme.accent),
            on_accent,
            text: conv(theme.text),
            dim: conv(theme.subtext0),
            faint: conv(theme.overlay0),
            dir: conv(theme.blue),
            link: conv(theme.mauve),
            mark: conv(theme.yellow),
            error: conv(theme.red),
            warn: conv(theme.peach),
            gauge: conv(theme.green),
            track: conv(theme.surface_dim),
            border_off: conv(theme.surface1),
            cursor_on: conv(theme.active_row_bg),
            cursor_off: conv(theme.surface0),
            mark_bg: conv(theme.selection_bg),
        }
    }
}

/// `Clear` resets cells to the terminal default; repaint the theme background over it.
fn clear(frame: &mut Frame, pal: &Pal, area: Rect) {
    frame.render_widget(Clear, area);
    if pal.bg != Color::Reset {
        frame.render_widget(Block::default().style(Style::new().bg(pal.bg)), area);
    }
}

const SIZE_COL: usize = 7;
const TIME_COL: usize = 12;
const MARKER_COL: usize = 2;

pub fn draw(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    let rows = Layout::vertical([Constraint::Min(3), Constraint::Length(2)]).split(area);
    let panes =
        Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).split(rows[0]);

    let theme = app.session.theme.clone();
    let pal = Pal::new(&theme);
    let pal = &pal;
    if pal.bg != Color::Reset {
        frame.render_widget(Block::default().style(Style::new().bg(pal.bg)), area);
    }

    draw_pane(frame, app, pal, 0, panes[0]);
    draw_pane(frame, app, pal, 1, panes[1]);
    draw_footer(frame, app, pal, rows[1]);

    let mode = app.mode.clone();
    match mode {
        Mode::Prompt(prompt) => draw_prompt(frame, pal, &prompt, area),
        Mode::Confirm(confirm) => draw_confirm(frame, pal, &confirm, area),
        Mode::Progress => draw_progress(frame, app, pal, area),
        Mode::Viewer(viewer) => draw_viewer(frame, pal, &theme, &app.session.keymap, &viewer, area),
        Mode::Help(tab) => draw_help(frame, pal, &app.session.help_pages(), tab, area),
        Mode::Normal => {}
    }
}

fn draw_pane(frame: &mut Frame, app: &mut App, pal: &Pal, side: usize, area: Rect) {
    let active = app.session.active == side;
    let border = if active { pal.accent } else { pal.border_off };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(if active {
            BorderType::Rounded
        } else {
            BorderType::Plain
        })
        .border_style(Style::new().fg(border));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.width < 16 || inner.height < 3 {
        return;
    }

    let mut lines: Vec<Line> = Vec::with_capacity(inner.height as usize);
    let list_height = (inner.height as usize).saturating_sub(2);

    {
        let pane = &app.session.panes[side];
        let width = inner.width as usize;
        let tag_style = if active {
            Style::new()
                .fg(pal.on_accent)
                .bg(pal.accent)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::new().fg(pal.dim)
        };
        let mut header = vec![
            Span::styled(if side == LEFT { " LEFT " } else { " RIGHT " }, tag_style),
            Span::raw(" "),
            Span::styled(
                ellipsis_left(&pane.path.display().to_string(), width.saturating_sub(16)),
                Style::new().fg(if active { pal.text } else { pal.dim }),
            ),
        ];
        if !pane.filter.trim().is_empty() {
            header.push(Span::styled(
                format!("  /{}", pane.filter.trim()),
                Style::new().fg(pal.mark),
            ));
        }
        if !pane.selected.is_empty() {
            header.push(Span::styled(
                format!("  ●{}", pane.selected.len()),
                Style::new().fg(pal.mark),
            ));
        }
        header.push(Span::styled(
            format!("  {} {}", pane.sort_key.label(), pane.sort_dir.arrow()),
            Style::new().fg(pal.faint),
        ));
        lines.push(Line::from(header));

        let name_col = width.saturating_sub(MARKER_COL + SIZE_COL + TIME_COL + 2);
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(pad_right("name", name_col), Style::new().fg(pal.faint)),
            Span::styled(pad_left("size", SIZE_COL), Style::new().fg(pal.faint)),
            Span::raw(" "),
            Span::styled(pad_left("modified", TIME_COL), Style::new().fg(pal.faint)),
        ]));

        if let Some(error) = &pane.error {
            lines.push(Line::from(Span::styled(
                ellipsis_right(error, width),
                Style::new().fg(pal.error),
            )));
        } else if pane.entries.is_empty() {
            lines.push(Line::from(Span::styled(
                "  (empty)",
                Style::new().fg(pal.faint),
            )));
        } else {
            let end = (pane.offset + list_height).min(pane.entries.len());
            for index in pane.offset..end {
                let entry = &pane.entries[index];
                let is_cursor = index == pane.cursor;
                let marked = pane.selected.contains(&entry.path);

                let marker = if is_cursor {
                    "› "
                } else if marked {
                    "▌ "
                } else {
                    "  "
                };

                let mut name_style = Style::new().fg(pal.text);
                if entry.is_dir() {
                    name_style = name_style.fg(pal.dir).add_modifier(Modifier::BOLD);
                } else if entry.symlink {
                    name_style = name_style.fg(pal.link);
                }
                if entry.hidden() {
                    name_style = name_style.fg(pal.faint);
                }

                let size_text = if entry.is_dir() {
                    "<dir>".to_string()
                } else {
                    human_size(entry.size)
                };
                let time_text = entry.modified.map(short_time).unwrap_or_default();

                let spans = vec![
                    Span::styled(marker, Style::new().fg(pal.mark)),
                    Span::styled(
                        pad_right(&ellipsis_right(&entry.name, name_col), name_col),
                        name_style,
                    ),
                    Span::styled(
                        pad_left(&size_text, SIZE_COL),
                        Style::new().fg(if is_cursor { pal.text } else { pal.dim }),
                    ),
                    Span::raw(" "),
                    Span::styled(pad_left(&time_text, TIME_COL), Style::new().fg(pal.dim)),
                ];

                let mut line = Line::from(spans);
                if is_cursor {
                    line = line.style(Style::new().bg(if active {
                        pal.cursor_on
                    } else {
                        pal.cursor_off
                    }));
                } else if marked {
                    line = line.style(Style::new().bg(pal.mark_bg));
                }
                lines.push(line);
            }
        }
    }

    frame.render_widget(Paragraph::new(lines), inner);
    app.viewport[side] = list_height;
    app.session.panes[side].clamp(list_height);
}

fn draw_footer(frame: &mut Frame, app: &App, pal: &Pal, area: Rect) {
    let width = area.width as usize;
    let hints = app.session.keymap.hints(&[
        (Action::SwitchPane, "switch"),
        (Action::Open, "open"),
        (Action::Parent, "up"),
        (Action::ToggleMark, "mark"),
        (Action::View, "view"),
        (Action::ViewHex, "hex"),
        (Action::Copy, "copy"),
        (Action::Move, "move"),
        (Action::MkDir, "mkdir"),
        (Action::Delete, "del"),
        (Action::Rename, "rename"),
        (Action::Filter, "filter"),
    ]);
    let quit = app.session.keymap.hints(&[(Action::Quit, "quit")]);
    let hints = format!("h help  {hints}  {quit}");

    let (status, color) = match app.session.job_progress() {
        Some(progress) if !progress.phase.is_finished() => (
            format!(
                "{} — {:>3}%  {}",
                progress.label,
                (progress.ratio() * 100.0).round() as u32,
                progress.current
            ),
            pal.accent,
        ),
        Some(progress) if progress.phase == Phase::Failed => (progress.headline(), pal.error),
        _ => {
            let status = app.session.status();
            let color = if status.starts_with("error") {
                pal.error
            } else if status.starts_with("config:") {
                pal.warn
            } else {
                pal.text
            };
            (status, color)
        }
    };

    let lines = vec![
        Line::from(Span::styled(
            ellipsis_right(&hints, width),
            Style::new().fg(pal.faint),
        )),
        Line::from(Span::styled(
            ellipsis_right(&status, width),
            Style::new().fg(color),
        )),
    ];
    frame.render_widget(Paragraph::new(lines), area);
}

fn draw_prompt(frame: &mut Frame, pal: &Pal, prompt: &Prompt, area: Rect) {
    let pop = centered(area, 76, 4);
    clear(frame, pal, pop);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(pal.accent))
        .title(Span::styled(
            format!(" {} ", prompt.title),
            Style::new().fg(pal.accent).add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(pop);
    frame.render_widget(block, pop);

    let label = format!("{}: ", prompt.kind.label());
    let room = (inner.width as usize).saturating_sub(label.chars().count() + 1);
    let line = Line::from(vec![
        Span::styled(label, Style::new().fg(pal.dim)),
        Span::styled(
            ellipsis_left(&prompt.value, room),
            Style::new().fg(pal.text),
        ),
        Span::styled("▏", Style::new().fg(pal.accent)),
    ]);
    frame.render_widget(Paragraph::new(line), inner);
}

fn draw_confirm(frame: &mut Frame, pal: &Pal, confirm: &Confirm, area: Rect) {
    let pop = centered(area, 56, 8);
    clear(frame, pal, pop);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(pal.error))
        .title(Span::styled(
            format!(" {} ", confirm.title),
            Style::new().fg(pal.error).add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(pop);
    frame.render_widget(block, pop);
    frame.render_widget(
        Paragraph::new(confirm.body.clone())
            .wrap(Wrap { trim: true })
            .style(Style::new().fg(pal.text)),
        inner,
    );
}

fn draw_progress(frame: &mut Frame, app: &App, pal: &Pal, area: Rect) {
    let Some(progress) = app.session.job_progress() else {
        return;
    };
    let pop = centered(area, 72, 7);
    clear(frame, pal, pop);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(pal.accent))
        .title(Span::styled(
            " Transfer ",
            Style::new().fg(pal.accent).add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(pop);
    frame.render_widget(block, pop);

    let rows = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .split(inner);

    let label = format!(
        "{}  ·  {} / {} files  ·  {} / {}",
        progress.label,
        progress.files_done,
        progress.files_total,
        human_size(progress.bytes_done),
        human_size(progress.bytes_total)
    );
    frame.render_widget(
        Paragraph::new(Span::styled(
            ellipsis_right(&label, inner.width as usize),
            Style::new().fg(pal.text),
        )),
        rows[0],
    );

    let gauge = Gauge::default()
        .ratio(progress.ratio() as f64)
        .label(format!("{:>3}%", (progress.ratio() * 100.0).round() as u32))
        .gauge_style(Style::new().fg(pal.gauge).bg(pal.track))
        .use_unicode(true);
    frame.render_widget(gauge, rows[1]);

    frame.render_widget(
        Paragraph::new(Span::styled(
            ellipsis_left(&progress.current, inner.width as usize),
            Style::new().fg(pal.faint),
        )),
        rows[2],
    );

    let hint = match progress.phase {
        Phase::Working => "Esc / c — cancel",
        Phase::Done => "Enter — close",
        Phase::Cancelled => "Enter — close",
        Phase::Failed => "Enter — close",
    };
    frame.render_widget(
        Paragraph::new(Span::styled(hint, Style::new().fg(pal.dim))),
        rows[3],
    );
}

fn draw_viewer(
    frame: &mut Frame,
    pal: &Pal,
    theme: &Theme,
    keymap: &Keymap,
    viewer: &Viewer,
    area: Rect,
) {
    clear(frame, pal, area);
    let title = format!(
        " {}  —  {}  ",
        viewer.preview.title(),
        human_size(viewer.preview.size)
    );
    let hint = keymap.viewer_hints(&[
        (ViewerAction::Up, "scroll"),
        (ViewerAction::ToggleHex, "hex"),
        (ViewerAction::ToggleWrap, "wrap"),
        (ViewerAction::Close, "close"),
    ]);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(pal.accent))
        .title(Span::styled(title, Style::new().fg(pal.text)))
        .title_bottom(Span::styled(
            format!(" {hint} "),
            Style::new().fg(pal.faint),
        ));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Only the visible slice is converted: previews can hold tens of thousands of lines.
    let body: Vec<Line> = viewer
        .preview
        .lines
        .iter()
        .skip(viewer.scroll)
        .take(inner.height as usize)
        .map(|line| styled_line(line, theme))
        .collect();

    let mut paragraph = Paragraph::new(body).style(Style::new().fg(pal.text));
    if viewer.wrap {
        paragraph = paragraph.wrap(Wrap { trim: false });
    }
    frame.render_widget(paragraph, inner);
}

fn styled_line(line: &StyledLine, theme: &Theme) -> Line<'static> {
    Line::from(
        line.spans
            .iter()
            .map(|span| {
                let mut style = Style::new();
                if let Some(color) = span.fg.and_then(|c| theme.resolve(c)) {
                    style = style.fg(conv(Some(color)));
                }
                if span.bold {
                    style = style.add_modifier(Modifier::BOLD);
                }
                Span::styled(span.text.clone(), style)
            })
            .collect::<Vec<_>>(),
    )
}

fn draw_help(frame: &mut Frame, pal: &Pal, pages: &[HelpPage], tab: usize, area: Rect) {
    clear(frame, pal, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(pal.accent))
        .title(Span::styled(
            " Bifold — help ",
            Style::new().fg(pal.accent).add_modifier(Modifier::BOLD),
        ))
        .title_bottom(Span::styled(
            " ←/→ or 1-5 switch tab · h / Esc close ",
            Style::new().fg(pal.faint),
        ));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.height < 2 {
        return;
    }
    let rows = Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).split(inner);

    let mut tabs: Vec<Span> = Vec::new();
    for (index, page) in pages.iter().enumerate() {
        let label = format!(" {} {} ", index + 1, page.title);
        tabs.push(if index == tab {
            Span::styled(
                label,
                Style::new()
                    .fg(pal.on_accent)
                    .bg(pal.accent)
                    .add_modifier(Modifier::BOLD),
            )
        } else {
            Span::styled(label, Style::new().fg(pal.dim))
        });
        tabs.push(Span::raw(" "));
    }
    frame.render_widget(Paragraph::new(Line::from(tabs)), rows[0]);

    let Some(page) = pages.get(tab) else {
        return;
    };
    let body: Vec<Line> = page
        .lines
        .iter()
        .map(|line| match line {
            HelpLine::Heading(text) => Line::from(Span::styled(
                text.clone(),
                Style::new().fg(pal.accent).add_modifier(Modifier::BOLD),
            )),
            HelpLine::Key(keys, text) => Line::from(vec![
                Span::styled(format!("{keys:<22}"), Style::new().fg(pal.mark)),
                Span::styled(text.clone(), Style::new().fg(pal.text)),
            ]),
            HelpLine::Note(text) => {
                Line::from(Span::styled(text.clone(), Style::new().fg(pal.text)))
            }
        })
        .collect();
    frame.render_widget(Paragraph::new(body).wrap(Wrap { trim: false }), rows[1]);
}

// -- helpers ---------------------------------------------------------------

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    Rect {
        x: area.x + (area.width.saturating_sub(width)) / 2,
        y: area.y + (area.height.saturating_sub(height)) / 2,
        width,
        height,
    }
}

fn pad_right(text: &str, width: usize) -> String {
    let len = text.chars().count();
    if len >= width {
        return ellipsis_right(text, width);
    }
    format!("{text}{}", " ".repeat(width - len))
}

fn pad_left(text: &str, width: usize) -> String {
    let len = text.chars().count();
    if len >= width {
        return ellipsis_right(text, width);
    }
    format!("{}{text}", " ".repeat(width - len))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use bifold_core::session::Session;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use crate::app::App;

    fn render(app: &mut App, width: u16, height: u16) -> String {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| super::draw(frame, app)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        let mut out = String::new();
        for y in 0..buffer.area.height {
            for x in 0..buffer.area.width {
                out.push_str(buffer[(x, y)].symbol());
            }
            out.push('\n');
        }
        out
    }

    fn fixture() -> (tempfile::TempDir, App) {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("left")).unwrap();
        fs::create_dir(dir.path().join("right")).unwrap();
        fs::write(dir.path().join("left/alpha.txt"), b"alpha").unwrap();
        fs::write(dir.path().join("left/beta.md"), b"beta").unwrap();
        let session = Session::new_at(&dir.path().join("left"), &dir.path().join("right"));
        (dir, App::new(session))
    }

    #[test]
    fn draws_both_panes_with_contents() {
        let (_dir, mut app) = fixture();
        let screen = render(&mut app, 120, 30);
        assert!(screen.contains("LEFT"));
        assert!(screen.contains("RIGHT"));
        assert!(screen.contains("alpha.txt"), "{screen}");
        assert!(screen.contains("beta.md"));
        assert!(screen.contains("name"));
        assert!(screen.contains("modified"));
    }

    #[test]
    fn draws_prompt_overlay() {
        let (_dir, mut app) = fixture();
        app.on_key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::F(7),
            crossterm::event::KeyModifiers::NONE,
        ));
        app.on_key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('d'),
            crossterm::event::KeyModifiers::NONE,
        ));
        let screen = render(&mut app, 100, 24);
        assert!(screen.contains("New folder"), "{screen}");
        assert!(screen.contains("name: d"), "{screen}");
    }

    #[test]
    fn viewer_renders_file_contents() {
        let (_dir, mut app) = fixture();
        app.on_key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::F(3),
            crossterm::event::KeyModifiers::NONE,
        ));
        let screen = render(&mut app, 100, 24);
        assert!(screen.contains("alpha"), "{screen}");
    }

    #[test]
    fn narrow_terminal_does_not_panic() {
        let (_dir, mut app) = fixture();
        for (w, h) in [(20u16, 8u16), (40, 6), (12, 4), (200, 60)] {
            let screen = render(&mut app, w, h);
            assert!(!screen.is_empty());
        }
    }

    #[test]
    fn empty_directory_shows_placeholder() {
        let (_dir, mut app) = fixture();
        let screen = render(&mut app, 100, 20);
        assert!(screen.contains("(empty)"));
    }

    fn key(code: crossterm::event::KeyCode) -> crossterm::event::KeyEvent {
        crossterm::event::KeyEvent::new(code, crossterm::event::KeyModifiers::NONE)
    }

    #[test]
    fn binary_file_renders_as_hex_dump() {
        let (dir, mut app) = fixture();
        fs::write(dir.path().join("left/zz.bin"), b"\x00\x01Hello\xff").unwrap();
        app.session.pane_mut().reload();
        let at = app
            .session
            .pane()
            .entries
            .iter()
            .position(|e| e.name == "zz.bin")
            .unwrap();
        app.session.pane_mut().cursor = at;
        app.on_key(key(crossterm::event::KeyCode::F(3)));
        let screen = render(&mut app, 100, 20);
        assert!(screen.contains("[hex]"), "{screen}");
        assert!(
            screen.contains("00000000  00 01 48 65 6c 6c 6f ff"),
            "{screen}"
        );
        assert!(screen.contains("|..Hello.|"), "{screen}");
        assert!(
            screen.contains("x hex"),
            "footer hint comes from the keymap: {screen}"
        );
    }

    fn press(app: &mut App, ch: char) {
        app.on_key(key(crossterm::event::KeyCode::Char(ch)));
    }

    #[test]
    fn h_opens_a_tabbed_help_and_every_tab_renders() {
        let (_dir, mut app) = fixture();
        press(&mut app, 'h');
        let screen = render(&mut app, 100, 30);
        for title in ["1 Keys", "2 Files", "3 Viewer", "4 Setup", "5 All keys"] {
            assert!(screen.contains(title), "{title}: {screen}");
        }
        assert!(
            screen.contains("move the cursor") && screen.contains("this help (fixed"),
            "{screen}"
        );

        for (tab, needle) in [
            ('2', "target of copy and move"),
            ('3', "hex dump"),
            ('4', "bifold --dump-keys"),
            ('5', "Copy to other pane"),
        ] {
            press(&mut app, tab);
            let screen = render(&mut app, 100, 60);
            assert!(
                screen.contains(needle),
                "tab {tab} lacks {needle}: {screen}"
            );
        }
    }

    #[test]
    fn help_shows_rebound_keys_on_the_first_tab() {
        let (_dir, mut app) = fixture();
        app.session.keymap = {
            let mut config = bifold_core::config::Config::default();
            config.keys.insert("ctrl+y".into(), "copy".into());
            config.keys.insert("f5".into(), "none".into());
            config.keymap().0
        };
        press(&mut app, 'h');
        let screen = render(&mut app, 100, 30);
        assert!(screen.contains("ctrl+y / f6"), "{screen}");
        press(&mut app, '5');
        assert!(render(&mut app, 100, 60).contains("ctrl+y"));
    }

    #[test]
    fn each_main_tab_fits_an_80x24_terminal() {
        let (_dir, mut app) = fixture();
        press(&mut app, 'h');
        for (tab, last_line) in [
            ('1', "quit"),
            ('2', "remembered when you quit."),
            ('3', "built-in viewer is used."),
            ('4', "bifold --list-themes."),
        ] {
            press(&mut app, tab);
            let screen = render(&mut app, 80, 24);
            // Notes wrap, so compare with borders removed and whitespace collapsed.
            let flat = screen
                .replace('│', " ")
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            assert!(
                flat.contains(last_line),
                "tab {tab} is cut off before '{last_line}':\n{screen}"
            );
        }
    }

    #[test]
    fn footer_advertises_the_fixed_help_key() {
        let (_dir, mut app) = fixture();
        assert!(
            render(&mut app, 80, 20).contains("h help"),
            "the footer is cut off at 80 columns, so the help hint must come first"
        );
    }

    #[test]
    fn viewer_and_help_survive_tiny_terminals() {
        let (_dir, mut app) = fixture();
        app.on_key(key(crossterm::event::KeyCode::F(3)));
        for (w, h) in [(12u16, 4u16), (30, 5), (200, 60)] {
            assert!(!render(&mut app, w, h).is_empty());
        }
        app.on_key(key(crossterm::event::KeyCode::Esc));
        app.on_key(key(crossterm::event::KeyCode::Char('?')));
        assert!(!render(&mut app, 12, 4).is_empty());
    }

    fn buffer(app: &mut App, width: u16, height: u16) -> ratatui::buffer::Buffer {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| super::draw(frame, app)).unwrap();
        terminal.backend().buffer().clone()
    }

    #[test]
    fn theme_paints_background_and_accent() {
        use ratatui::style::Color;
        let (_dir, mut app) = fixture();
        assert!(app.session.set_theme("nord"));
        let buf = buffer(&mut app, 100, 20);
        // Border of the active pane uses the accent; every cell sits on the theme background.
        assert_eq!(buf[(0, 0)].fg, Color::Rgb(136, 192, 208));
        for (x, y) in [(0u16, 0u16), (50, 10), (99, 19), (10, 18)] {
            assert_eq!(buf[(x, y)].bg, Color::Rgb(46, 52, 64), "cell {x},{y}");
        }
        // The inactive pane's border is the dimmer surface colour.
        assert_eq!(buf[(50, 0)].fg, Color::Rgb(67, 76, 94));
    }

    #[test]
    fn light_theme_is_applied_too() {
        use ratatui::style::Color;
        let (_dir, mut app) = fixture();
        assert!(app.session.set_theme("gruvbox-light"));
        let buf = buffer(&mut app, 100, 20);
        assert_eq!(buf[(10, 10)].bg, Color::Rgb(251, 241, 199));
        assert_eq!(buf[(0, 0)].fg, Color::Rgb(7, 102, 120));
    }

    #[test]
    fn terminal_theme_keeps_the_terminal_background_and_ansi_accent() {
        use ratatui::style::Color;
        let (_dir, mut app) = fixture();
        assert!(app.session.set_theme("terminal"));
        let buf = buffer(&mut app, 100, 20);
        assert_eq!(buf[(10, 10)].bg, Color::Reset);
        assert_eq!(buf[(0, 0)].fg, Color::Indexed(4));
    }

    #[test]
    fn popups_keep_the_theme_background() {
        use ratatui::style::Color;
        let (_dir, mut app) = fixture();
        assert!(app.session.set_theme("solarized"));
        app.on_key(key(crossterm::event::KeyCode::F(7)));
        let buf = buffer(&mut app, 100, 20);
        // Centre of the 76x4 prompt popup: cleared, then repainted with the theme background.
        assert_eq!(buf[(50, 9)].bg, Color::Rgb(0, 43, 54));
        assert!(render(&mut app, 100, 20).contains("New folder"));
    }

    #[test]
    fn t_cycles_the_theme_and_the_screen_follows() {
        use ratatui::style::Color;
        let (_dir, mut app) = fixture();
        let before = buffer(&mut app, 100, 20)[(0, 0)].fg;
        app.on_key(key(crossterm::event::KeyCode::Char('t')));
        assert_eq!(app.session.theme.name, "terminal", "bifold -> terminal");
        let after = buffer(&mut app, 100, 20)[(0, 0)].fg;
        assert_ne!(before, after);
        assert_eq!(after, Color::Indexed(4));
        assert!(render(&mut app, 100, 20).contains("theme: terminal"));
    }

    #[test]
    fn viewer_hex_colours_follow_the_theme() {
        use ratatui::style::Color;
        let (dir, mut app) = fixture();
        std::fs::write(dir.path().join("left/zz.bin"), [0u8, 1, 2, 3]).unwrap();
        app.session.pane_mut().reload();
        let at = app
            .session
            .pane()
            .entries
            .iter()
            .position(|e| e.name == "zz.bin")
            .unwrap();
        app.session.pane_mut().cursor = at;
        assert!(app.session.set_theme("nord"));
        app.on_key(key(crossterm::event::KeyCode::F(3)));
        let buf = buffer(&mut app, 100, 20);
        // First content row, first cell: the offset column is the theme's overlay1.
        assert_eq!(buf[(1, 1)].fg, Color::Rgb(100, 110, 130));
        assert_eq!(buf[(1, 1)].bg, Color::Rgb(46, 52, 64));
    }
}
