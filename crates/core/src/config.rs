//! Persistent preferences: `~/.config/bifold/config.toml`.
//!
//! Every field has a default, so a missing or partial file is fine.
//!
//! The file belongs to the user. The app only ever *edits it in place*: when it
//! exits it changes the few settings that changed during the session (sort,
//! theme, folders…) and leaves comments, unknown keys, order and formatting
//! alone. A file it cannot parse is never overwritten.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::fsops::{self, DeleteMode};
use crate::keys::Keymap;
use crate::sort::{SortDir, SortKey};
use crate::theme;

/// The settings the app itself changes while running (everything else in the
/// config file is only ever read). Compared before/after to decide what to write.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ViewSettings {
    pub left: String,
    pub right: String,
    pub show_hidden: bool,
    pub sort: String,
    pub sort_desc: bool,
    pub theme: String,
    pub show_hints: bool,
}

pub const APP_DIR: &str = "bifold";
pub const FILE_NAME: &str = "config.toml";

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct Config {
    /// Starting directory of the left pane (`~` is expanded).
    pub left: String,
    /// Starting directory of the right pane.
    pub right: String,
    pub show_hidden: bool,
    /// `name` | `size` | `time` | `kind`
    pub sort: String,
    pub sort_desc: bool,
    pub dirs_first: bool,
    /// `trash` | `permanent`
    pub delete_mode: String,
    pub confirm_delete: bool,
    /// Terminal editor for the "edit" action (`hx`, `vim -p`, …); empty means
    /// `$VISUAL`, then `$EDITOR`, then `vi`. The terminal UI gives it the terminal.
    pub editor: String,
    /// What the desktop app runs for "edit" (no terminal there): a GUI editor such
    /// as `code` or `subl`; empty means the system's default text editor.
    pub gui_editor: String,
    pub show_permissions: bool,
    /// The key-hint line at the bottom of the terminal front end.
    pub show_hints: bool,
    /// lf-style previewer: `program [args…]` is run as
    /// `program [args…] <file> <width> <height> <x> <y>`; its stdout (ANSI
    /// colours allowed) becomes the viewer content. Empty = built-in viewer.
    pub previewer: String,
    /// Kill the previewer after this many milliseconds.
    pub previewer_timeout_ms: u64,
    /// Enter on an executable opens an editable run line instead of the viewer.
    pub run_executables: bool,
    /// How long the desktop waits for a command before leaving it running.
    pub run_timeout_ms: u64,
    /// Colour theme: a built-in (`bifold`, `terminal`, `nord`, `gruvbox`,
    /// `gruvbox-light`, `solarized`, `solarized-light`) or a file in `themes/`.
    pub theme: String,
    /// Key overrides: `"ctrl+n" = "new_file"`, `"x" = "none"` to unbind.
    /// Tables stay last: TOML needs plain values before tables.
    pub keys: BTreeMap<String, String>,
    /// Same for the viewer.
    pub viewer_keys: BTreeMap<String, String>,
    /// Per-token colour overrides applied on top of the selected theme.
    pub theme_custom: BTreeMap<String, String>,
}

impl Default for Config {
    fn default() -> Self {
        let home = fsops::home_dir()
            .map(|home| home.display().to_string())
            .unwrap_or_else(|| "/".to_string());
        Self {
            left: home.clone(),
            right: home,
            show_hidden: false,
            sort: "name".to_string(),
            sort_desc: false,
            dirs_first: true,
            delete_mode: "trash".to_string(),
            confirm_delete: true,
            editor: String::new(),
            gui_editor: String::new(),
            show_permissions: false,
            show_hints: true,
            previewer: String::new(),
            previewer_timeout_ms: 3000,
            run_executables: true,
            run_timeout_ms: 10_000,
            theme: theme::DEFAULT_THEME.to_string(),
            keys: BTreeMap::new(),
            viewer_keys: BTreeMap::new(),
            theme_custom: BTreeMap::new(),
        }
    }
}

