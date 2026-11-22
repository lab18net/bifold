//! Colour themes: built-in palettes, user theme files and per-token overrides.
//!
//! A theme is 18 colour tokens named like herdr's (`accent`, `panel_bg`,
//! `text`, `surface0`, …), so a herdr `[theme.custom]` block can be pasted into
//! a Bifold theme file. A token is a colour or `None`, meaning "the terminal's
//! own default" (`reset` in the file) — the desktop front end substitutes a
//! neutral colour for those.
//!
//! User themes are TOML files in `<config dir>/themes/<name>.toml`:
//!
//! ```toml
//! extends = "nord"        # optional; default "bifold"
//! accent = "#ff8800"
//! panel_bg = "reset"
//! ```
//!
//! Colours: `#rrggbb`, `#rgb`, `rgb(r,g,b)`, `ansi:N` (0–255), an ANSI name
//! (`red`, `lightblue`, `darkgray`, …) or `reset`.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::style::Color;
use crate::theme_builtin::BUILTIN;

const MAX_EXTENDS_DEPTH: usize = 8;
pub const DEFAULT_THEME: &str = "bifold";

macro_rules! tokens {
    ($($t:ident),* $(,)?) => {
        /// The colours a front end paints with. See the module docs.
        #[derive(Debug, Clone, PartialEq, Eq)]
        pub struct Theme {
            pub name: String,
            $(pub $t: Option<Color>,)*
        }

        /// Token names accepted in theme files and `theme_custom`.
        pub const TOKENS: &[&str] = &[$(stringify!($t)),*];

        impl Theme {
            fn blank(name: &str) -> Self {
                Self { name: name.to_string(), $($t: None,)* }
            }

            /// Set one token by name; false when the name is unknown.
            pub fn set_token(&mut self, token: &str, value: Option<Color>) -> bool {
                $(if token == stringify!($t) {
                    self.$t = value;
                    return true;
                })*
                false
            }
        }
    };
}

tokens!(
    accent,
    panel_bg,
    active_row_bg,
    selection_bg,
    surface0,
    surface1,
    surface_dim,
    overlay0,
    overlay1,
    text,
    subtext0,
    mauve,
    green,
    yellow,
    red,
    blue,
    teal,
    peach,
);

impl Theme {
    /// The default look.
    pub fn default_theme() -> Self {
        builtin(DEFAULT_THEME).unwrap_or_else(|| Self::blank(DEFAULT_THEME))
    }

    /// Replace the semantic colours previewers and the hex dump use
    /// (`Color::Faint`, `Color::Accent`) with this theme's colours.
    pub fn resolve(&self, color: Color) -> Option<Color> {
        match color {
            Color::Faint => self.overlay1,
            Color::Accent => self.teal,
            other => Some(other),
        }
    }

    /// True for themes with a light background (decides the desktop's base style).
    pub fn is_light(&self) -> bool {
        match self.panel_bg {
            Some(color) => {
                let (r, g, b) = color.to_rgb();
                0.299 * f32::from(r) + 0.587 * f32::from(g) + 0.114 * f32::from(b) > 140.0
            }
            None => false,
        }
    }
}

/// `Nord`, `gruvbox_dark`, `Solarized Light` → `nord`, `gruvbox`, `solarized-light`.
pub fn canonical_name(name: &str) -> String {
    let name = name.trim().to_lowercase().replace([' ', '_'], "-");
    match name.as_str() {
        "gruvbox-dark" => "gruvbox".into(),
        "solarized-dark" => "solarized".into(),
        _ => name,
    }
}

fn safe_name(name: &str) -> bool {
    !name.is_empty() && !name.starts_with('.') && !name.contains(['/', '\\', '\0'])
}

pub fn builtin_names() -> Vec<&'static str> {
    BUILTIN.iter().map(|(name, _)| *name).collect()
}

pub fn builtin(name: &str) -> Option<Theme> {
    let canonical = canonical_name(name);
    let (_, text) = BUILTIN.iter().find(|(n, _)| *n == canonical)?;
    let mut warnings = Vec::new();
    let table: toml::Table = toml::from_str(text).ok()?;
    let mut theme = Theme::blank(&canonical);
    apply_table(&mut theme, &table, &mut warnings, "builtin");
    Some(theme)
}

