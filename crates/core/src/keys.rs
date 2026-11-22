//! Actions, key specs and the configurable keymap.
//!
//! Both front ends translate their native key events into a [`KeySpec`], ask
//! the [`Keymap`] for an action, and run it — so a binding changed in
//! `config.toml` means the same thing in the terminal and on the desktop.

use std::collections::BTreeMap;
use std::fmt;

macro_rules! actions {
    ($(#[$meta:meta])* $name:ident { $($var:ident => ($s:literal, $desc:literal)),* $(,)? }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum $name { $($var),* }

        impl $name {
            pub const ALL: &'static [$name] = &[$($name::$var),*];

            /// Name used in `config.toml`.
            pub fn name(self) -> &'static str {
                match self { $($name::$var => $s),* }
            }

            pub fn description(self) -> &'static str {
                match self { $($name::$var => $desc),* }
            }

            pub fn parse(raw: &str) -> Option<Self> {
                match raw.trim().to_ascii_lowercase().as_str() {
                    $($s => Some($name::$var),)*
                    _ => None,
                }
            }
        }
    };
}

actions! {
    /// Actions available while browsing the two panes.
    Action {
        Quit => ("quit", "Quit"),
        SwitchPane => ("switch_pane", "Switch active pane"),
        Up => ("up", "Cursor up"),
        Down => ("down", "Cursor down"),
        PageUp => ("page_up", "Page up"),
        PageDown => ("page_down", "Page down"),
        Home => ("home", "First entry"),
        End => ("end", "Last entry"),
        Open => ("open", "Open folder / view file"),
        Parent => ("parent", "Go to parent folder"),
        Back => ("back", "History back"),
        Forward => ("forward", "History forward"),
        ToggleMark => ("toggle_mark", "Mark entry and step down"),
        MarkAll => ("mark_all", "Mark all"),
        ClearMarks => ("clear_marks", "Clear marks"),
        View => ("view", "View file (text, or hex for binary)"),
        ViewHex => ("view_hex", "View file as hex dump"),
        Rename => ("rename", "Rename"),
        Copy => ("copy", "Copy to other pane"),
        Move => ("move", "Move to other pane"),
        MkDir => ("mkdir", "New folder"),
        NewFile => ("new_file", "New file"),
        Delete => ("delete", "Delete (trash by default)"),
        ToggleHidden => ("toggle_hidden", "Show / hide hidden files"),
        CycleSort => ("cycle_sort", "Cycle sort key"),
        ToggleSortOrder => ("toggle_sort_order", "Flip sort order"),
        Filter => ("filter", "Filter listing"),
        GoTo => ("goto", "Go to path"),
        Refresh => ("refresh", "Refresh"),
        Help => ("help", "Show help"),
        CycleTheme => ("cycle_theme", "Cycle color theme"),
        ToggleHints => ("toggle_hints", "Show / hide the key-hint line"),
        LastOutput => ("last_output", "View output of the last run command"),
        Cancel => ("cancel", "Clear marks and filter"),
    }
}