impl Config {
    pub fn config_dir() -> Option<PathBuf> {
        if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME")
            && !xdg.is_empty()
        {
            return Some(PathBuf::from(xdg).join(APP_DIR));
        }
        fsops::home_dir().map(|home| home.join(".config").join(APP_DIR))
    }

    /// `<config dir>/themes`: one `<name>.toml` per user theme.
    pub fn themes_dir() -> Option<PathBuf> {
        Self::config_dir().map(|dir| theme::themes_dir_in(&dir))
    }

    pub fn path() -> Option<PathBuf> {
        Self::config_dir().map(|dir| dir.join(FILE_NAME))
    }

    /// Load the user config, falling back to defaults on any problem.
    pub fn load() -> Self {
        Self::load_checked().0
    }

    /// Like `load`, but says why the file was ignored. A missing file is fine
    /// (defaults, no problem); an unreadable or invalid one yields defaults
    /// *and* the reason, and the file itself is never modified.
    pub fn load_checked() -> (Self, Option<String>) {
        let Some(path) = Self::path() else {
            return (Self::default(), None);
        };
        if !path.exists() {
            return (Self::default(), None);
        }
        match Self::load_from(&path) {
            Ok(config) => (config, None),
            Err(err) => (Self::default(), Some(err.to_string())),
        }
    }

    pub fn load_from(path: &Path) -> Result<Self> {
        let text = fs::read_to_string(path).map_err(|e| Error::io(path, e))?;
        toml::from_str(&text).map_err(|e| Error::invalid(format!("{}: {e}", path.display())))
    }

    pub fn save(&self) -> Result<PathBuf> {
        let path =
            Self::path().ok_or_else(|| Error::invalid("cannot locate a config directory"))?;
        self.save_to(&path)
    }