/// Built-in names followed by the user's theme files, deduplicated.
pub fn available(themes_dir: Option<&Path>) -> Vec<String> {
    let mut names: Vec<String> = builtin_names().into_iter().map(String::from).collect();
    if let Some(dir) = themes_dir
        && let Ok(read) = fs::read_dir(dir)
    {
        let mut user: Vec<String> = read
            .flatten()
            .filter_map(|entry| {
                let path = entry.path();
                if path.extension()? != "toml" {
                    return None;
                }
                Some(canonical_name(&path.file_stem()?.to_string_lossy()))
            })
            .filter(|name| safe_name(name))
            .collect();
        user.sort();
        for name in user {
            if !names.contains(&name) {
                names.push(name);
            }
        }
    }
    names
}

/// Resolve `name` (user file first, then built-in), then apply `overrides` on top.
/// Problems become warnings; an unknown theme falls back to the default.
pub fn resolve(
    name: &str,
    themes_dir: Option<&Path>,
    overrides: &BTreeMap<String, String>,
) -> (Theme, Vec<String>) {
    let mut warnings = Vec::new();
    let mut theme = match load(name, themes_dir, 0, &mut warnings) {
        Some(theme) => theme,
        None => {
            warnings.push(format!(
                "unknown theme '{name}'; using '{DEFAULT_THEME}' (try --list-themes)"
            ));
            Theme::default_theme()
        }
    };
    for (token, raw) in overrides {
        apply_token(&mut theme, token, raw, &mut warnings, "theme_custom");
    }
    (theme, warnings)
}

fn load(
    name: &str,
    themes_dir: Option<&Path>,
    depth: usize,
    warnings: &mut Vec<String>,
) -> Option<Theme> {
    let canonical = canonical_name(name);
    if !safe_name(&canonical) {
        return None;
    }
    if let Some(dir) = themes_dir {
        let path = dir.join(format!("{canonical}.toml"));
        if path.is_file() {
            return load_file(&canonical, &path, themes_dir, depth, warnings);
        }
    }
    builtin(&canonical)
}

fn load_file(
    name: &str,
    path: &Path,
    themes_dir: Option<&Path>,
    depth: usize,
    warnings: &mut Vec<String>,
) -> Option<Theme> {
    let label = format!("theme '{name}'");
    let table: toml::Table = match fs::read_to_string(path)
        .map_err(|e| e.to_string())
        .and_then(|text| toml::from_str(&text).map_err(|e| e.to_string()))
    {
        Ok(table) => table,
        Err(e) => {
            warnings.push(format!("{label}: {e}"));
            return None;
        }
    };
    let base_name = table
        .get("extends")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .unwrap_or_else(|| DEFAULT_THEME.to_string());
    let mut theme = if depth >= MAX_EXTENDS_DEPTH {
        warnings.push(format!("{label}: 'extends' chain too deep (cycle?)"));
        Theme::default_theme()
    } else {
        // A theme may extend the built-in of the same name ("nord" tweaking "nord").
        let own = canonical_name(&base_name) == name;
        let base_dir = if own { None } else { themes_dir };
        match load(&base_name, base_dir, depth + 1, warnings) {
            Some(base) => base,
            None => {
                warnings.push(format!(
                    "{label}: unknown base '{base_name}'; using '{DEFAULT_THEME}'"
                ));
                Theme::default_theme()
            }
        }
    };
    theme.name = name.to_string();
    apply_table(&mut theme, &table, warnings, &label);
    Some(theme)
}

fn apply_table(theme: &mut Theme, table: &toml::Table, warnings: &mut Vec<String>, label: &str) {
    for (key, value) in table {
        if key == "extends" {
            continue;
        }
        match value.as_str() {
            Some(raw) => apply_token(theme, key, raw, warnings, label),
            None => warnings.push(format!("{label}: '{key}' must be a string")),
        }
    }
}

