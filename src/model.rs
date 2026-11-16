use std::fmt;

use zeroize::Zeroize;

/// A secret string that is zeroed on drop and never shown by `Debug`.
pub struct Secret(String);

impl Secret {
    pub fn new(value: String) -> Self {
        Self(value)
    }

    pub fn expose(&self) -> &str {
        &self.0
    }

    pub fn masked(&self) -> String {
        "•".repeat(self.0.chars().count().clamp(8, 24))
    }
}

impl Drop for Secret {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(•••)")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemType {
    Login,
    SecureNote,
    Card,
    Identity,
    Other,
}

impl ItemType {
    pub fn from_rbw(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "login" => Self::Login,
            "note" | "secure note" | "securenote" => Self::SecureNote,
            "card" => Self::Card,
            "identity" => Self::Identity,
            _ => Self::Other,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Login => "Login",
            Self::SecureNote => "Secure note",
            Self::Card => "Card",
            Self::Identity => "Identity",
            Self::Other => "Other",
        }
    }
}

/// Non-secret metadata. Safe to keep in memory and to search over.
#[derive(Clone, Debug)]
pub struct ItemMeta {
    pub id: String,
    pub name: String,
    pub username: Option<String>,
    pub uris: Vec<String>,
    pub folder: Option<String>,
    pub item_type: ItemType,
}

impl ItemMeta {
    pub fn domain(&self) -> Option<String> {
        self.uris.first().map(|u| domain_of(u))
    }
}

/// A fully decrypted item. Secrets live only here and are masked by `Debug`.
pub struct ItemDetails {
    pub id: String,
    pub name: String,
    pub username: Option<String>,
    pub password: Option<Secret>,
    pub uris: Vec<String>,
    pub notes: Option<String>,
    pub item_type: ItemType,
    pub has_totp: bool,
}

impl fmt::Debug for ItemDetails {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ItemDetails")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("item_type", &self.item_type)
            .field("has_totp", &self.has_totp)
            .field("username", &self.username.as_deref().map(|_| "•••"))
            .field("password", &self.password.as_ref().map(|_| "•••"))
            .field("notes", &self.notes.as_deref().map(|_| "•••"))
            .finish()
    }
}

/// Extract a searchable domain from a URL without pulling in a URL parser.
pub fn domain_of(uri: &str) -> String {
    let mut rest = uri.trim();
    if let Some((_, tail)) = rest.split_once("://") {
        rest = tail;
    }
    rest = rest.split(['/', '?', '#']).next().unwrap_or(rest);
    rest = rest.rsplit('@').next().unwrap_or(rest);
    let host = rest
        .split(':')
        .next()
        .unwrap_or(rest)
        .trim_start_matches("www.");
    host.to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_debug_and_mask() {
        let secret = Secret::new("hunter2".into());
        assert!(!format!("{secret:?}").contains("hunter2"));
        assert_eq!(secret.masked().chars().count(), 8);
        assert_eq!(secret.expose(), "hunter2");
    }

    #[test]
    fn domains() {
        assert_eq!(
            domain_of("https://www.example.com/login?x=1"),
            "example.com"
        );
        assert_eq!(domain_of("example.com:8443"), "example.com");
        assert_eq!(domain_of("user@example.com/path"), "example.com");
    }

    #[test]
    fn item_types() {
        assert_eq!(ItemType::from_rbw("Login"), ItemType::Login);
        assert_eq!(ItemType::from_rbw("SSH Key"), ItemType::Other);
        assert_eq!(ItemType::from_rbw("Note"), ItemType::SecureNote);
    }
}