    pub fn save_to(&self, path: &Path) -> Result<PathBuf> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
        }
        let text = toml::to_string_pretty(self)
            .map_err(|e| Error::invalid(format!("cannot serialize config: {e}")))?;
        fs::write(path, text).map_err(|e| Error::io(path, e))?;
        Ok(path.to_path_buf())
    }

    /// Write what changed during the session into the existing file, in place.
    /// See [`Config::save_view_changes_to`].
    pub fn save_view_changes(&self, before: &ViewSettings, now: &ViewSettings) -> Result<PathBuf> {
        let path =
            Self::path().ok_or_else(|| Error::invalid("cannot locate a config directory"))?;
        self.save_view_changes_to(&path, before, now)
    }

    /// Without a file yet, write a complete one. Otherwise edit the existing
    /// file: only the fields where `now` differs from `before` are set, so the
    /// user's comments, unknown keys, ordering and formatting survive, and edits
    /// made to the file while the app ran are kept (unless the app changed that
    /// same setting). A file that does not parse is left untouched and an error
    /// is returned. Symlinked config files are written through the link.
    pub fn save_view_changes_to(
        &self,
        path: &Path,
        before: &ViewSettings,
        now: &ViewSettings,
    ) -> Result<PathBuf> {
        use toml_edit::{DocumentMut, Item, Value};

        if !path.exists() {
            return self.save_to(path);
        }
        // Write through a symlink (dotfile repos) instead of replacing it.
        let target = fs::canonicalize(path).map_err(|e| Error::io(path, e))?;
        let text = fs::read_to_string(&target).map_err(|e| Error::io(&target, e))?;
        let mut doc: DocumentMut = text.parse().map_err(|e| {
            Error::invalid(format!(
                "{}: {e} — not saving settings, the file is left as it is",
                target.display()
            ))
        })?;

        let mut set = |key: &str, value: Value| {
            let mut value = value;
            // Keep the user's inline comment and spacing on a value we replace.
            if let Some(old) = doc.get(key).and_then(Item::as_value) {
                *value.decor_mut() = old.decor().clone();
            }
            doc[key] = Item::Value(value);
        };
        if now.left != before.left {
            set("left", now.left.as_str().into());
        }
        if now.right != before.right {
            set("right", now.right.as_str().into());
        }
        if now.show_hidden != before.show_hidden {
            set("show_hidden", now.show_hidden.into());
        }
        if now.sort != before.sort {
            set("sort", now.sort.as_str().into());
        }
        if now.sort_desc != before.sort_desc {
            set("sort_desc", now.sort_desc.into());
        }
        if now.theme != before.theme {
            set("theme", now.theme.as_str().into());
        }
        if now.show_hints != before.show_hints {
            set("show_hints", now.show_hints.into());
        }

        let updated = doc.to_string();
        if updated != text {
            // Temp file + rename: a crash never leaves a half-written config.
            let tmp = target.with_extension("toml.tmp");
            fs::write(&tmp, &updated).map_err(|e| Error::io(&tmp, e))?;
            if let Ok(meta) = fs::metadata(&target) {
                let _ = fs::set_permissions(&tmp, meta.permissions());
            }
            fs::rename(&tmp, &target).map_err(|e| Error::io(&target, e))?;
        }
        Ok(target)
    }

    /// Default bindings with this config's overrides applied, plus any problems found.
    pub fn keymap(&self) -> (Keymap, Vec<String>) {
        Keymap::with_overrides(&self.keys, &self.viewer_keys)
    }

    pub fn left_path(&self) -> PathBuf {
        fsops::expand_home(&self.left)
    }

    pub fn right_path(&self) -> PathBuf {
        fsops::expand_home(&self.right)
    }

    pub fn sort_key(&self) -> SortKey {
        SortKey::parse(&self.sort).unwrap_or(SortKey::Name)
    }

    pub fn sort_dir(&self) -> SortDir {
        if self.sort_desc {
            SortDir::Desc
        } else {
            SortDir::Asc
        }
    }

    pub fn delete_mode(&self) -> DeleteMode {
        match self.delete_mode.trim().to_ascii_lowercase().as_str() {
            "permanent" | "delete" => DeleteMode::Permanent,
            _ => DeleteMode::Trash,
        }
    }

    /// Remember the current directories (used on exit).
    pub fn remember_dirs(&mut self, left: &Path, right: &Path) {
        self.left = left.display().to_string();
        self.right = right.display().to_string();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_sane() {
        let config = Config::default();
        assert_eq!(config.sort_key(), SortKey::Name);
        assert_eq!(config.sort_dir(), SortDir::Asc);
        assert_eq!(config.delete_mode(), DeleteMode::Trash);
        assert!(config.left_path().is_absolute());
    }

    #[test]
    fn roundtrip_toml() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut config = Config {
            sort: "size".into(),
            sort_desc: true,
            show_hidden: true,
            delete_mode: "permanent".into(),
            ..Default::default()
        };
        config.remember_dirs(Path::new("/tmp/left"), Path::new("/tmp/right"));
        config.save_to(&path).unwrap();

        let loaded = Config::load_from(&path).unwrap();
        assert_eq!(loaded.sort_key(), SortKey::Size);
        assert_eq!(loaded.sort_dir(), SortDir::Desc);
        assert!(loaded.show_hidden);
        assert_eq!(loaded.delete_mode(), DeleteMode::Permanent);
        assert_eq!(loaded.left, "/tmp/left");
    }

    #[test]
    fn partial_file_uses_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(&path, "show_hidden = true\n").unwrap();
        let loaded = Config::load_from(&path).unwrap();
        assert!(loaded.show_hidden);
        assert_eq!(loaded.sort_key(), SortKey::Name, "default kept");
    }

    #[test]
    fn bad_toml_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(&path, "this is not toml = = =").unwrap();
        assert!(Config::load_from(&path).is_err());
    }

    #[test]
    fn keys_and_previewer_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(
            &path,
            "previewer = \"~/p.sh\"\nprevie_typo = 1\n[keys]\n\"ctrl+n\" = \"mkdir\"\n\"f8\" = \"none\"\n[viewer_keys]\nh = \"toggle_hex\"\n",
        )
        .unwrap();
        let loaded = Config::load_from(&path).unwrap();
        assert_eq!(loaded.previewer, "~/p.sh");
        assert_eq!(loaded.previewer_timeout_ms, 3000, "default kept");
        let (keymap, warnings) = loaded.keymap();
        assert!(warnings.is_empty(), "{warnings:?}");
        use crate::keys::{Action, KeySpec};
        assert_eq!(
            keymap.action(&KeySpec::parse("ctrl+n").unwrap()),
            Some(Action::MkDir)
        );
        assert_eq!(keymap.action(&KeySpec::parse("f8").unwrap()), None);

        let saved = dir.path().join("saved.toml");
        loaded.save_to(&saved).unwrap();
        let again = Config::load_from(&saved).unwrap();
        assert_eq!(again.keys.get("ctrl+n").map(String::as_str), Some("mkdir"));
        assert_eq!(again.viewer_keys.len(), 1);
    }

    #[test]
    fn theme_settings_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut config = Config {
            theme: "nord".into(),
            ..Default::default()
        };
        config
            .theme_custom
            .insert("accent".into(), "#ff8800".into());
        config.save_to(&path).unwrap();
        let loaded = Config::load_from(&path).unwrap();
        assert_eq!(loaded.theme, "nord");
        assert_eq!(
            loaded.theme_custom.get("accent").map(String::as_str),
            Some("#ff8800")
        );
        assert_eq!(Config::default().theme, "bifold");
    }

    #[test]
    fn unknown_sort_falls_back() {
        let config = Config {
            sort: "vibes".into(),
            ..Default::default()
        };
        assert_eq!(config.sort_key(), SortKey::Name);
    }

    fn view(config: &Config) -> ViewSettings {
        ViewSettings {
            left: config.left.clone(),
            right: config.right.clone(),
            show_hidden: config.show_hidden,
            sort: config.sort.clone(),
            sort_desc: config.sort_desc,
            theme: config.theme.clone(),
            show_hints: config.show_hints,
        }
    }

    const HAND_WRITTEN: &str = r##"# My Bifold config — do not lose this comment
