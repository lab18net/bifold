//! Content of the one-screen help (`h`): a few tabs of key bindings and short
//! notes on the main features. UI-agnostic — both front ends render
//! [`HelpPage`]s. Key rows are generated from the live [`Keymap`], so rebinding
//! a key in `config.toml` shows up here; `h` itself is fixed.

use crate::keys::{Action, Keymap, ViewerAction};

pub use crate::keys::is_help_key;

/// Tab titles in order; page `i` of [`pages`] has title `TITLES[i]`.
pub const TITLES: [&str; 5] = ["Keys", "Files", "Viewer", "Setup", "All keys"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HelpLine {
    Heading(String),
    /// (keys, what they do)
    Key(String, String),
    Note(String),
}

#[derive(Debug, Clone)]
pub struct HelpPage {
    pub title: &'static str,
    pub lines: Vec<HelpLine>,
}

/// Facts about this install that the notes quote.
#[derive(Debug, Clone)]
pub struct HelpContext {
    pub config_dir: String,
    pub theme: String,
    pub themes: Vec<String>,
}

/// Up to `per` keys of each action, `" / "`-joined: `up / down`.
fn keys_of(map: &Keymap, actions: &[Action], per: usize) -> String {
    let mut parts = Vec::new();
    for action in actions {
        parts.extend(
            map.keys_for(*action)
                .iter()
                .take(per)
                .map(|k| k.to_string()),
        );
    }
    parts.join(" / ")
}

fn viewer_keys_of(map: &Keymap, actions: &[ViewerAction], per: usize) -> String {
    let mut parts = Vec::new();
    for action in actions {
        parts.extend(
            map.viewer_keys_for(*action)
                .iter()
                .take(per)
                .map(|k| k.to_string()),
        );
    }
    parts.join(" / ")
}

fn key(map: &Keymap, actions: &[Action], per: usize, text: &str) -> HelpLine {
    let keys = keys_of(map, actions, per);
    HelpLine::Key(
        if keys.is_empty() {
            "(unbound)".into()
        } else {
            keys
        },
        text.into(),
    )
}

fn vkey(map: &Keymap, actions: &[ViewerAction], per: usize, text: &str) -> HelpLine {
    let keys = viewer_keys_of(map, actions, per);
    HelpLine::Key(
        if keys.is_empty() {
            "(unbound)".into()
        } else {
            keys
        },
        text.into(),
    )
}

fn heading(text: &str) -> HelpLine {
    HelpLine::Heading(text.into())
}

fn note(text: impl Into<String>) -> HelpLine {
    HelpLine::Note(text.into())
}

pub fn pages(map: &Keymap, ctx: &HelpContext) -> Vec<HelpPage> {
    use Action as A;
    let keys = vec![
        heading("Move"),
        key(map, &[A::Up, A::Down], 1, "move the cursor"),
        key(map, &[A::SwitchPane], 1, "switch pane"),
        key(map, &[A::Open, A::Parent], 1, "open folder or file / go up"),
        heading("Work"),
        key(
            map,
            &[A::ToggleMark],
            1,
            "mark an entry (actions then use the marks)",
        ),
        key(map, &[A::Copy, A::Move], 1, "copy / move to the other pane"),
        key(map, &[A::Delete], 1, "delete (to the trash)"),
        key(map, &[A::MkDir, A::NewFile], 1, "new folder / new file"),
        key(map, &[A::Rename], 1, "rename"),
        key(map, &[A::LastOutput], 1, "output of the last run command"),
        heading("Look"),
        key(
            map,
            &[A::View, A::ViewHex],
            1,
            "view a file / view it as hex",
        ),
        key(
            map,
            &[A::Filter, A::GoTo],
            1,
            "filter the list / go to a path",
        ),
        key(map, &[A::ToggleHidden], 1, "show or hide dotfiles"),
        key(
            map,
            &[A::CycleSort, A::ToggleSortOrder],
            1,
            "sort key / sort order",
        ),
        key(map, &[A::CycleTheme], 1, "next colour theme"),
        heading("App"),
        HelpLine::Key("h".into(), "this help (fixed — cannot be rebound)".into()),
        key(map, &[A::Quit], 1, "quit"),
    ];

    let files = vec![
        heading("Two panes"),
        note(
            "The pane you are not in is the target of copy and move; the prompt is pre-filled with its folder.",
        ),
        note(
            "Mark entries to act on several at once; without marks, actions use the entry under the cursor.",
        ),
        heading("Running programs"),
        note(
            "Enter on an executable shows an editable run line (./name). Edit it, Enter runs it in this folder.",
        ),
        heading("Safety"),
        note(
            "Delete moves to the trash (~/.Trash, or $BIFOLD_TRASH_DIR). Set delete_mode = \"permanent\" to really delete.",
        ),
        note(
            "One transfer runs at a time; Esc in its dialog cancels it. Symlinks are copied as links, never followed.",
        ),
        note("A name clash never overwrites: the copy becomes \"report 2.pdf\"."),
        heading("Finding things"),
        note(
            "Filter takes plain text, or * and ? globs (*.md). Esc clears the filter and the marks.",
        ),
        note(
            "Go-to accepts ~/paths. Sort key and order, and hidden files, are remembered when you quit.",
        ),
    ];

    let viewer = vec![
        heading("Viewing files"),
        note(
            "Text opens as text. Binary files open as a hex dump (offset, 16 bytes, ASCII) instead of being refused.",
        ),
        vkey(
            map,
            &[ViewerAction::ToggleHex],
            2,
            "switch between text and hex",
        ),
        vkey(map, &[ViewerAction::Up, ViewerAction::Down], 1, "scroll"),
        vkey(
            map,
            &[ViewerAction::PageUp, ViewerAction::PageDown],
            1,
            "page",
        ),
        vkey(
            map,
            &[ViewerAction::Top, ViewerAction::Bottom],
            1,
            "start / end",
        ),
        vkey(map, &[ViewerAction::ToggleWrap], 1, "wrap lines (terminal)"),
        vkey(map, &[ViewerAction::Close], 2, "close"),
        heading("Previewer (like lf)"),
        note(
            "Set previewer = \"script\" in config.toml. It runs as: script <file> <width> <height> <x> <y>",
        ),
        note(
            "and its coloured output is shown. If it fails or takes too long, the built-in viewer is used.",
        ),
    ];

    let themes = ctx.themes.join(", ");
    let setup = vec![
        heading("Configuration"),
        note(format!(
            "Config: {}/config.toml (written when you quit).",
            ctx.config_dir
        )),
        note(
            "Rebind keys in [keys] and [viewer_keys]; \"none\" unbinds. List them: bifold --dump-keys.",
        ),
        note("Dialog keys (y/n, Enter, Esc) and the keys on this screen are fixed."),
        heading(&format!("Themes — now: {}", ctx.theme)),
        note(format!("Available: {themes}.")),
        note(format!(
            "Add your own as {}/themes/<name>.toml:",
            ctx.config_dir
        )),
        note("  extends = \"nord\"   accent = \"#ff8800\"   panel_bg = \"reset\""),
        note(
            "Pick with theme = \"name\", --theme NAME, or the theme key; see bifold --list-themes.",
        ),
    ];

    let all = map
        .help_rows()
        .into_iter()
        .map(|(keys, text)| {
            if text.is_empty() && !keys.is_empty() {
                HelpLine::Heading(keys)
            } else if text.is_empty() {
                HelpLine::Note(String::new())
            } else {
                HelpLine::Key(keys, text)
            }
        })
        .collect();

    [keys, files, viewer, setup, all]
        .into_iter()
        .zip(TITLES)
        .map(|(lines, title)| HelpPage { title, lines })
        .collect()
}

/// Wrap-around tab arithmetic shared by both front ends.
pub fn step(tab: usize, delta: isize, count: usize) -> usize {
    if count == 0 {
        return 0;
    }
    (tab as isize + delta).rem_euclid(count as isize) as usize
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    fn ctx() -> HelpContext {
        HelpContext {
            config_dir: "~/.config/bifold".into(),
            theme: "nord".into(),
            themes: vec!["bifold".into(), "nord".into()],
        }
    }

    fn text(page: &HelpPage) -> String {
        page.lines
            .iter()
            .map(|l| match l {
                HelpLine::Heading(t) | HelpLine::Note(t) => t.clone(),
                HelpLine::Key(k, t) => format!("{k} {t}"),
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn has_the_five_titled_tabs() {
        let pages = pages(&Keymap::defaults(), &ctx());
        let titles: Vec<_> = pages.iter().map(|p| p.title).collect();
        assert_eq!(titles, TITLES);
        assert!(pages.iter().all(|p| !p.lines.is_empty()));
    }

    #[test]
    fn the_first_four_tabs_fit_one_screen() {
        for page in pages(&Keymap::defaults(), &ctx()).iter().take(4) {
            assert!(
                page.lines.len() <= 20,
                "{} has {} lines",
                page.title,
                page.lines.len()
            );
        }
    }

    #[test]
    fn key_rows_follow_the_live_keymap() {
        let page = &pages(&Keymap::defaults(), &ctx())[0];
        let body = text(page);
        assert!(
            body.contains("f5 / f6 copy / move to the other pane"),
            "{body}"
        );
        assert!(body.contains("h this help (fixed"), "{body}");
        assert!(body.contains("up / down move the cursor"), "{body}");

        let overrides: BTreeMap<String, String> = [("ctrl+y", "copy"), ("f5", "none")]
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        let (rebound, _) = Keymap::with_overrides(&overrides, &BTreeMap::new());
        let body = text(&pages(&rebound, &ctx())[0]);
        assert!(body.contains("ctrl+y / f6 copy / move"), "{body}");
        assert!(!body.contains("f5"), "{body}");
    }

    #[test]
    fn unbound_actions_say_so_instead_of_showing_nothing() {
        let overrides: BTreeMap<String, String> = [("f7", "none"), ("ctrl+n", "none")]
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        let (map, _) = Keymap::with_overrides(&overrides, &BTreeMap::new());
        assert!(text(&pages(&map, &ctx())[0]).contains("(unbound) new folder / new file"));
    }

    #[test]
    fn notes_cover_the_main_features() {
        let pages = pages(&Keymap::defaults(), &ctx());
        let files = text(&pages[1]);
        for needle in ["trash", "report 2.pdf", "Mark", "glob"] {
            assert!(
                files.to_lowercase().contains(&needle.to_lowercase()),
                "files lacks {needle}"
            );
        }
        let viewer = text(&pages[2]);
        assert!(
            viewer.contains("hex dump") && viewer.contains("previewer"),
            "{viewer}"
        );
        assert!(
            viewer.contains("x / f4 switch between text and hex"),
            "{viewer}"
        );
        let setup = text(&pages[3]);
        assert!(setup.contains("~/.config/bifold/config.toml"), "{setup}");
        assert!(
            setup.contains("now: nord") && setup.contains("bifold, nord"),
            "{setup}"
        );
        assert!(setup.contains("--dump-keys") && setup.contains("themes/<name>.toml"));
    }

    #[test]
    fn the_last_tab_lists_every_bound_action() {
        let all = text(&pages(&Keymap::defaults(), &ctx())[4]);
        for action in Action::ALL {
            assert!(
                all.contains(action.description()),
                "missing {}",
                action.name()
            );
        }
    }

    #[test]
    fn tab_stepping_wraps_both_ways() {
        assert_eq!(step(0, -1, 5), 4);
        assert_eq!(step(4, 1, 5), 0);
        assert_eq!(step(2, 1, 5), 3);
        assert_eq!(step(0, 1, 0), 0);
    }
}
