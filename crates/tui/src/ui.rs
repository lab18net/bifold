//! Rendering. Commander-shaped: two bordered panes, a status line, and modal
//! overlays for prompts, confirmations, progress, viewer and help.

use bifold_core::format::{ellipsis_left, ellipsis_right, human_size, short_time};
use bifold_core::job::Phase;
use bifold_core::session::LEFT;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Gauge, Paragraph, Wrap};

use crate::app::{App, Confirm, Mode, Prompt, Viewer};

const ACCENT: Color = Color::Rgb(96, 208, 192);
const ACCENT_DIM: Color = Color::Rgb(28, 58, 56);
const TEXT: Color = Color::Rgb(220, 224, 228);
const DIM: Color = Color::Rgb(112, 118, 124);
const FAINT: Color = Color::Rgb(80, 86, 92);
const DIR: Color = Color::Rgb(122, 172, 255);
const LINK: Color = Color::Rgb(186, 146, 232);
const MARK: Color = Color::Rgb(232, 200, 122);
const ERROR: Color = Color::Rgb(232, 112, 112);

const SIZE_COL: usize = 7;
const TIME_COL: usize = 12;
const MARKER_COL: usize = 2;

pub fn draw(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    let rows = Layout::vertical([Constraint::Min(3), Constraint::Length(2)]).split(area);
    let panes =
        Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).split(rows[0]);

    draw_pane(frame, app, 0, panes[0]);
    draw_pane(frame, app, 1, panes[1]);
    draw_footer(frame, app, rows[1]);

    let mode = app.mode.clone();
    match mode {
        Mode::Prompt(prompt) => draw_prompt(frame, &prompt, area),
        Mode::Confirm(confirm) => draw_confirm(frame, &confirm, area),
        Mode::Progress => draw_progress(frame, app, area),
        Mode::Viewer(viewer) => draw_viewer(frame, &viewer, area),
        Mode::Help => draw_help(frame, area),
        Mode::Normal => {}
    }
}

fn draw_pane(frame: &mut Frame, app: &mut App, side: usize, area: Rect) {
    let active = app.session.active == side;
    let border = if active {
        ACCENT
    } else {
        Color::Rgb(64, 68, 74)
    };
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
                .fg(Color::Black)
                .bg(ACCENT)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::new().fg(DIM)
        };
        let mut header = vec![
            Span::styled(if side == LEFT { " LEFT " } else { " RIGHT " }, tag_style),
            Span::raw(" "),
            Span::styled(
                ellipsis_left(&pane.path.display().to_string(), width.saturating_sub(16)),
                Style::new().fg(if active { TEXT } else { DIM }),
            ),
        ];
        if !pane.filter.trim().is_empty() {
            header.push(Span::styled(
                format!("  /{}", pane.filter.trim()),
                Style::new().fg(MARK),
            ));
        }
        if !pane.selected.is_empty() {
            header.push(Span::styled(
                format!("  ●{}", pane.selected.len()),
                Style::new().fg(MARK),
            ));
        }
        header.push(Span::styled(
            format!("  {} {}", pane.sort_key.label(), pane.sort_dir.arrow()),
            Style::new().fg(FAINT),
        ));
        lines.push(Line::from(header));

        let name_col = width.saturating_sub(MARKER_COL + SIZE_COL + TIME_COL + 2);
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(pad_right("name", name_col), Style::new().fg(FAINT)),
            Span::styled(pad_left("size", SIZE_COL), Style::new().fg(FAINT)),
            Span::raw(" "),
            Span::styled(pad_left("modified", TIME_COL), Style::new().fg(FAINT)),
        ]));

        if let Some(error) = &pane.error {
            lines.push(Line::from(Span::styled(
                ellipsis_right(error, width),
                Style::new().fg(ERROR),
            )));
        } else if pane.entries.is_empty() {
            lines.push(Line::from(Span::styled(
                "  (empty)",
                Style::new().fg(FAINT),
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

                let mut name_style = Style::new().fg(TEXT);
                if entry.is_dir() {
                    name_style = name_style.fg(DIR).add_modifier(Modifier::BOLD);
                } else if entry.symlink {
                    name_style = name_style.fg(LINK);
                }
                if entry.hidden() {
                    name_style = name_style.fg(FAINT);
                }

                let size_text = if entry.is_dir() {
                    "<dir>".to_string()
                } else {
                    human_size(entry.size)
                };
                let time_text = entry.modified.map(short_time).unwrap_or_default();

                let spans = vec![
                    Span::styled(marker, Style::new().fg(MARK)),
                    Span::styled(
                        pad_right(&ellipsis_right(&entry.name, name_col), name_col),
                        name_style,
                    ),
                    Span::styled(
                        pad_left(&size_text, SIZE_COL),
                        Style::new().fg(if is_cursor { TEXT } else { DIM }),
                    ),
                    Span::raw(" "),
                    Span::styled(pad_left(&time_text, TIME_COL), Style::new().fg(DIM)),
                ];

                let mut line = Line::from(spans);
                if is_cursor {
                    line = line.style(Style::new().bg(if active {
                        ACCENT_DIM
                    } else {
                        Color::Rgb(46, 48, 52)
                    }));
                } else if marked {
                    line = line.style(Style::new().bg(Color::Rgb(42, 38, 26)));
                }
                lines.push(line);
            }
        }
    }

    frame.render_widget(Paragraph::new(lines), inner);
    app.viewport[side] = list_height;
    app.session.panes[side].clamp(list_height);
}