fn apply_token(theme: &mut Theme, token: &str, raw: &str, warnings: &mut Vec<String>, label: &str) {
    match parse_color(raw) {
        Ok(color) => {
            if !theme.set_token(token, color) {
                warnings.push(format!(
                    "{label}: unknown token '{token}' (known: {})",
                    TOKENS.join(", ")
                ));
            }
        }
        Err(e) => warnings.push(format!("{label}: {token}: {e}")),
    }
}

/// `Ok(None)` is "terminal default".
pub fn parse_color(raw: &str) -> Result<Option<Color>, String> {
    let text = raw.trim();
    let lower = text.to_ascii_lowercase();
    if matches!(lower.as_str(), "reset" | "default" | "none" | "") {
        return Ok(None);
    }
    if let Some(hex) = lower.strip_prefix('#') {
        let digit = |i: usize, width: usize| u8::from_str_radix(&hex[i..i + width], 16);
        let bad = || format!("bad colour '{raw}' (use #rrggbb)");
        return match hex.len() {
            _ if !hex.is_ascii() => Err(bad()),
            6 => Ok(Some(Color::Rgb(
                digit(0, 2).map_err(|_| bad())?,
                digit(2, 2).map_err(|_| bad())?,
                digit(4, 2).map_err(|_| bad())?,
            ))),
            3 => {
                let one = |i: usize| digit(i, 1).map(|v| v * 17).map_err(|_| bad());
                Ok(Some(Color::Rgb(one(0)?, one(1)?, one(2)?)))
            }
            _ => Err(bad()),
        };
    }
    if let Some(inner) = lower.strip_prefix("rgb(").and_then(|s| s.strip_suffix(')')) {
        let parts: Vec<_> = inner.split(',').map(|p| p.trim().parse::<u8>()).collect();
        return match parts.as_slice() {
            [Ok(r), Ok(g), Ok(b)] => Ok(Some(Color::Rgb(*r, *g, *b))),
            _ => Err(format!("bad colour '{raw}' (use rgb(0-255,0-255,0-255))")),
        };
    }
    if let Some(n) = lower
        .strip_prefix("ansi:")
        .or_else(|| lower.strip_prefix("indexed:"))
    {
        return n
            .trim()
            .parse::<u8>()
            .map(|n| Some(Color::Indexed(n)))
            .map_err(|_| format!("bad colour '{raw}' (ansi:0-255)"));
    }
    let compact: String = lower
        .chars()
        .filter(|c| !matches!(c, '-' | '_' | ' '))
        .collect::<String>()
        .replace("bright", "light");
    let index = match compact.as_str() {
        "black" => 0,
        "red" => 1,
        "green" => 2,
        "yellow" => 3,
        "blue" => 4,
        "magenta" | "purple" => 5,
        "cyan" => 6,
        "gray" | "grey" => 7,
        "darkgray" | "darkgrey" => 8,
        "lightred" => 9,
        "lightgreen" => 10,
        "lightyellow" => 11,
        "lightblue" => 12,
        "lightmagenta" => 13,
        "lightcyan" => 14,
        "white" => 15,
        _ => return Err(format!("unknown colour '{raw}'")),
    };
    Ok(Some(Color::Indexed(index)))
}

/// One line per theme for `--list-themes`.
pub fn list_text(themes_dir: Option<&Path>) -> String {
    let mut out = String::new();
    for name in available(themes_dir) {
        let user_file = themes_dir
            .map(|dir| dir.join(format!("{name}.toml")))
            .filter(|path| path.is_file());
        match user_file {
            Some(path) if builtin(&name).is_some() => out.push_str(&format!(
                "{name:<18} built-in, overridden by {}\n",
                path.display()
            )),
            Some(path) => out.push_str(&format!("{name:<18} {}\n", path.display())),
            None => out.push_str(&format!("{name:<18} built-in\n")),
        }
    }
    out
}

