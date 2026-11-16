use std::borrow::Cow;

use gpui::{AssetSource, Result, SharedString};

/// Icons are embedded so the binary stays self-contained.
pub struct Assets;

macro_rules! icons {
    ($($name:literal),* $(,)?) => {
        const ICONS: &[(&str, &[u8])] = &[
            $(($name, include_bytes!(concat!("../assets/icons/", $name)))),*
        ];
    };
}

icons![
    "card.svg",
    "chevron-down.svg",
    "clock.svg",
    "close.svg",
    "copy.svg",
    "dice.svg",
    "eye-off.svg",
    "eye.svg",
    "identity.svg",
    "key.svg",
    "list.svg",
    "lock.svg",
    "minus.svg",
    "note.svg",
    "plus.svg",
    "refresh.svg",
    "search.svg",
    "settings.svg",
    "shield.svg",
    "star-filled.svg",
    "star.svg",
    "unlock.svg",
    "user.svg",
];

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        let name = path.strip_prefix("icons/").unwrap_or(path);
        Ok(ICONS
            .iter()
            .find(|(icon, _)| *icon == name)
            .map(|(_, bytes)| Cow::Borrowed(*bytes)))
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        if path == "icons" || path == "icons/" {
            Ok(ICONS.iter().map(|(name, _)| (*name).into()).collect())
        } else {
            Ok(Vec::new())
        }
    }
}