fn draw_footer(frame: &mut Frame, app: &App, area: Rect) {
    let width = area.width as usize;
    let hints = "Tab switch  ↑↓ move  ⏎ open  ⌫ up  Space mark  F3 view  F5 copy  F6 move  F7 mkdir  F8 del  F2 rename  / filter  ? help  q quit";

    let (status, color) = match app.session.job_progress() {
        Some(progress) if !progress.phase.is_finished() => (
            format!(
                "{} — {:>3}%  {}",
                progress.label,
                (progress.ratio() * 100.0).round() as u32,
                progress.current
            ),
            ACCENT,
        ),
        Some(progress) if progress.phase == Phase::Failed => (progress.headline(), ERROR),
        _ => {
            let status = app.session.status();
            let color = if status.starts_with("error") {
                ERROR
            } else {
                TEXT
            };
            (status, color)
        }
    };

    let lines = vec![
        Line::from(Span::styled(
            ellipsis_right(hints, width),
            Style::new().fg(FAINT),
        )),
        Line::from(Span::styled(
            ellipsis_right(&status, width),
            Style::new().fg(color),
        )),
    ];
    frame.render_widget(Paragraph::new(lines), area);
}

fn draw_prompt(frame: &mut Frame, prompt: &Prompt, area: Rect) {
    let pop = centered(area, 76, 4);
    frame.render_widget(Clear, pop);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(ACCENT))
        .title(Span::styled(
            format!(" {} ", prompt.title),
            Style::new().fg(ACCENT).add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(pop);
    frame.render_widget(block, pop);

    let label = format!("{}: ", prompt.kind.label());
    let room = (inner.width as usize).saturating_sub(label.chars().count() + 1);
    let line = Line::from(vec![
        Span::styled(label, Style::new().fg(DIM)),
        Span::styled(ellipsis_left(&prompt.value, room), Style::new().fg(TEXT)),
        Span::styled("▏", Style::new().fg(ACCENT)),
    ]);
    frame.render_widget(Paragraph::new(line), inner);
}

fn draw_confirm(frame: &mut Frame, confirm: &Confirm, area: Rect) {
    let pop = centered(area, 56, 8);
    frame.render_widget(Clear, pop);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(ERROR))
        .title(Span::styled(
            format!(" {} ", confirm.title),
            Style::new().fg(ERROR).add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(pop);
    frame.render_widget(block, pop);
    frame.render_widget(
        Paragraph::new(confirm.body.clone())
            .wrap(Wrap { trim: true })
            .style(Style::new().fg(TEXT)),
        inner,
    );
}

fn draw_progress(frame: &mut Frame, app: &App, area: Rect) {
    let Some(progress) = app.session.job_progress() else {
        return;
    };
    let pop = centered(area, 72, 7);
    frame.render_widget(Clear, pop);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(ACCENT))
        .title(Span::styled(
            " Transfer ",
            Style::new().fg(ACCENT).add_modifier(Modifier::BOLD),
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
            Style::new().fg(TEXT),
        )),
        rows[0],
    );

    let gauge = Gauge::default()
        .ratio(progress.ratio() as f64)
        .label(format!("{:>3}%", (progress.ratio() * 100.0).round() as u32))
        .gauge_style(Style::new().fg(ACCENT).bg(Color::Rgb(38, 42, 46)))
        .use_unicode(true);
    frame.render_widget(gauge, rows[1]);

    frame.render_widget(
        Paragraph::new(Span::styled(
            ellipsis_left(&progress.current, inner.width as usize),
            Style::new().fg(FAINT),
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
        Paragraph::new(Span::styled(hint, Style::new().fg(DIM))),
        rows[3],
    );
}

fn draw_viewer(frame: &mut Frame, viewer: &Viewer, area: Rect) {
    frame.render_widget(Clear, area);
    let title = format!(
        " {}  —  {}  ",
        viewer.preview.title(),
        human_size(viewer.preview.size)
    );
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(ACCENT))
        .title(Span::styled(title, Style::new().fg(TEXT)))
        .title_bottom(Span::styled(
            " ↑↓ / PgUp / PgDn scroll   w wrap   Esc close ",
            Style::new().fg(FAINT),
        ));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let body: Vec<Line> = if viewer.preview.binary {
        vec![Line::from(Span::styled(
            "binary file — no preview",
            Style::new().fg(ERROR),
        ))]
    } else {
        viewer
            .preview
            .lines
            .iter()
            .map(|line| Line::from(Span::raw(line.clone())))
            .collect()
    };

    let mut paragraph = Paragraph::new(body)
        .style(Style::new().fg(TEXT))
        .scroll((viewer.scroll.min(u16::MAX as usize) as u16, 0));
    if viewer.wrap {
        paragraph = paragraph.wrap(Wrap { trim: false });
    }
    frame.render_widget(paragraph, inner);
}

fn draw_help(frame: &mut Frame, area: Rect) {
    frame.render_widget(Clear, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(ACCENT))
        .title(Span::styled(
            " Bifold — help ",
            Style::new().fg(ACCENT).add_modifier(Modifier::BOLD),
        ))
        .title_bottom(Span::styled(" any key to close ", Style::new().fg(FAINT)));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let body: Vec<Line> = crate::HELP
        .lines()
        .map(|line| Line::from(Span::styled(line.to_string(), Style::new().fg(TEXT))))
        .collect();
    frame.render_widget(Paragraph::new(body), inner);
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
}