/// Where user themes live next to the config file.
pub fn themes_dir_in(config_dir: &Path) -> PathBuf {
    config_dir.join("themes")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rgb(r: u8, g: u8, b: u8) -> Option<Color> {
        Some(Color::Rgb(r, g, b))
    }

    #[test]
    fn requested_builtins_exist_and_define_every_token() {
        for name in [
            "terminal",
            "nord",
            "gruvbox",
            "gruvbox-light",
            "solarized",
            "solarized-light",
        ] {
            assert!(builtin_names().contains(&name), "{name} missing");
        }
        for (name, text) in BUILTIN {
            let table: toml::Table = toml::from_str(text).unwrap_or_else(|e| panic!("{name}: {e}"));
            for token in TOKENS {
                assert!(table.contains_key(*token), "{name} lacks {token}");
            }
            assert_eq!(table.len(), TOKENS.len(), "{name} has unknown keys");
            let (theme, warnings) = resolve(name, None, &BTreeMap::new());
            assert!(warnings.is_empty(), "{name}: {warnings:?}");
            assert_eq!(theme.name, *name);
        }
    }

    #[test]
    fn palettes_match_herdr() {
        let nord = builtin("nord").unwrap();
        assert_eq!(nord.accent, rgb(136, 192, 208));
        assert_eq!(nord.panel_bg, rgb(46, 52, 64));
        let gruvbox = builtin("Gruvbox_Dark").unwrap();
        assert_eq!(gruvbox.name, "gruvbox");
        assert_eq!(gruvbox.red, rgb(251, 73, 52));
        let sol = builtin("solarized-light").unwrap();
        assert_eq!(sol.text, rgb(101, 123, 131));
        let term = builtin("terminal").unwrap();
        assert_eq!(
            term.text, None,
            "terminal theme keeps the terminal's text colour"
        );
        assert_eq!(term.accent, Some(Color::Indexed(4)));
        assert_eq!(term.red, Some(Color::Indexed(9)));
    }

    #[test]
    fn light_and_dark_are_told_apart() {
        for light in ["gruvbox-light", "solarized-light"] {
            assert!(builtin(light).unwrap().is_light(), "{light}");
        }
        for dark in ["nord", "gruvbox", "solarized", "terminal", "bifold"] {
            assert!(!builtin(dark).unwrap().is_light(), "{dark}");
        }
    }

    #[test]
    fn parses_colour_syntaxes() {
        assert_eq!(parse_color("#fa0").unwrap(), rgb(255, 170, 0));
        assert_eq!(parse_color("#FF8800").unwrap(), rgb(255, 136, 0));
        assert_eq!(parse_color("rgb(1, 2, 3)").unwrap(), rgb(1, 2, 3));
        assert_eq!(parse_color("ansi:208").unwrap(), Some(Color::Indexed(208)));
        assert_eq!(parse_color("Dark-Gray").unwrap(), Some(Color::Indexed(8)));
        assert_eq!(
            parse_color("bright-blue").unwrap(),
            Some(Color::Indexed(12))
        );
        assert_eq!(parse_color("reset").unwrap(), None);
        for bad in [
            "#12",
            "#gg0000",
            "rgb(1,2)",
            "rgb(300,0,0)",
            "ansi:999",
            "mauve",
            "#é1é1é1",
        ] {
            assert!(parse_color(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn user_theme_extends_a_builtin() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("mine.toml"),
            "extends = \"nord\"\naccent = \"#ff8800\"\npanel_bg = \"reset\"\n",
        )
        .unwrap();
        let (theme, warnings) = resolve("Mine", Some(dir.path()), &BTreeMap::new());
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(theme.name, "mine");
        assert_eq!(theme.accent, rgb(255, 136, 0));
        assert_eq!(theme.panel_bg, None);
        assert_eq!(
            theme.text,
            builtin("nord").unwrap().text,
            "inherits the rest"
        );
    }

    #[test]
    fn user_theme_chains_and_can_shadow_a_builtin() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("base.toml"),
            "extends = \"gruvbox\"\nred = \"#111111\"\n",
        )
        .unwrap();
        fs::write(
            dir.path().join("top.toml"),
            "extends = \"base\"\ngreen = \"#222222\"\n",
        )
        .unwrap();
        fs::write(
            dir.path().join("nord.toml"),
            "extends = \"nord\"\naccent = \"#333333\"\n",
        )
        .unwrap();
        let none = BTreeMap::new();

        let (top, w) = resolve("top", Some(dir.path()), &none);
        assert!(w.is_empty(), "{w:?}");
        assert_eq!((top.red, top.green), (rgb(17, 17, 17), rgb(34, 34, 34)));
        assert_eq!(top.accent, builtin("gruvbox").unwrap().accent);

        let (nord, w) = resolve("nord", Some(dir.path()), &none);
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(
            nord.accent,
            rgb(51, 51, 51),
            "user file wins over the built-in"
        );
        assert_eq!(
            nord.panel_bg,
            builtin("nord").unwrap().panel_bg,
            "while still extending it"
        );
    }

    #[test]
    fn extends_cycles_are_stopped() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a.toml"), "extends = \"b\"\n").unwrap();
        fs::write(dir.path().join("b.toml"), "extends = \"a\"\n").unwrap();
        let (theme, warnings) = resolve("a", Some(dir.path()), &BTreeMap::new());
        assert!(
            warnings.iter().any(|w| w.contains("too deep")),
            "{warnings:?}"
        );
        assert_eq!(theme.accent, Theme::default_theme().accent);
    }

    #[test]
    fn bad_input_warns_and_never_fails() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("messy.toml"),
            "accent = \"#zzz\"\nbogus = \"#fff\"\ntext = 5\nred = \"#010203\"\n",
        )
        .unwrap();
        let (theme, warnings) = resolve("messy", Some(dir.path()), &BTreeMap::new());
        assert_eq!(warnings.len(), 3, "{warnings:?}");
        assert_eq!(theme.red, rgb(1, 2, 3), "good keys still apply");

        let (unknown, warnings) = resolve("does-not-exist", Some(dir.path()), &BTreeMap::new());
        assert_eq!(unknown.name, DEFAULT_THEME);
        assert!(warnings[0].contains("does-not-exist"));

        fs::write(dir.path().join("broken.toml"), "= = =").unwrap();
        let (fallback, warnings) = resolve("broken", Some(dir.path()), &BTreeMap::new());
        assert_eq!(fallback.name, DEFAULT_THEME);
        assert!(warnings.len() >= 2, "{warnings:?}");
    }

    #[test]
    fn path_tricks_in_names_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let (theme, warnings) = resolve("../../etc/passwd", Some(dir.path()), &BTreeMap::new());
        assert_eq!(theme.name, DEFAULT_THEME);
        assert_eq!(warnings.len(), 1);
    }

    #[test]
    fn overrides_apply_on_top() {
        let custom: BTreeMap<String, String> = [("accent", "#010101"), ("nonsense", "#fff")]
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        let (theme, warnings) = resolve("nord", None, &custom);
        assert_eq!(theme.accent, rgb(1, 1, 1));
        assert_eq!(warnings.len(), 1);
    }

    #[test]
    fn available_lists_builtins_then_user_themes() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("zebra.toml"), "").unwrap();
        fs::write(dir.path().join("Alpha Theme.toml"), "").unwrap();
        fs::write(dir.path().join("nord.toml"), "").unwrap();
        fs::write(dir.path().join("notes.txt"), "").unwrap();
        let names = available(Some(dir.path()));
        assert_eq!(&names[..2], ["bifold", "terminal"]);
        assert_eq!(names.iter().filter(|n| *n == "nord").count(), 1);
        let tail: Vec<_> = names.iter().rev().take(2).rev().cloned().collect();
        assert_eq!(tail, ["alpha-theme", "zebra"]);
        assert_eq!(available(None), builtin_names());
    }

    #[test]
    fn semantic_colours_follow_the_theme() {
        let nord = builtin("nord").unwrap();
        assert_eq!(nord.resolve(Color::Faint), nord.overlay1);
        assert_eq!(nord.resolve(Color::Accent), nord.teal);
        assert_eq!(nord.resolve(Color::Indexed(3)), Some(Color::Indexed(3)));
    }
}
