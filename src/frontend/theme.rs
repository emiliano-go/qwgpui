//! Aster-style token palette.
//!
//! Values mirror the Aster theme tokens: dark uses Aster Dark, light uses
//! Kestrel Light. Colours are `0xRRGGBBAA` so translucent borders/hover layers
//! behave the same as the CSS token sheets.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Once;

use crate::config::Theme;

#[allow(dead_code)]
pub struct Palette {
    pub light: bool,
    pub bg: u32,
    pub sidebar: u32,
    pub surface: u32,
    pub card: u32,
    pub strong: u32,
    pub menu: u32,
    pub input: u32,
    pub field: u32,
    pub hover: u32,
    pub selected: u32,
    pub border: u32,
    pub border_strong: u32,
    pub message_divider: u32,
    pub text: u32,
    pub text2: u32,
    pub muted: u32,
    pub accent: u32,
    pub accent_button: u32,
    pub accent_hover: u32,
    pub accent_text: u32,
    pub accent_ink: u32,
    pub danger: u32,
    pub warning: u32,
    pub success: u32,
    pub selection: u32,
    pub overlay: u32,
}

pub const DARK: Palette = Palette {
    light: false,
    bg: 0x0b0d10ff,
    sidebar: 0x12151aff,
    surface: 0x1c2128ff,
    card: 0x262c35ff,
    strong: 0x36404dff,
    menu: 0x262c35ff,
    input: 0x101318ff,
    field: 0x232932ff,
    hover: 0xffffff16,
    selected: 0x38424fff,
    border: 0xffffff26,
    border_strong: 0xffffff42,
    message_divider: 0xffffff1a,
    text: 0xf5f1e9ff,
    text2: 0xd9d3c9ff,
    muted: 0xa29b90ff,
    accent: 0xd98e5cff,
    accent_button: 0xd98e5cff,
    accent_hover: 0xe7a97fff,
    accent_text: 0xf0b088ff,
    accent_ink: 0x171717ff,
    danger: 0xe0837bff,
    warning: 0xe3bd73ff,
    success: 0x8cbd90ff,
    selection: 0xd98e5c66,
    overlay: 0x000000a6,
};

pub const LIGHT: Palette = Palette {
    light: true,
    bg: 0xe7ebf1ff,
    sidebar: 0xdde1e8ff,
    surface: 0xffffffff,
    card: 0xffffffff,
    strong: 0xd9dfe8ff,
    menu: 0xffffffff,
    input: 0xf2f4f8ff,
    field: 0xf3f5f9ff,
    hover: 0x00000012,
    selected: 0xc7d2e2ff,
    border: 0x00000026,
    border_strong: 0x00000040,
    message_divider: 0x0000001f,
    text: 0x1c1f2aff,
    text2: 0x343a4aff,
    muted: 0x606879ff,
    accent: 0x4b91b8ff,
    accent_button: 0x1f3a5fff,
    accent_hover: 0x2b4f79ff,
    accent_text: 0x2f6f8fff,
    accent_ink: 0xf7fbffff,
    danger: 0xb04040ff,
    warning: 0x7d7030ff,
    success: 0x356f52ff,
    selection: 0xbfcbdcff,
    overlay: 0x00000066,
};

pub fn palette(theme: Theme, system_dark: bool) -> &'static Palette {
    match theme {
        Theme::Light => &LIGHT,
        Theme::Dark => &DARK,
        Theme::System if system_dark => &DARK,
        Theme::System => &LIGHT,
    }
}

pub fn name(theme: Theme, system_dark: bool) -> &'static str {
    match theme {
        Theme::Light => "Kestrel Light",
        Theme::Dark => "Aster Dark",
        Theme::System if system_dark => "Aster Dark",
        Theme::System => "Kestrel Light",
    }
}

static SYSTEM_DARK: AtomicBool = AtomicBool::new(false);
static WATCHER: Once = Once::new();

/// Starts a background watcher once per process and returns the current
/// preference. `gsettings`/`kreadconfig` run off the UI thread.
pub fn start_watcher() -> bool {
    WATCHER.call_once(|| {
        let initial = detect_system_dark();
        SYSTEM_DARK.store(initial, Ordering::Relaxed);
        std::thread::spawn(|| loop {
            std::thread::sleep(std::time::Duration::from_secs(10));
            SYSTEM_DARK.store(detect_system_dark(), Ordering::Relaxed);
        });
    });
    SYSTEM_DARK.load(Ordering::Relaxed)
}

pub fn system_dark() -> bool {
    SYSTEM_DARK.load(Ordering::Relaxed)
}

/// Mirrors aster-qt's Qt styleHints source: GTK_THEME, GNOME gsettings and KDE
/// config. The XDG desktop portal preference is intentionally not used, so the
/// two clients agree on which theme slot to show.
pub fn detect_system_dark() -> bool {
    if let Ok(value) = std::env::var("GTK_THEME") {
        if let Some(dark) = scheme_is_dark(&value) {
            return dark;
        }
    }
    for args in [
        &["get", "org.gnome.desktop.interface", "gtk-theme"][..],
        &["get", "org.gnome.desktop.interface", "color-scheme"][..],
    ] {
        if let Ok(output) = std::process::Command::new("gsettings")
            .args(args)
            .env_remove("XDG_CONFIG_HOME")
            .output()
        {
            if let Some(dark) = scheme_is_dark(&String::from_utf8_lossy(&output.stdout)) {
                return dark;
            }
        }
    }
    for command in ["kreadconfig6", "kreadconfig5"] {
        if let Ok(output) = std::process::Command::new(command)
            .env_remove("XDG_CONFIG_HOME")
            .args([
                "--file",
                "kdeglobals",
                "--group",
                "General",
                "--key",
                "ColorScheme",
            ])
            .output()
        {
            if let Some(dark) = scheme_is_dark(&String::from_utf8_lossy(&output.stdout)) {
                return dark;
            }
        }
    }
    false
}

fn scheme_is_dark(value: &str) -> Option<bool> {
    let value = value.to_ascii_lowercase();
    if value.contains("prefer-dark") || value.contains("dark") {
        Some(true)
    } else if value.contains("prefer-light") || value.contains("light") {
        Some(false)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_scheme_values() {
        assert_eq!(scheme_is_dark("prefer-dark"), Some(true));
        assert_eq!(scheme_is_dark("Adwaita:dark"), Some(true));
        assert_eq!(scheme_is_dark("prefer-light"), Some(false));
        assert_eq!(scheme_is_dark("Adwaita"), None);
    }
}
