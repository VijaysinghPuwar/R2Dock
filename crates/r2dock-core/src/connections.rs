//! Non-secret connection configuration and input validation.
//!
//! Secrets are never stored here. A connection holds only an opaque
//! reference to its entry in the OS credential store.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use crate::app_error::AppError;

pub const CONNECTIONS_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS, Hash)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum Jurisdiction {
    #[default]
    Default,
    Eu,
    Fedramp,
}

impl Jurisdiction {
    pub fn label(self) -> &'static str {
        match self {
            Self::Default => "Default",
            Self::Eu => "EU",
            Self::Fedramp => "FedRAMP",
        }
    }
}

/// The R2 S3 API endpoint for an account.
pub fn endpoint_url(account_id: &str, jurisdiction: Jurisdiction) -> String {
    match jurisdiction {
        Jurisdiction::Default => format!("https://{account_id}.r2.cloudflarestorage.com"),
        Jurisdiction::Eu => format!("https://{account_id}.eu.r2.cloudflarestorage.com"),
        Jurisdiction::Fedramp => format!("https://{account_id}.fedramp.r2.cloudflarestorage.com"),
    }
}

/// Where a connection's secret lives.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum SecretLocation {
    /// OS credential store entry, keyed by a random id.
    Keychain { reference: String },
    /// Held in memory for this session only (credential store unavailable).
    Session,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionConfig {
    pub id: Uuid,
    pub name: String,
    pub account_id: String,
    pub access_key_id: String,
    pub jurisdiction: Jurisdiction,
    pub secret: SecretLocation,
    /// Buckets entered by the user (needed for bucket-scoped tokens).
    #[serde(default)]
    pub manual_buckets: Vec<String>,
    /// True when the last test showed the key cannot list buckets.
    #[serde(default)]
    pub bucket_scoped: bool,
    /// Public base URL per bucket, used only to build "Copy public URL".
    #[serde(default)]
    pub public_base_urls: BTreeMap<String, String>,
    #[serde(default)]
    pub favorite_buckets: Vec<String>,
    pub created_at: DateTime<Utc>,
    #[serde(default)]
    pub last_tested_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ConnectionsFile {
    pub version: u32,
    pub connections: Vec<ConnectionConfig>,
    pub active: Option<Uuid>,
    /// Credential store references whose secret is no longer used but could
    /// not be deleted yet (for example, the store was unavailable). Deletion
    /// is retried at startup and after each connection change. These are
    /// opaque random ids, never secret values.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub secret_cleanup: Vec<String>,
}

impl Default for ConnectionsFile {
    fn default() -> Self {
        Self {
            version: CONNECTIONS_VERSION,
            connections: Vec::new(),
            active: None,
            secret_cleanup: Vec::new(),
        }
    }
}

impl ConnectionsFile {
    /// Whether a connection still uses this credential store reference.
    pub fn uses_reference(&self, reference: &str) -> bool {
        self.connections.iter().any(
            |c| matches!(&c.secret, SecretLocation::Keychain { reference: r } if r == reference),
        )
    }

    /// Record a reference whose secret must be deleted. A reference that is
    /// still in use is never recorded.
    pub fn add_cleanup(&mut self, reference: String) {
        if !self.uses_reference(&reference) && !self.secret_cleanup.contains(&reference) {
            self.secret_cleanup.push(reference);
        }
    }
}

/// Shorten an identifier for display: `a1b2…9f0e`.
pub fn shorten_id(id: &str) -> String {
    let chars: Vec<char> = id.chars().collect();
    if chars.len() <= 10 {
        return id.to_string();
    }
    let head: String = chars[..4].iter().collect();
    let tail: String = chars[chars.len() - 4..].iter().collect();
    format!("{head}\u{2026}{tail}")
}

/// Validate a connection name (1 to 60 characters after trimming).
pub fn validate_name(name: &str) -> Result<String, AppError> {
    let trimmed = name.trim();
    let count = trimmed.chars().count();
    if count == 0 {
        return Err(AppError::invalid_input("Enter a name for this connection."));
    }
    if count > 60 {
        return Err(AppError::invalid_input(
            "Name must be 60 characters or fewer.",
        ));
    }
    if trimmed.chars().any(char::is_control) {
        return Err(AppError::invalid_input(
            "Name contains characters that cannot be shown.",
        ));
    }
    Ok(trimmed.to_string())
}

/// Validate an account ID. It becomes part of a hostname, so only ASCII
/// letters and digits are accepted.
pub fn validate_account_id(account_id: &str) -> Result<String, AppError> {
    let id = account_id.trim().to_ascii_lowercase();
    if id.is_empty() {
        return Err(AppError::invalid_input("Enter your Cloudflare account ID."));
    }
    if id.len() > 64 || !id.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return Err(AppError::invalid_input(
            "Account ID should contain only letters and numbers. It is shown on your R2 overview page.",
        ));
    }
    Ok(id)
}

