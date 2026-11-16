use std::fs;
use std::io::Read as _;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

const MAX_CONFIG_BYTES: u64 = 1024 * 1024;
const MAX_FAVORITES_BYTES: u64 = 256 * 1024;
const MAX_FAVORITES: usize = 10_000;

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
        let _ = private_dir(&config_dir());
        let _ = restrict_file(&config_path());
        match read_limited(&config_path(), MAX_CONFIG_BYTES) {
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
        private_dir(&dir)?;
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
    let _ = restrict_file(&favorites_path());
    read_limited(&favorites_path(), MAX_FAVORITES_BYTES)
        .ok()
        .and_then(|text| serde_json::from_str::<Vec<String>>(&text).ok())
        .map(|mut ids| {
            ids.retain(|id| is_safe_id(id));
            ids.truncate(MAX_FAVORITES);
            ids
        })
        .unwrap_or_default()
}

pub fn save_favorites(favorites: &[String]) -> Result<(), String> {
    let dir = config_dir();
    private_dir(&dir)?;
    let favorites: Vec<&String> = favorites
        .iter()
        .filter(|id| is_safe_id(id))
        .take(MAX_FAVORITES)
        .collect();
    let text = serde_json::to_string_pretty(&favorites).map_err(|e| e.to_string())?;
    write_atomic(&favorites_path(), &text)
}

fn is_safe_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && !id.starts_with('-')
        && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

fn read_limited(path: &Path, max: u64) -> std::io::Result<String> {
    let file = fs::File::open(path)?;
    if file.metadata()?.len() > max {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "file is too large",
        ));
    }
    let mut text = String::new();
    file.take(max + 1).read_to_string(&mut text)?;
    if text.len() as u64 > max {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "file is too large",
        ));
    }
    Ok(text)
}

fn restrict_file(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        if path.exists() {
            fs::set_permissions(path, fs::Permissions::from_mode(0o600))
                .map_err(|e| format!("chmod {}: {e}", path.display()))?;
        }
    }
    Ok(())
}

fn private_dir(path: &Path) -> Result<(), String> {
    fs::create_dir_all(path).map_err(|e| format!("create {}: {e}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .map_err(|e| format!("chmod {}: {e}", path.display()))?;
    }
    Ok(())
}

fn write_atomic(path: &Path, contents: &str) -> Result<(), String> {
    let tmp = path.with_extension("tmp");
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    {
        use std::io::Write as _;
        let mut file = options
            .open(&tmp)
            .map_err(|e| format!("write {}: {e}", tmp.display()))?;
        file.write_all(contents.as_bytes())
            .map_err(|e| format!("write {}: {e}", tmp.display()))?;
        file.sync_all()
            .map_err(|e| format!("sync {}: {e}", tmp.display()))?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600))
            .map_err(|e| format!("chmod {}: {e}", tmp.display()))?;
    }
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
    let exe = exe
        .to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('%', "%%");
    let entry = format!(
        "[Desktop Entry]\nType=Application\nName=Vault\nComment=Bitwarden via rbw\nExec=\"{exe}\"\nTerminal=false\nX-GNOME-Autostart-enabled=true\n"
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
