//! Persistent preferences: `~/.config/bifold/config.toml`.
//!
//! Every field has a default, so a missing or partial file is fine.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::fsops::{self, DeleteMode};
use crate::keys::Keymap;
use crate::sort::{SortDir, SortKey};
use crate::theme;

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
    /// Opened by the "edit" action; empty means `$EDITOR` or `vi`.
    pub editor: String,
    pub show_permissions: bool,
    /// lf-style previewer: `program [args…]` is run as
    /// `program [args…] <file> <width> <height> <x> <y>`; its stdout (ANSI
    /// colours allowed) becomes the viewer content. Empty = built-in viewer.
    pub previewer: String,
    /// Kill the previewer after this many milliseconds.
    pub previewer_timeout_ms: u64,
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
            show_permissions: false,
            previewer: String::new(),
            previewer_timeout_ms: 3000,
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
        match Self::path() {
            Some(path) => Self::load_from(&path).unwrap_or_default(),
            None => Self::default(),
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
}