/// Account IDs are usually 32 hex characters. A mismatch is only a warning.
pub fn account_id_looks_unusual(account_id: &str) -> bool {
    !(account_id.len() == 32 && account_id.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// Validate an access key ID.
pub fn validate_access_key_id(key: &str) -> Result<String, AppError> {
    let key = key.trim();
    if key.is_empty() {
        return Err(AppError::invalid_input("Enter the Access Key ID."));
    }
    if key.len() > 128 || !key.bytes().all(|b| b.is_ascii_graphic()) {
        return Err(AppError::invalid_input(
            "Access Key ID contains unexpected characters.",
        ));
    }
    Ok(key.to_string())
}

/// Validate a secret access key without returning it.
pub fn validate_secret(secret: &str) -> Result<(), AppError> {
    if secret.trim().is_empty() {
        return Err(AppError::invalid_input("Enter the Secret Access Key."));
    }
    if secret.len() > 256 || !secret.trim().bytes().all(|b| b.is_ascii_graphic()) {
        return Err(AppError::invalid_input(
            "Secret Access Key contains unexpected characters.",
        ));
    }
    Ok(())
}

/// R2 bucket names: 3 to 63 characters, lowercase letters, digits, and hyphens,
/// starting and ending with a letter or digit.
pub fn validate_bucket_name(name: &str) -> Result<String, AppError> {
    let name = name.trim();
    let ok_chars = name
        .bytes()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
    let ok_ends = name
        .bytes()
        .next()
        .zip(name.bytes().last())
        .is_some_and(|(a, z)| a.is_ascii_alphanumeric() && z.is_ascii_alphanumeric());
    if !(3..=63).contains(&name.len()) || !ok_chars || !ok_ends {
        return Err(AppError::invalid_input(
            "Bucket names use 3 to 63 lowercase letters, numbers, and hyphens.",
        ));
    }
    Ok(name.to_string())
}

/// Validate a public base URL such as `https://media.example.com`.
pub fn validate_public_base_url(url: &str) -> Result<String, AppError> {
    let url = url.trim().trim_end_matches('/');
    let rest = url
        .strip_prefix("https://")
        .ok_or_else(|| AppError::invalid_input("Public URL must start with https://"))?;
    let host = rest.split('/').next().unwrap_or("");
    let host_ok = !host.is_empty()
        && host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b':'));
    if !host_ok || url.contains(['?', '#', ' ', '@']) {
        return Err(AppError::invalid_input(
            "Enter a URL like https://media.example.com",
        ));
    }
    Ok(url.to_string())
}