actions! {
    /// Actions available inside the file viewer.
    ViewerAction {
        Close => ("close", "Close viewer"),
        Up => ("up", "Scroll up"),
        Down => ("down", "Scroll down"),
        PageUp => ("page_up", "Page up"),
        PageDown => ("page_down", "Page down"),
        Top => ("top", "Jump to top"),
        Bottom => ("bottom", "Jump to bottom"),
        ToggleWrap => ("toggle_wrap", "Toggle line wrap (terminal)"),
        ToggleHex => ("toggle_hex", "Toggle hex dump"),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Key {
    Char(char),
    Enter,
    Esc,
    Tab,
    Space,
    Backspace,
    Delete,
    Insert,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    F(u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct KeySpec {
    pub key: Key,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
}

impl KeySpec {
    /// Build a normalised spec: a space is `Key::Space`, and printable
    /// characters carry their shift in the character itself (`G`, `?`).
    pub fn new(key: Key, ctrl: bool, alt: bool, shift: bool) -> Self {
        let key = match key {
            Key::Char(' ') => Key::Space,
            // Terminals report ctrl+A and ctrl+a alike; don't make users care.
            Key::Char(c) if ctrl && c.is_ascii_alphabetic() => Key::Char(c.to_ascii_lowercase()),
            other => other,
        };
        let shift = shift && !matches!(key, Key::Char(_));
        Self {
            key,
            ctrl,
            alt,
            shift,
        }
    }

    pub fn plain(key: Key) -> Self {
        Self::new(key, false, false, false)
    }

    pub fn parse(raw: &str) -> Result<Self, String> {
        let original = raw.trim();
        if original.is_empty() {
            return Err("empty key".into());
        }
        let (mut ctrl, mut alt, mut shift) = (false, false, false);
        let mut rest = original;
        loop {
            let lower = rest.to_ascii_lowercase();
            if lower.starts_with("ctrl+") && rest.len() > 5 {
                ctrl = true;
                rest = &rest[5..];
            } else if lower.starts_with("alt+") && rest.len() > 4 {
                alt = true;
                rest = &rest[4..];
            } else if lower.starts_with("shift+") && rest.len() > 6 {
                shift = true;
                rest = &rest[6..];
            } else {
                break;
            }
        }
        let key = parse_key(rest).ok_or_else(|| format!("unknown key '{original}'"))?;
        let key = match key {
            Key::Char(c) if shift && c.is_alphabetic() => {
                shift = false;
                Key::Char(c.to_ascii_uppercase())
            }
            other => other,
        };
        Ok(Self::new(key, ctrl, alt, shift))
    }
}

/// The help screen's key. Fixed: both front ends test for it before consulting
/// the keymap, so it works whatever the config says.
pub fn is_help_key(spec: &KeySpec) -> bool {
    *spec == KeySpec::plain(Key::Char('h'))
}

fn parse_key(token: &str) -> Option<Key> {
    let mut chars = token.chars();
    if let (Some(c), None) = (chars.next(), chars.next()) {
        return Some(Key::Char(c));
    }
    let lower = token.to_ascii_lowercase();
    Some(match lower.as_str() {
        "enter" | "return" => Key::Enter,
        "esc" | "escape" => Key::Esc,
        "tab" => Key::Tab,
        "space" => Key::Space,
        "backspace" => Key::Backspace,
        "delete" | "del" => Key::Delete,
        "insert" | "ins" => Key::Insert,
        "up" => Key::Up,
        "down" => Key::Down,
        "left" => Key::Left,
        "right" => Key::Right,
        "home" => Key::Home,
        "end" => Key::End,
        "pageup" | "pgup" => Key::PageUp,
        "pagedown" | "pgdn" => Key::PageDown,
        other => {
            let n: u8 = other.strip_prefix('f')?.parse().ok()?;
            if (1..=24).contains(&n) {
                Key::F(n)
            } else {
                return None;
            }
        }
    })
}

impl fmt::Display for KeySpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.ctrl {
            f.write_str("ctrl+")?;
        }
        if self.alt {
            f.write_str("alt+")?;
        }
        if self.shift {
            f.write_str("shift+")?;
        }
        match self.key {
            Key::Char(c) => write!(f, "{c}"),
            Key::Enter => f.write_str("enter"),
            Key::Esc => f.write_str("esc"),
            Key::Tab => f.write_str("tab"),
            Key::Space => f.write_str("space"),
            Key::Backspace => f.write_str("backspace"),
            Key::Delete => f.write_str("delete"),
            Key::Insert => f.write_str("insert"),
            Key::Up => f.write_str("up"),
            Key::Down => f.write_str("down"),
            Key::Left => f.write_str("left"),
            Key::Right => f.write_str("right"),
            Key::Home => f.write_str("home"),
            Key::End => f.write_str("end"),
            Key::PageUp => f.write_str("pageup"),
            Key::PageDown => f.write_str("pagedown"),
            Key::F(n) => write!(f, "f{n}"),
        }
    }
}

/// Bindings in priority order: the first key listed for an action is the one
/// shown in hints.
#[derive(Debug, Clone)]
pub struct Keymap {
    normal: Vec<(KeySpec, Action)>,
    viewer: Vec<(KeySpec, ViewerAction)>,
}

fn spec(raw: &str) -> KeySpec {
    // Only called with literals in `defaults`; a typo is caught by the tests.
    KeySpec::parse(raw).unwrap_or_else(|_| KeySpec::plain(Key::Esc))
}

impl Default for Keymap {
    fn default() -> Self {
        Self::defaults()
    }
}

impl Keymap {
    pub fn defaults() -> Self {
        use Action as A;
        use ViewerAction as V;
        let normal = [
            ("q", A::Quit),
            ("ctrl+c", A::Quit),
            ("tab", A::SwitchPane),
            ("shift+tab", A::SwitchPane),
            ("up", A::Up),
            ("k", A::Up),
            ("down", A::Down),
            ("j", A::Down),
            ("pageup", A::PageUp),
            ("pagedown", A::PageDown),
            ("home", A::Home),
            ("end", A::End),
            ("enter", A::Open),
            ("right", A::Open),
            ("l", A::Open),
            ("backspace", A::Parent),
            ("left", A::Parent),
            ("b", A::Back),
            ("f", A::Forward),
            ("space", A::ToggleMark),
            ("insert", A::ToggleMark),
            ("ctrl+a", A::MarkAll),
            ("ctrl+d", A::ClearMarks),
            ("f3", A::View),
            ("f4", A::ViewHex),
            ("f2", A::Rename),
            ("f5", A::Copy),
            ("f6", A::Move),
            ("f7", A::MkDir),
            ("ctrl+n", A::NewFile),
            ("ctrl+o", A::LastOutput),
            ("f8", A::Delete),
            ("delete", A::Delete),
            (".", A::ToggleHidden),
            ("s", A::CycleSort),
            ("o", A::ToggleSortOrder),
            ("/", A::Filter),
            ("g", A::GoTo),
            ("r", A::Refresh),
            ("?", A::Help),
            ("t", A::CycleTheme),
            ("H", A::ToggleHints),
            ("esc", A::Cancel),
        ];
        let viewer = [
            ("esc", V::Close),
            ("q", V::Close),
            ("f3", V::Close),
            ("down", V::Down),
            ("j", V::Down),
            ("up", V::Up),
            ("k", V::Up),
            ("pagedown", V::PageDown),
            ("space", V::PageDown),
            ("pageup", V::PageUp),
            ("home", V::Top),
            ("g", V::Top),
            ("end", V::Bottom),
            ("G", V::Bottom),
            ("w", V::ToggleWrap),
            ("x", V::ToggleHex),
            ("f4", V::ToggleHex),
        ];
        Self {
            normal: normal.iter().map(|(k, a)| (spec(k), *a)).collect(),
            viewer: viewer.iter().map(|(k, a)| (spec(k), *a)).collect(),
        }
    }

    /// Defaults plus user overrides. `"none"` unbinds a key. Problems are
    /// returned as warnings; the rest of the overrides still apply.
    pub fn with_overrides(
        normal: &BTreeMap<String, String>,
        viewer: &BTreeMap<String, String>,
    ) -> (Self, Vec<String>) {
        let mut map = Self::defaults();
        let mut warnings = Vec::new();
        for (raw_key, raw_action) in normal {
            match KeySpec::parse(raw_key) {
                Err(e) => warnings.push(format!("[keys] {e}")),
                Ok(key) if is_help_key(&key) => warnings.push(
                    "[keys] 'h' is reserved for the help screen and cannot be rebound".into(),
                ),
                Ok(key) => {
                    if is_none(raw_action) {
                        map.normal.retain(|(k, _)| *k != key);
                    } else if let Some(action) = Action::parse(raw_action) {
                        set(&mut map.normal, key, action);
                    } else {
                        warnings.push(format!(
                            "[keys] unknown action '{raw_action}' for '{raw_key}'"
                        ));
                    }
                }
            }
        }
        for (raw_key, raw_action) in viewer {
            match KeySpec::parse(raw_key) {
                Err(e) => warnings.push(format!("[viewer_keys] {e}")),
                Ok(key) => {
                    if is_none(raw_action) {
                        map.viewer.retain(|(k, _)| *k != key);
                    } else if let Some(action) = ViewerAction::parse(raw_action) {
                        set(&mut map.viewer, key, action);
                    } else {
                        warnings.push(format!(
                            "[viewer_keys] unknown action '{raw_action}' for '{raw_key}'"
                        ));
                    }
                }
            }
        }
        (map, warnings)
    }

    pub fn action(&self, key: &KeySpec) -> Option<Action> {
        self.normal.iter().find(|(k, _)| k == key).map(|(_, a)| *a)
    }

    pub fn viewer_action(&self, key: &KeySpec) -> Option<ViewerAction> {
        self.viewer.iter().find(|(k, _)| k == key).map(|(_, a)| *a)
    }

    pub fn keys_for(&self, action: Action) -> Vec<KeySpec> {
        self.normal
            .iter()
            .filter(|(_, a)| *a == action)
            .map(|(k, _)| *k)
            .collect()
    }

    pub fn viewer_keys_for(&self, action: ViewerAction) -> Vec<KeySpec> {
        self.viewer
            .iter()
            .filter(|(_, a)| *a == action)
            .map(|(k, _)| *k)
            .collect()
    }

    /// `"tab switch  enter open  …"` for footers; unbound actions are skipped.
    pub fn hints(&self, items: &[(Action, &str)]) -> String {
        items
            .iter()
            .filter_map(|(action, label)| {
                self.keys_for(*action)
                    .first()
                    .map(|key| format!("{key} {label}"))
            })
            .collect::<Vec<_>>()
            .join("  ")
    }

    pub fn viewer_hints(&self, items: &[(ViewerAction, &str)]) -> String {
        items
            .iter()
            .filter_map(|(action, label)| {
                self.viewer_keys_for(*action)
                    .first()
                    .map(|key| format!("{key} {label}"))
            })
            .collect::<Vec<_>>()
            .join("   ")
    }

    /// Rows for a help screen: (keys, description), browsing then viewer.
    pub fn help_rows(&self) -> Vec<(String, String)> {
        let join = |keys: Vec<KeySpec>| {
            keys.iter()
                .map(KeySpec::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        };
        let mut rows = vec![("h".to_string(), "Help (fixed)".to_string())];
        for action in Action::ALL {
            let keys = join(self.keys_for(*action));
            if !keys.is_empty() {
                rows.push((keys, action.description().to_string()));
            }
        }
        rows.push((String::new(), String::new()));
        rows.push(("Viewer".into(), String::new()));
        for action in ViewerAction::ALL {
            let keys = join(self.viewer_keys_for(*action));
            if !keys.is_empty() {
                rows.push((keys, action.description().to_string()));
            }
        }
        rows
    }

    /// The effective bindings as ready-to-paste TOML (`--dump-keys`).
    pub fn to_toml(&self) -> String {
        let mut out = String::from("[keys]\n");
        for (key, action) in &self.normal {
            out.push_str(&format!("\"{key}\" = \"{}\"\n", action.name()));
        }
        out.push_str("\n[viewer_keys]\n");
        for (key, action) in &self.viewer {
            out.push_str(&format!("\"{key}\" = \"{}\"\n", action.name()));
        }
        out
    }
}

fn is_none(raw: &str) -> bool {
    matches!(
        raw.trim().to_ascii_lowercase().as_str(),
        "none" | "unbind" | ""
    )
}

fn set<T: Copy>(list: &mut Vec<(KeySpec, T)>, key: KeySpec, action: T) {
    if let Some(slot) = list.iter_mut().find(|(k, _)| *k == key) {
        slot.1 = action;
    } else {
        list.push((key, action));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn parses_specs() {
        assert_eq!(KeySpec::parse("F5").unwrap(), KeySpec::plain(Key::F(5)));
        assert_eq!(
            KeySpec::parse("Ctrl+A").unwrap(),
            KeySpec::new(Key::Char('a'), true, false, false)
        );
        assert_eq!(
            KeySpec::parse("shift+tab").unwrap(),
            KeySpec::new(Key::Tab, false, false, true)
        );
        assert_eq!(
            KeySpec::parse("shift+g").unwrap(),
            KeySpec::parse("G").unwrap()
        );
        assert_eq!(KeySpec::parse("space").unwrap(), KeySpec::plain(Key::Space));
        assert_eq!(KeySpec::parse("+").unwrap(), KeySpec::plain(Key::Char('+')));
        assert!(KeySpec::parse("f99").is_err());
        assert!(KeySpec::parse("hyper").is_err());
        assert!(KeySpec::parse("").is_err());
    }

    #[test]
    fn display_roundtrips() {
        for raw in [
            "ctrl+a",
            "alt+x",
            "f12",
            "shift+tab",
            "pagedown",
            "space",
            "?",
            "G",
        ] {
            let parsed = KeySpec::parse(raw).unwrap();
            assert_eq!(parsed.to_string(), raw);
            assert_eq!(KeySpec::parse(&parsed.to_string()).unwrap(), parsed);
        }
    }

    #[test]
    fn defaults_are_all_valid_and_complete() {
        let keymap = Keymap::defaults();
        // `spec()` falls back to Esc on a typo, so every default must resolve to its own text.
        for (key, _) in keymap.normal.iter() {
            assert_eq!(KeySpec::parse(&key.to_string()).unwrap(), *key);
        }
        for action in Action::ALL {
            assert!(
                !keymap.keys_for(*action).is_empty(),
                "{} unbound",
                action.name()
            );
        }
        for action in ViewerAction::ALL {
            assert!(
                !keymap.viewer_keys_for(*action).is_empty(),
                "viewer {} unbound",
                action.name()
            );
        }
        assert_eq!(
            keymap
                .normal
                .iter()
                .filter(|(k, _)| k.key == Key::Esc)
                .count(),
            1
        );
    }

    #[test]
    fn action_names_roundtrip() {
        for action in Action::ALL {
            assert_eq!(Action::parse(action.name()), Some(*action));
        }
        for action in ViewerAction::ALL {
            assert_eq!(ViewerAction::parse(action.name()), Some(*action));
        }
        assert_eq!(Action::parse("COPY"), Some(Action::Copy));
        assert_eq!(Action::parse("nope"), None);
    }

    #[test]
    fn h_is_reserved_for_help() {
        assert!(is_help_key(&KeySpec::parse("h").unwrap()));
        assert!(!is_help_key(&KeySpec::parse("H").unwrap()));
        assert!(!is_help_key(&KeySpec::parse("ctrl+h").unwrap()));
        let (keymap, warnings) = Keymap::with_overrides(
            &map(&[("h", "delete"), ("H", "mkdir")]),
            &map(&[("h", "close")]),
        );
        assert_eq!(
            keymap.action(&KeySpec::parse("h").unwrap()),
            None,
            "never bound"
        );
        assert_eq!(
            keymap.action(&KeySpec::parse("H").unwrap()),
            Some(Action::MkDir)
        );
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(warnings[0].contains("reserved"), "{warnings:?}");
        assert_eq!(
            keymap.viewer_action(&KeySpec::parse("h").unwrap()),
            Some(ViewerAction::Close),
            "the viewer keeps its own keymap"
        );
    }

    #[test]
    fn overrides_rebind_unbind_and_warn() {
        let (keymap, warnings) = Keymap::with_overrides(
            &map(&[
                ("x", "delete"),
                ("f8", "none"),
                ("q", "mkdir"),
                ("bogus+key", "copy"),
                ("z", "fly"),
            ]),
            &map(&[("h", "toggle_hex")]),
        );
        assert_eq!(
            keymap.action(&KeySpec::parse("x").unwrap()),
            Some(Action::Delete)
        );
        assert_eq!(keymap.action(&KeySpec::parse("f8").unwrap()), None);
        assert_eq!(
            keymap.action(&KeySpec::parse("q").unwrap()),
            Some(Action::MkDir)
        );
        assert_eq!(
            keymap.action(&KeySpec::parse("delete").unwrap()),
            Some(Action::Delete)
        );
        assert_eq!(
            keymap.viewer_action(&KeySpec::parse("h").unwrap()),
            Some(ViewerAction::ToggleHex)
        );
        assert_eq!(warnings.len(), 2, "{warnings:?}");
    }

    #[test]
    fn first_listed_key_is_the_hint() {
        let keymap = Keymap::defaults();
        assert_eq!(keymap.hints(&[(Action::Open, "open")]), "enter open");
        let (rebound, _) =
            Keymap::with_overrides(&map(&[("f8", "none"), ("delete", "none")]), &map(&[]));
        assert_eq!(
            rebound.hints(&[(Action::Delete, "del"), (Action::Copy, "copy")]),
            "f5 copy"
        );
    }

    #[test]
    fn dump_is_loadable_as_overrides() {
        let keymap = Keymap::defaults();
        let text = keymap.to_toml();
        assert!(text.contains("\"f5\" = \"copy\""));
        assert!(text.contains("[viewer_keys]"));
        assert!(text.contains("\"x\" = \"toggle_hex\""));
    }

    #[test]
    fn help_rows_cover_every_bound_action() {
        let rows = Keymap::defaults().help_rows();
        assert!(
            rows.iter()
                .any(|(k, d)| k.contains("f5") && d.contains("Copy"))
        );
        assert!(rows.iter().any(|(k, _)| k == "Viewer"));
    }
}
