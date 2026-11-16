use std::io::ErrorKind;
use std::process::{Output, Stdio};
use std::time::Duration;

use async_trait::async_trait;
use serde::Deserialize;
use tokio::process::Command;

use super::{
    Result, VaultError, VaultProvider, DETAIL_TIMEOUT, LIST_TIMEOUT, SYNC_TIMEOUT, UNLOCK_TIMEOUT,
};
use crate::model::{ItemDetails, ItemMeta, ItemType, Secret};

/// Talks to `rbw` as an external process. We deliberately do not link rbw's
/// internals: its agent protocol is private and may change.
#[derive(Clone)]
pub struct RbwProvider {
    path: String,
}

impl RbwProvider {
    pub fn new(path: String) -> Self {
        let path = if path.trim().is_empty() {
            "rbw".to_string()
        } else {
            path
        };
        Self { path }
    }

    fn command(&self) -> Command {
        let mut command = Command::new(&self.path);
        command.stdin(Stdio::null()).kill_on_drop(true);
        command
    }

    async fn run(&self, args: &[&str], timeout: Duration, action: &str) -> Result<Output> {
        let mut command = self.command();
        command.args(args);
        match tokio::time::timeout(timeout, command.output()).await {
            Err(_) => Err(VaultError::Timeout(action.to_string())),
            Ok(Err(err)) if err.kind() == ErrorKind::NotFound => Err(VaultError::NotInstalled),
            Ok(Err(err)) => Err(VaultError::CommandFailed(format!(
                "could not run rbw: {err}"
            ))),
            Ok(Ok(output)) => Ok(output),
        }
    }

    async fn run_ok(&self, args: &[&str], timeout: Duration, action: &str) -> Result<String> {
        let output = self.run(args, timeout, action).await?;
        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
        } else {
            Err(classify(&String::from_utf8_lossy(&output.stderr)))
        }
    }
}

#[async_trait]
impl VaultProvider for RbwProvider {
    async fn is_installed(&self) -> Result<()> {
        match self
            .run(&["--version"], Duration::from_secs(10), "checking rbw")
            .await
        {
            Ok(_) => Ok(()),
            Err(VaultError::NotInstalled) => Err(VaultError::NotInstalled),
            Err(err) => Err(err),
        }
    }

    async fn is_unlocked(&self) -> Result<bool> {
        let output = self
            .run(
                &["unlocked"],
                Duration::from_secs(10),
                "checking lock state",
            )
            .await?;
        if output.status.success() {
            return Ok(true);
        }
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        if stderr.to_ascii_lowercase().contains("config")
            || stderr.to_ascii_lowercase().contains("email")
        {
            return Err(VaultError::NotInitialized);
        }
        Ok(false)
    }

    async fn list_items(&self) -> Result<Vec<ItemMeta>> {
        let stdout = self
            .run_ok(&["list", "--raw"], LIST_TIMEOUT, "listing entries")
            .await?;
        parse_list_json(&stdout)
    }

    async fn get_item(&self, id: &str) -> Result<ItemDetails> {
        let stdout = self
            .run_ok(&["get", id, "--raw"], DETAIL_TIMEOUT, "loading entry")
            .await?;
        parse_item_json(&stdout)
    }

    async fn get_totp(&self, id: &str) -> Result<String> {
        let code = self
            .run_ok(&["code", id], DETAIL_TIMEOUT, "generating TOTP")
            .await?;
        if code.is_empty() {
            Err(VaultError::NotFound(
                "this entry has no TOTP configured".into(),
            ))
        } else {
            Ok(code)
        }
    }

    async fn lock(&self) -> Result<()> {
        self.run_ok(&["lock"], Duration::from_secs(30), "locking")
            .await
            .map(|_| ())
    }

    async fn unlock(&self) -> Result<()> {
        self.run_ok(&["unlock"], UNLOCK_TIMEOUT, "unlocking")
            .await
            .map(|_| ())
    }

    async fn sync(&self) -> Result<()> {
        self.run_ok(&["sync"], SYNC_TIMEOUT, "syncing")
            .await
            .map(|_| ())
    }
}

#[derive(Deserialize)]
struct RawListEntry {
    id: String,
    name: Option<String>,
    user: Option<String>,
    folder: Option<String>,
    #[serde(default)]
    uris: Option<Vec<String>>,
    #[serde(rename = "type")]
    entry_type: Option<String>,
}

pub(crate) fn parse_list_json(text: &str) -> Result<Vec<ItemMeta>> {
    let entries: Vec<RawListEntry> =
        serde_json::from_str(text).map_err(|err| VaultError::InvalidJson(err.to_string()))?;
    Ok(entries
        .into_iter()
        .map(|entry| ItemMeta {
            id: entry.id,
            name: entry.name.unwrap_or_else(|| "(unnamed)".into()),
            username: entry.user,
            uris: entry.uris.unwrap_or_default(),
            folder: entry.folder,
            item_type: entry
                .entry_type
                .as_deref()
                .map(ItemType::from_rbw)
                .unwrap_or(ItemType::Other),
        })
        .collect())
}