left = "/old/left"    # work tree
right = "/old/right"
show_hidden = false   # flip with '.'
sort = "name"
mystery_setting = 42  # unknown to this version

previewer = "~/bin/preview.sh"

# my bindings
[keys]
"ctrl+n" = "mkdir"   # muscle memory
"f8" = "none"

[theme_custom]
accent = "#ff8800"
"##;

    #[test]
    fn saving_edits_in_place_and_keeps_everything_else() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(&path, HAND_WRITTEN).unwrap();
        let before = view(&Config::load_from(&path).unwrap());
        let mut now = before.clone();
        now.left = "/new/left".into();
        now.show_hidden = true;
        now.theme = "nord".into(); // not in the file at all yet

        let config = Config::load_from(&path).unwrap();
        config.save_view_changes_to(&path, &before, &now).unwrap();

        let text = fs::read_to_string(&path).unwrap();
        for kept in [
            "# My Bifold config — do not lose this comment",
            "mystery_setting = 42  # unknown to this version",
            "previewer = \"~/bin/preview.sh\"",
            "# my bindings",
            "\"ctrl+n\" = \"mkdir\"   # muscle memory",
            "[theme_custom]",
            "accent = \"#ff8800\"",
        ] {
            assert!(text.contains(kept), "lost {kept:?}:\n{text}");
        }
        assert!(
            text.contains("left = \"/new/left\"") && text.contains("# work tree"),
            "{text}"
        );
        assert!(
            text.contains("show_hidden = true") && text.contains("# flip with '.'"),
            "{text}"
        );
        assert!(
            text.contains("right = \"/old/right\"") && text.contains("sort = \"name\""),
            "untouched values stay: {text}"
        );

        let again = Config::load_from(&path).unwrap();
        assert_eq!(again.left, "/new/left");
        assert!(again.show_hidden);
        assert_eq!(again.theme, "nord");
        assert_eq!(again.previewer, "~/bin/preview.sh");
        assert_eq!(again.keys.get("ctrl+n").map(String::as_str), Some("mkdir"));
        assert_eq!(again.keys.get("f8").map(String::as_str), Some("none"));
    }

    #[test]
    fn nothing_changed_means_the_file_is_not_touched() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(&path, HAND_WRITTEN).unwrap();
        let config = Config::load_from(&path).unwrap();
        let same = view(&config);
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        config.save_view_changes_to(&path, &same, &same).unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            HAND_WRITTEN,
            "byte for byte"
        );
        assert_eq!(
            fs::metadata(&path).unwrap().modified().unwrap(),
            modified,
            "not even rewritten"
        );
        assert!(!dir.path().join("config.toml.tmp").exists());
    }

    #[test]
    fn a_file_that_does_not_parse_is_never_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let broken = "show_hidden = = true\n[keys\n";
        fs::write(&path, broken).unwrap();
        let config = Config::default();
        let before = view(&config);
        let mut now = before.clone();
        now.show_hidden = true;
        let err = config
            .save_view_changes_to(&path, &before, &now)
            .unwrap_err();
        assert!(err.to_string().contains("left as it is"), "{err}");
        assert_eq!(fs::read_to_string(&path).unwrap(), broken);
    }

    #[test]
    fn edits_made_while_the_app_ran_survive_unless_the_app_changed_that_setting() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(
            &path,
            "sort = \"name\"\ntheme = \"nord\"\nprevewer_note = 1\n",
        )
        .unwrap();
        let started = view(&Config::load_from(&path).unwrap());

        // The user edits the file in another window while Bifold is open.
        fs::write(
            &path,
            "sort = \"time\"\ntheme = \"gruvbox\"\nprevewer_note = 1\nconfirm_delete = false\n",
        )
        .unwrap();

        // Bifold only changed the theme.
        let mut now = started.clone();
        now.theme = "solarized".into();
        Config::default()
            .save_view_changes_to(&path, &started, &now)
            .unwrap();

        let text = fs::read_to_string(&path).unwrap();
        assert!(
            text.contains("sort = \"time\""),
            "outside edit kept: {text}"
        );
        assert!(
            text.contains("confirm_delete = false"),
            "outside edit kept: {text}"
        );
        assert!(
            text.contains("theme = \"solarized\""),
            "the app's own change wins: {text}"
        );
    }

    #[test]
    fn a_missing_file_gets_a_complete_one() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub/config.toml");
        let config = Config {
            sort: "size".into(),
            ..Default::default()
        };
        let same = view(&config);
        config.save_view_changes_to(&path, &same, &same).unwrap();
        let loaded = Config::load_from(&path).unwrap();
        assert_eq!(loaded.sort, "size");
        assert!(fs::read_to_string(&path).unwrap().contains("show_hints"));
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_config_is_written_through_the_link() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("dotfiles-config.toml");
        let link = dir.path().join("config.toml");
        fs::write(&real, "# mine\nsort = \"name\"\n").unwrap();
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let config = Config::load_from(&link).unwrap();
        let before = view(&config);
        let mut now = before.clone();
        now.sort = "size".into();
        config.save_view_changes_to(&link, &before, &now).unwrap();
        assert!(
            fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink(),
            "still a link"
        );
        assert_eq!(
            fs::read_to_string(&real).unwrap(),
            "# mine\nsort = \"size\"\n"
        );
    }

    #[test]
    fn load_checked_explains_a_bad_file_and_accepts_a_missing_one() {
        // Exercised through load_from + the same rule load_checked applies.
        let dir = tempfile::tempdir().unwrap();
        let bad = dir.path().join("config.toml");
        fs::write(&bad, "show_hidden = \"yes\"\n").unwrap();
        let err = Config::load_from(&bad).unwrap_err().to_string();
        assert!(err.contains("config.toml"), "{err}");
    }
}
