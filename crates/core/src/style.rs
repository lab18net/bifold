//! UI-agnostic styled text: what a previewer prints (ANSI SGR) after parsing.
//!
//! Both front ends map [`Color`] onto their own palette; core never sees a
//! terminal or egui type.

/// A terminal colour: xterm palette index (0–255) or 24-bit RGB.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Color {
    Indexed(u8),
    Rgb(u8, u8, u8),
    /// Dim secondary text; resolved through the active theme (`Theme::resolve`).
    Faint,
    /// Secondary accent (e.g. the hex dump's ASCII gutter); resolved through the theme.
    Accent,
}

impl Color {
    /// Resolve to RGB using the xterm palette (for front ends without one).
    pub fn to_rgb(self) -> (u8, u8, u8) {
        const BASE: [(u8, u8, u8); 16] = [
            (0, 0, 0),
            (205, 49, 49),
            (13, 188, 121),
            (229, 229, 16),
            (36, 114, 200),
            (188, 63, 188),
            (17, 168, 205),
            (229, 229, 229),
            (102, 102, 102),
            (241, 76, 76),
            (35, 209, 139),
            (245, 245, 67),
            (59, 142, 234),
            (214, 112, 214),
            (41, 184, 219),
            (255, 255, 255),
        ];
        match self {
            Self::Rgb(r, g, b) => (r, g, b),
            // Only reached by front ends that skip theme resolution.
            Self::Faint => (100, 106, 112),
            Self::Accent => (17, 168, 205),
            Self::Indexed(n) if n < 16 => BASE[n as usize],
            Self::Indexed(n) if n < 232 => {
                let n = n - 16;
                let level = |v: u8| if v == 0 { 0 } else { 55 + v * 40 };
                (level(n / 36), level((n / 6) % 6), level(n % 6))
            }
            Self::Indexed(n) => {
                let v = 8 + (n - 232) * 10;
                (v, v, v)
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Span {
    pub text: String,
    pub fg: Option<Color>,
    pub bold: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct StyledLine {
    pub spans: Vec<Span>,
}

impl StyledLine {
    pub fn plain_text(text: &str) -> Self {
        Self {
            spans: vec![Span {
                text: sanitize(text),
                ..Default::default()
            }],
        }
    }

    pub fn push(&mut self, text: impl Into<String>, fg: Option<Color>) {
        self.spans.push(Span {
            text: text.into(),
            fg,
            bold: false,
        });
    }

    /// The line without styling.
    pub fn plain(&self) -> String {
        self.spans.iter().map(|s| s.text.as_str()).collect()
    }
}

/// Tabs become spaces and other control characters are dropped, so cell
/// widths stay predictable in both front ends.
pub fn sanitize(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '\t' => out.push_str("    "),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out
}

#[derive(Clone, Copy, Default)]
struct Pen {
    fg: Option<Color>,
    bold: bool,
}

/// Parse text containing ANSI escapes into styled lines. SGR state carries over
/// line breaks; every other escape sequence is discarded.
pub fn parse_ansi(text: &str) -> Vec<StyledLine> {
    let mut lines = Vec::new();
    let mut line = StyledLine::default();
    let mut buf = String::new();
    let mut pen = Pen::default();
    let mut chars = text.chars().peekable();

    fn flush(buf: &mut String, line: &mut StyledLine, pen: Pen) {
        if !buf.is_empty() {
            line.spans.push(Span {
                text: sanitize(buf),
                fg: pen.fg,
                bold: pen.bold,
            });
            buf.clear();
        }
    }

    while let Some(ch) = chars.next() {
        match ch {
            '\n' => {
                flush(&mut buf, &mut line, pen);
                lines.push(std::mem::take(&mut line));
            }
            '\r' => {}
            '\x1b' => match chars.peek().copied() {
                Some('[') => {
                    chars.next();
                    let mut params = String::new();
                    let mut final_byte = None;
                    for c in chars.by_ref() {
                        if ('\x40'..='\x7e').contains(&c) {
                            final_byte = Some(c);
                            break;
                        }
                        params.push(c);
                    }
                    if final_byte == Some('m') {
                        flush(&mut buf, &mut line, pen);
                        apply_sgr(&mut pen, &params);
                    }
                }
                Some(']') => {
                    chars.next();
                    while let Some(c) = chars.next() {
                        if c == '\x07' {
                            break;
                        }
                        if c == '\x1b' {
                            chars.next();
                            break;
                        }
                    }
                }
                // Charset selection (ESC ( B) has one more byte than other two-byte escapes.
                Some('(' | ')' | '*' | '+') => {
                    chars.next();
                    chars.next();
                }
                Some(_) => {
                    chars.next();
                }
                None => {}
            },
            c => buf.push(c),
        }
    }
    flush(&mut buf, &mut line, pen);
    lines.push(line);
    lines
}

fn apply_sgr(pen: &mut Pen, params: &str) {
    let mut codes = params
        .split([';', ':'])
        .map(|p| p.parse::<u16>().unwrap_or(0));
    if params.is_empty() {
        *pen = Pen::default();
        return;
    }
    while let Some(code) = codes.next() {
        match code {
            0 => *pen = Pen::default(),
            1 => pen.bold = true,
            22 => pen.bold = false,
            30..=37 => pen.fg = Some(Color::Indexed((code - 30) as u8)),
            39 => pen.fg = None,
            90..=97 => pen.fg = Some(Color::Indexed((code - 90 + 8) as u8)),
            38 => match codes.next() {
                Some(5) => {
                    if let Some(n) = codes.next() {
                        pen.fg = Some(Color::Indexed(n.min(255) as u8));
                    }
                }
                Some(2) => {
                    let (r, g, b) = (codes.next(), codes.next(), codes.next());
                    if let (Some(r), Some(g), Some(b)) = (r, g, b) {
                        pen.fg = Some(Color::Rgb(
                            r.min(255) as u8,
                            g.min(255) as u8,
                            b.min(255) as u8,
                        ));
                    }
                }
                _ => {}
            },
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_colours_and_resets() {
        let lines = parse_ansi("\x1b[31mred\x1b[0m plain\n\x1b[1;38;5;208mhot\x1b[m");
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].spans[0].fg, Some(Color::Indexed(1)));
        assert_eq!(lines[0].spans[0].text, "red");
        assert_eq!(lines[0].spans[1].fg, None);
        assert_eq!(lines[0].plain(), "red plain");
        assert_eq!(lines[1].spans[0].fg, Some(Color::Indexed(208)));
        assert!(lines[1].spans[0].bold);
    }

    #[test]
    fn truecolor_and_state_across_lines() {
        let lines = parse_ansi("\x1b[38;2;10;20;30ma\nb");
        assert_eq!(lines[0].spans[0].fg, Some(Color::Rgb(10, 20, 30)));
        assert_eq!(lines[1].spans[0].fg, Some(Color::Rgb(10, 20, 30)));
    }

    #[test]
    fn other_escapes_are_stripped() {
        let lines = parse_ansi("a\x1b[2Kb\x1b]0;title\x07c\x1b(Bd\r\n");
        assert_eq!(lines[0].plain(), "abcd");
    }

    #[test]
    fn tabs_expand_and_controls_drop() {
        assert_eq!(sanitize("a\tb\x00c"), "a    bc");
    }

    #[test]
    fn palette_resolves() {
        assert_eq!(Color::Indexed(15).to_rgb(), (255, 255, 255));
        assert_eq!(Color::Indexed(16).to_rgb(), (0, 0, 0));
        assert_eq!(Color::Indexed(231).to_rgb(), (255, 255, 255));
        assert_eq!(Color::Indexed(232).to_rgb(), (8, 8, 8));
    }
}
