use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    System,
    Dark,
    Light,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CopyField {
    Username,
    Password,
    Totp,
}

impl CopyField {
    pub fn label(self) -> &'static str {
        match self {
            Self::Username => "Username",
            Self::Password => "Password",
            Self::Totp => "TOTP",
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::Username => Self::Password,
            Self::Password => Self::Totp,
            Self::Totp => Self::Username,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Shortcuts {
    pub focus_search: String,
    pub copy_default: String,
    pub copy_password: String,
    pub copy_username: String,
    pub copy_totp: String,
    pub sync: String,
    pub generator: String,
    pub toggle_favorite: String,
}

impl Default for Shortcuts {
    fn default() -> Self {
        Self {
            focus_search: "ctrl-k".into(),
            copy_default: "ctrl-c".into(),
            copy_password: "ctrl-shift-c".into(),
            copy_username: "ctrl-shift-u".into(),
            copy_totp: "ctrl-shift-t".into(),
            sync: "ctrl-r".into(),
            generator: "ctrl-g".into(),
            toggle_favorite: "ctrl-d".into(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct GeneratorConfig {
    pub length: usize,
    pub uppercase: bool,
    pub lowercase: bool,
    pub numbers: bool,
    pub symbols: bool,
    pub exclude_ambiguous: bool,
    pub passphrase: bool,
    pub words: usize,
    pub separator: String,
}

impl Default for GeneratorConfig {
    fn default() -> Self {
        Self {
            length: 20,
            uppercase: true,
            lowercase: true,
            numbers: true,
            symbols: true,
            exclude_ambiguous: false,
            passphrase: false,
            words: 5,
            separator: "-".into(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub theme: Theme,
    pub clipboard_timeout_secs: u64,
    pub clear_clipboard_on_exit: bool,
    pub start_minimized: bool,
    pub launch_at_login: bool,
    pub default_copy_field: CopyField,
    pub favorites_first: bool,
    pub rbw_path: String,
    pub notifications: bool,
    pub shortcuts: Shortcuts,
    pub generator: GeneratorConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            theme: Theme::System,
            clipboard_timeout_secs: 30,
            clear_clipboard_on_exit: true,
            start_minimized: false,
            launch_at_login: false,
            default_copy_field: CopyField::Password,
            favorites_first: true,
            rbw_path: "rbw".into(),
            notifications: true,
            shortcuts: Shortcuts::default(),
            generator: GeneratorConfig::default(),
        }
    }
}

fn config_home() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| PathBuf::from("."))
}

pub fn config_dir() -> PathBuf {
    config_home().join("vault")
}

pub fn config_path() -> PathBuf {
    config_dir().join("config.toml")
}

fn favorites_path() -> PathBuf {
    config_dir().join("favorites.json")
}

impl Config {
    /// Loads settings, returning defaults plus a human-readable notice on failure.
    /// Vault contents are never read or written here.
    pub fn load() -> (Self, Option<String>) {
        match fs::read_to_string(config_path()) {
            Ok(text) => match toml::from_str::<Self>(&text) {
                Ok(config) => (config, None),
                Err(err) => (
                    Self::default(),
                    Some(format!("Invalid config.toml, using defaults: {err}")),
                ),
            },
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => (Self::default(), None),
            Err(err) => (
                Self::default(),
                Some(format!("Could not read config.toml: {err}")),
            ),
        }
    }

    pub fn save(&self) -> Result<(), String> {
        let dir = config_dir();
        fs::create_dir_all(&dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
        let text = toml::to_string_pretty(self).map_err(|e| e.to_string())?;
        write_atomic(&config_path(), &text)
    }

    /// Executable name used for rbw; keeps the default in one place.
    pub fn script_path(&self) -> &str {
        if self.rbw_path.trim().is_empty() {
            "rbw"
        } else {
            self.rbw_path.trim()
        }
    }
}

pub fn load_favorites() -> Vec<String> {
    fs::read_to_string(favorites_path())
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

pub fn save_favorites(favorites: &[String]) -> Result<(), String> {
    let dir = config_dir();
    fs::create_dir_all(&dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    let text = serde_json::to_string_pretty(favorites).map_err(|e| e.to_string())?;
    write_atomic(&favorites_path(), &text)
}

fn write_atomic(path: &PathBuf, contents: &str) -> Result<(), String> {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, contents).map_err(|e| format!("write {}: {e}", tmp.display()))?;
    fs::rename(&tmp, path).map_err(|e| format!("rename {}: {e}", path.display()))
}

/// Best-effort XDG autostart entry. Never touches vault data.
pub fn set_autostart(enabled: bool) -> Result<(), String> {
    let dir = config_home().join("autostart");
    let path = dir.join("vault.desktop");
    if !enabled {
        return match fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(format!("remove {}: {err}", path.display())),
        };
    }
    fs::create_dir_all(&dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    let exe = std::env::current_exe().map_err(|e| format!("locate executable: {e}"))?;
    let entry = format!(
        "[Desktop Entry]\nType=Application\nName=Vault\nComment=Bitwarden via rbw\nExec=\"{}\"\nTerminal=false\nX-GNOME-Autostart-enabled=true\n",
        exe.display()
    );
    write_atomic(&path, &entry)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_sane() {
        let config = Config::default();
        assert_eq!(config.clipboard_timeout_secs, 30);
        assert_eq!(config.script_path(), "rbw");
        assert!(config.clear_clipboard_on_exit);
        assert_eq!(config.theme, Theme::System);
    }

    #[test]
    fn toml_round_trip() {
        let config = Config {
            clipboard_timeout_secs: 45,
            theme: Theme::Light,
            ..Config::default()
        };
        let text = toml::to_string_pretty(&config).unwrap();
        let parsed: Config = toml::from_str(&text).unwrap();
        assert_eq!(parsed.clipboard_timeout_secs, 45);
        assert_eq!(parsed.theme, Theme::Light);
    }
}