#[derive(Deserialize)]
struct RawItem {
    id: Option<String>,
    name: Option<String>,
    notes: Option<String>,
    data: serde_json::Value,
    #[serde(rename = "type")]
    entry_type: Option<String>,
}

pub(crate) fn parse_item_json(text: &str) -> Result<ItemDetails> {
    let raw: RawItem =
        serde_json::from_str(text).map_err(|err| VaultError::InvalidJson(err.to_string()))?;
    let data = raw.data;

    let username = string_field(&data, "username");
    let password = string_field(&data, "password").map(Secret::new);
    let uris = data
        .get("uris")
        .and_then(|value| value.as_array())
        .map(|entries| {
            entries
                .iter()
                .filter_map(|entry| entry.get("uri").and_then(|uri| uri.as_str()))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    // Presence only. The TOTP secret itself is never retained; codes come from `rbw code`.
    let has_totp = data
        .get("totp")
        .and_then(|value| value.as_str())
        .is_some_and(|value| !value.is_empty());

    let item_type = raw
        .entry_type
        .as_deref()
        .map(ItemType::from_rbw)
        .unwrap_or_else(|| infer_type(&data));

    Ok(ItemDetails {
        id: raw.id.unwrap_or_default(),
        name: raw.name.unwrap_or_else(|| "(unnamed)".into()),
        username,
        password,
        uris,
        notes: raw.notes.filter(|n| !n.trim().is_empty()),
        item_type,
        has_totp,
    })
}

fn string_field(data: &serde_json::Value, key: &str) -> Option<String> {
    data.get(key)
        .and_then(|value| value.as_str())
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn infer_type(data: &serde_json::Value) -> ItemType {
    if data.is_null() {
        return ItemType::SecureNote;
    }
    let Some(map) = data.as_object() else {
        return ItemType::Other;
    };
    if map.contains_key("number") || map.contains_key("cardholder_name") {
        ItemType::Card
    } else if map.contains_key("first_name")
        || map.contains_key("middle_name")
        || map.contains_key("last_name")
        || map.contains_key("ssn")
        || map.contains_key("passport_number")
    {
        ItemType::Identity
    } else if map.contains_key("public_key") || map.contains_key("private_key") {
        ItemType::Other
    } else {
        ItemType::Login
    }
}

fn classify(stderr: &str) -> VaultError {
    let line = stderr
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("rbw command failed");
    let lower = line.to_ascii_lowercase();
    if lower.contains("couldn't find entry")
        || lower.contains("could not find")
        || lower.contains("no entry")
    {
        VaultError::NotFound(line.to_string())
    } else if lower.contains("locked") {
        VaultError::Locked
    } else if lower.contains("config") || lower.contains("email") || lower.contains("not logged in")
    {
        VaultError::NotInitialized
    } else if lower.contains("authentication") || lower.contains("invalid master password") {
        VaultError::AuthFailed
    } else {
        VaultError::CommandFailed(truncate(line, 240))
    }
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_string()
    } else {
        text.chars().take(max).collect::<String>() + "…"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_list() {
        let json = r#"[
            {"id":"a","name":"GitHub","user":"octocat","folder":null,
             "uris":["https://github.com"],"type":"Login"},
            {"id":"b","name":null,"user":null,"folder":"Work",
             "uris":null,"type":"Note"}
        ]"#;
        let items = parse_list_json(json).unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].name, "GitHub");
        assert_eq!(items[0].item_type, ItemType::Login);
        assert_eq!(items[1].name, "(unnamed)");
        assert_eq!(items[1].item_type, ItemType::SecureNote);
    }

    #[test]
    fn parses_login_without_leaking_secrets_in_debug() {
        let json = r#"{
            "id":"a","name":"GitHub","notes":"hello",
            "data":{"username":"octocat","password":"hunter2","totp":"JBSWY3DPEHPK3PXP",
                    "uris":[{"uri":"https://github.com"}]}
        }"#;
        let item = parse_item_json(json).unwrap();
        assert_eq!(item.username.as_deref(), Some("octocat"));
        assert_eq!(item.password.as_ref().unwrap().expose(), "hunter2");
        assert!(item.has_totp);
        assert_eq!(item.uris, vec!["https://github.com"]);
        let debug = format!("{item:?}");
        assert!(!debug.contains("hunter2"));
        assert!(!debug.contains("octocat"));
    }

    #[test]
    fn parses_secure_note() {
        let item = parse_item_json(r#"{"name":"Note","notes":"body","data":null}"#).unwrap();
        assert_eq!(item.item_type, ItemType::SecureNote);
        assert!(item.password.is_none());
    }

    #[test]
    fn classifies_errors() {
        assert!(matches!(
            classify("error: could not find entry for 'x'"),
            VaultError::NotFound(_)
        ));
        assert!(matches!(
            classify("error: vault is locked"),
            VaultError::Locked
        ));
        assert!(matches!(
            classify("error: no account configured"),
            VaultError::NotInitialized
        ));
    }

    #[test]
    fn rejects_bad_json() {
        assert!(matches!(
            parse_list_json("not json"),
            Err(VaultError::InvalidJson(_))
        ));
    }
}
