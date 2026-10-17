//! Secret storage for R2 credentials.
//!
//! Secrets live in the OS credential store (macOS Keychain, Windows
//! Credential Manager, Linux Secret Service). When that store is not
//! available, a memory-only store keeps secrets for the current session.
//! There is no plaintext fallback.
//!
//! Every call here may block on the OS, so async callers must use
//! `spawn_blocking`.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

use std::collections::HashMap;

use parking_lot::Mutex;
use secrecy::{ExposeSecret, SecretString};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SecretError {
    #[error("no saved secret was found")]
    NotFound,
    #[error("the system credential store is not available")]
    Unavailable,
    #[error("the system credential store refused the request")]
    Denied,
    #[error("the system credential store returned an error")]
    Other,
}

/// Which backend holds secrets. Shown in Settings > Security.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    MacosKeychain,
    WindowsCredentialManager,
    SecretService,
    SessionMemory,
}

impl Backend {
    pub fn label(self) -> &'static str {
        match self {
            Self::MacosKeychain => "macOS Keychain",
            Self::WindowsCredentialManager => "Windows Credential Manager",
            Self::SecretService => "Secret Service (GNOME Keyring or KWallet)",
            Self::SessionMemory => "Memory for this session only",
        }
    }

    /// The native backend for the current platform.
    pub fn native() -> Self {
        if cfg!(target_os = "macos") {
            Self::MacosKeychain
        } else if cfg!(target_os = "windows") {
            Self::WindowsCredentialManager
        } else {
            Self::SecretService
        }
    }
}

pub trait SecretStore: Send + Sync {
    fn backend(&self) -> Backend;
    fn set(&self, reference: &str, secret: &SecretString) -> Result<(), SecretError>;
    fn get(&self, reference: &str) -> Result<SecretString, SecretError>;
    /// Deleting a missing entry is not an error.
    fn delete(&self, reference: &str) -> Result<(), SecretError>;
}

/// A new random reference for a credential store entry.
pub fn new_reference() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// OS credential store via the `keyring` crate.
pub struct KeyringStore {
    service: String,
}

impl KeyringStore {
    pub fn new(service: impl Into<String>) -> Self {
        Self {
            service: service.into(),
        }
    }

    fn entry(&self, reference: &str) -> Result<keyring::Entry, SecretError> {
        keyring::Entry::new(&self.service, reference).map_err(map_keyring_error)
    }

    /// Check that the store works by writing, reading, and deleting a probe entry.
    pub fn probe(&self) -> bool {
        let reference = format!("probe-{}", new_reference());
        let probe = SecretString::from("r2dock-probe".to_string());
        let ok = self.set(&reference, &probe).is_ok()
            && self
                .get(&reference)
                .is_ok_and(|v| v.expose_secret() == "r2dock-probe");
        let _ = self.delete(&reference);
        ok
    }
}

fn map_keyring_error(err: keyring::Error) -> SecretError {
    match err {
        keyring::Error::NoEntry => SecretError::NotFound,
        keyring::Error::NoStorageAccess(_) | keyring::Error::PlatformFailure(_) => {
            SecretError::Unavailable
        }
        keyring::Error::Ambiguous(_) => SecretError::Other,
        _ => SecretError::Other,
    }
}

impl SecretStore for KeyringStore {
    fn backend(&self) -> Backend {
        Backend::native()
    }

    fn set(&self, reference: &str, secret: &SecretString) -> Result<(), SecretError> {
        self.entry(reference)?
            .set_password(secret.expose_secret())
            .map_err(map_keyring_error)
    }

    fn get(&self, reference: &str) -> Result<SecretString, SecretError> {
        let value = self
            .entry(reference)?
            .get_password()
            .map_err(map_keyring_error)?;
        Ok(SecretString::from(value))
    }

    fn delete(&self, reference: &str) -> Result<(), SecretError> {
        match self.entry(reference)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(err) => Err(map_keyring_error(err)),
        }
    }
}

/// Secrets held only in memory. Values are zeroized when dropped.
#[derive(Default)]
pub struct MemoryStore {
    entries: Mutex<HashMap<String, SecretString>>,
}

impl MemoryStore {
    pub fn new() -> Self {
        Self::default()
    }
}

impl SecretStore for MemoryStore {
    fn backend(&self) -> Backend {
        Backend::SessionMemory
    }

    fn set(&self, reference: &str, secret: &SecretString) -> Result<(), SecretError> {
        self.entries.lock().insert(
            reference.to_string(),
            SecretString::from(secret.expose_secret().to_string()),
        );
        Ok(())
    }

    fn get(&self, reference: &str) -> Result<SecretString, SecretError> {
        self.entries
            .lock()
            .get(reference)
            .map(|s| SecretString::from(s.expose_secret().to_string()))
            .ok_or(SecretError::NotFound)
    }

    fn delete(&self, reference: &str) -> Result<(), SecretError> {
        self.entries.lock().remove(reference);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_store_round_trip() {
        let store = MemoryStore::new();
        let r = new_reference();
        assert_eq!(store.get(&r).unwrap_err(), SecretError::NotFound);
        store
            .set(&r, &SecretString::from("s3cr3t".to_string()))
            .unwrap();
        assert_eq!(store.get(&r).unwrap().expose_secret(), "s3cr3t");
        store.delete(&r).unwrap();
        store.delete(&r).unwrap();
        assert_eq!(store.get(&r).unwrap_err(), SecretError::NotFound);
    }

    #[test]
    fn secret_debug_is_redacted() {
        let s = SecretString::from("topsecretvalue".to_string());
        assert!(!format!("{s:?}").contains("topsecretvalue"));
    }

    #[test]
    fn references_are_unique() {
        assert_ne!(new_reference(), new_reference());
    }

    /// Touches the real OS credential store. Run manually per platform:
    /// `cargo test -p r2dock-secrets -- --ignored`
    #[test]
    #[ignore = "uses the real OS credential store"]
    fn keyring_round_trip() {
        let store = KeyringStore::new("app.r2dock.credentials.test");
        assert!(store.probe(), "credential store should be available");
        let r = new_reference();
        store
            .set(&r, &SecretString::from("abc".to_string()))
            .unwrap();
        assert_eq!(store.get(&r).unwrap().expose_secret(), "abc");
        store.delete(&r).unwrap();
        assert_eq!(store.get(&r).unwrap_err(), SecretError::NotFound);
    }
}