/// Build a public URL for a key from a configured base URL.
pub fn public_url(base: &str, key: &str) -> String {
    format!(
        "{}/{}",
        base.trim_end_matches('/'),
        crate::keys::percent_encode_key(key)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoints() {
        assert_eq!(
            endpoint_url("abc", Jurisdiction::Default),
            "https://abc.r2.cloudflarestorage.com"
        );
        assert_eq!(
            endpoint_url("abc", Jurisdiction::Eu),
            "https://abc.eu.r2.cloudflarestorage.com"
        );
        assert_eq!(
            endpoint_url("abc", Jurisdiction::Fedramp),
            "https://abc.fedramp.r2.cloudflarestorage.com"
        );
    }

    #[test]
    fn cleanup_never_records_a_reference_in_use() {
        let mut file = ConnectionsFile::default();
        file.connections.push(ConnectionConfig {
            id: Uuid::nil(),
            name: "n".into(),
            account_id: "a".into(),
            access_key_id: "k".into(),
            jurisdiction: Jurisdiction::Default,
            secret: SecretLocation::Keychain {
                reference: "live".into(),
            },
            manual_buckets: Vec::new(),
            bucket_scoped: false,
            public_base_urls: Default::default(),
            favorite_buckets: Vec::new(),
            created_at: chrono::Utc::now(),
            last_tested_at: None,
        });
        file.add_cleanup("live".into());
        file.add_cleanup("old".into());
        file.add_cleanup("old".into());
        assert_eq!(file.secret_cleanup, vec!["old".to_string()]);

        // Older files without the field still load, and an empty list is not written.
        let json = serde_json::to_string(&ConnectionsFile::default()).unwrap();
        assert!(!json.contains("secretCleanup"));
        let back: ConnectionsFile =
            serde_json::from_str(r#"{"version":1,"connections":[]}"#).unwrap();
        assert!(back.secret_cleanup.is_empty());
        let json = serde_json::to_string(&file).unwrap();
        let back: ConnectionsFile = serde_json::from_str(&json).unwrap();
        assert_eq!(back.secret_cleanup, vec!["old".to_string()]);
    }

    #[test]
    fn account_id_cannot_inject_host() {
        assert!(validate_account_id("evil.com/x").is_err());
        assert!(validate_account_id("a b").is_err());
        assert!(validate_account_id("").is_err());
        assert_eq!(validate_account_id(" A1B2 ").unwrap(), "a1b2");
        assert!(account_id_looks_unusual("a1b2"));
        assert!(!account_id_looks_unusual(
            "a1b2c3d4e5f60718293a4b5c6d7e9f0e"
        ));
    }

    #[test]
    fn names_and_keys() {
        assert!(validate_name("").is_err());
        assert!(validate_name(&"x".repeat(61)).is_err());
        assert_eq!(validate_name("  Personal R2 ").unwrap(), "Personal R2");
        assert!(validate_access_key_id("has space").is_err());
        assert!(validate_secret("   ").is_err());
        assert!(validate_secret("abc123").is_ok());
    }

    #[test]
    fn buckets() {
        assert!(validate_bucket_name("media-library").is_ok());
        assert!(validate_bucket_name("ab").is_err());
        assert!(validate_bucket_name("Upper").is_err());
        assert!(validate_bucket_name("-bad").is_err());
        assert!(validate_bucket_name("bad-").is_err());
        assert!(validate_bucket_name("has_underscore").is_err());
    }

    #[test]
    fn public_urls() {
        assert_eq!(
            validate_public_base_url("https://media.example.com/").unwrap(),
            "https://media.example.com"
        );
        assert!(validate_public_base_url("http://media.example.com").is_err());
        assert!(validate_public_base_url("https://user@evil.com").is_err());
        assert!(validate_public_base_url("https://x.com/?q=1").is_err());
        assert_eq!(
            public_url("https://media.example.com", "Audiobook G/cover.webp"),
            "https://media.example.com/Audiobook%20G/cover.webp"
        );
    }

    #[test]
    fn shortens_ids() {
        assert_eq!(
            shorten_id("a1b2c3d4e5f60718293a4b5c6d7e9f0e"),
            "a1b2\u{2026}9f0e"
        );
        assert_eq!(shorten_id("short"), "short");
    }
}
