//! Shared app state and short-lived registries (selections, plans, tokens).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::{Mutex, RwLock};
use r2dock_core::AppError;
use r2dock_core::settings::SettingsFile;
use r2dock_core::storage::write_json_atomic;
use r2dock_r2::R2Client;
use r2dock_r2::bulk::MoveItem;
use r2dock_transfer::{BoxFuture, ClientProvider, Engine};
use uuid::Uuid;

use crate::connections::ConnectionService;
use crate::media::MediaRegistry;

/// Tokens and plans expire after this long.
pub const TOKEN_TTL: Duration = Duration::from_secs(15 * 60);
/// Cap on how many pending items of one kind are kept.
const REGISTRY_CAP: usize = 256;

pub struct Paths {
    pub config: PathBuf,
    pub data: PathBuf,
    pub cache: PathBuf,
    pub logs: PathBuf,
    pub home: Option<PathBuf>,
    pub downloads: Option<PathBuf>,
}

/// A map of opaque ids to values that expire.
pub struct Registry<T> {
    items: Mutex<HashMap<String, (Instant, T)>>,
}

impl<T> Default for Registry<T> {
    fn default() -> Self {
        Self {
            items: Mutex::new(HashMap::new()),
        }
    }
}

impl<T> Registry<T> {
    pub fn insert(&self, value: T) -> String {
        let id = Uuid::new_v4().simple().to_string();
        let mut items = self.items.lock();
        let now = Instant::now();
        items.retain(|_, (t, _)| now.duration_since(*t) < TOKEN_TTL);
        if items.len() >= REGISTRY_CAP
            && let Some(oldest) = items
                .iter()
                .min_by_key(|(_, (t, _))| *t)
                .map(|(k, _)| k.clone())
        {
            items.remove(&oldest);
        }
        items.insert(id.clone(), (now, value));
        id
    }

    /// Remove and return a value (single use).
    pub fn take(&self, id: &str) -> Option<T> {
        let (t, v) = self.items.lock().remove(id)?;
        (t.elapsed() < TOKEN_TTL).then_some(v)
    }

    /// Put a taken value back under its id, so a step that failed after
    /// `take` can be tried again.
    pub fn restore(&self, id: &str, value: T) {
        self.items
            .lock()
            .insert(id.to_owned(), (Instant::now(), value));
    }
}

impl<T: Clone> Registry<T> {
    pub fn get(&self, id: &str) -> Option<T> {
        let items = self.items.lock();
        let (t, v) = items.get(id)?;
        (t.elapsed() < TOKEN_TTL).then(|| v.clone())
    }
}

pub struct UploadFile {
    pub local: PathBuf,
    pub key: String,
    pub size: u64,
    pub identity: Option<r2dock_transfer::source::FileIdentity>,
    /// What compression would do, when it is on (set by the estimate).
    pub compression: Option<r2dock_media::estimate::FileDecision>,
}

pub struct PreparedUpload {
    pub connection: Uuid,
    pub bucket: String,
    pub files: Vec<UploadFile>,
    pub markers: Vec<String>,
    pub label: String,
    pub grouped: bool,
}

pub struct PendingDelete {
    pub connection: Uuid,
    pub bucket: String,
    pub keys: Vec<String>,
    pub needs_typed: bool,
    pub typed_phrase: String,
}

pub struct PendingMove {
    pub connection: Uuid,
    pub bucket: String,
    pub items: Vec<MoveItem>,
}

pub struct AppState {
    pub paths: Paths,
    pub settings: RwLock<SettingsFile>,
    pub connections: Arc<ConnectionService>,
    pub engine: Engine,
    pub selections: Registry<Vec<PathBuf>>,
    pub uploads: Registry<PreparedUpload>,
    pub deletes: Registry<PendingDelete>,
    pub moves: Registry<PendingMove>,
    pub media: MediaRegistry,
    pub credential_store_label: String,
    pub activity: Arc<crate::activity::ActivityLog>,
    pub compression: crate::compression::CompressionService,
    pub public_links: crate::public_links::PublicLinks,
}

impl AppState {
    pub fn settings_path(&self) -> PathBuf {
        self.paths.config.join("settings.json")
    }

    /// Change the settings: the change is applied to a copy, the copy is
    /// saved, and only then does it become the live settings. When the save
    /// fails nothing changes. Returns the settings that were replaced.
    pub async fn update_settings(
        &self,
        change: impl FnOnce(&mut SettingsFile),
    ) -> Result<SettingsFile, AppError> {
        // Changes go one at a time, so none is lost and the newest settings
        // are always the last ones on disk.
        static WRITING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
        commit_json(&self.settings, &WRITING, self.settings_path(), change).await
    }

    /// The download folder: chosen in settings, else the system Downloads folder.
    pub fn download_dir(&self) -> Option<PathBuf> {
        self.settings
            .read()
            .download_dir
            .clone()
            .or_else(|| self.paths.downloads.clone())
            .or_else(|| self.paths.home.clone())
    }

    /// The activity log folder: chosen in settings, else `<logs>/activity`.
    pub fn activity_dir(&self) -> PathBuf {
        self.settings
            .read()
            .activity_dir
            .clone()
            .unwrap_or_else(|| self.paths.logs.join("activity"))
    }

    pub fn display_path(&self, path: &std::path::Path) -> String {
        r2dock_core::redact::redact_path(path, self.paths.home.as_deref())
    }
}

/// Apply `change` to a copy of `cell`, write the copy to `path`, then publish
/// it. Holding `lock` for the whole step keeps concurrent changes in order.
async fn commit_json<T>(
    cell: &RwLock<T>,
    lock: &tokio::sync::Mutex<()>,
    path: PathBuf,
    change: impl FnOnce(&mut T),
) -> Result<T, AppError>
where
    T: Clone + serde::Serialize + Send + 'static,
{
    let _writing = lock.lock().await;
    let old = cell.read().clone();
    let mut candidate = old.clone();
    change(&mut candidate);
    let snapshot = candidate.clone();
    tokio::task::spawn_blocking(move || write_json_atomic(&path, &snapshot))
        .await
        .map_err(|_| AppError::internal("A background task stopped unexpectedly."))?
        .map_err(|_| AppError::new(r2dock_core::ErrorCode::Io, "Could not save settings."))?;
    *cell.write() = candidate;
    Ok(old)
}

/// Lets the transfer engine get clients from the connection service.
pub struct EngineClients(pub Arc<ConnectionService>);

impl ClientProvider for EngineClients {
    fn client(&self, connection: Uuid) -> BoxFuture<'_, Result<R2Client, AppError>> {
        Box::pin(self.0.client(connection))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_is_single_use_and_capped() {
        let r: Registry<u32> = Registry::default();
        let id = r.insert(7);
        assert_eq!(r.get(&id), Some(7));
        assert_eq!(r.take(&id), Some(7));
        assert_eq!(r.take(&id), None);
        for i in 0..(REGISTRY_CAP as u32 + 10) {
            r.insert(i);
        }
        assert!(r.items.lock().len() <= REGISTRY_CAP);
        assert_eq!(r.take("unknown"), None);
    }

    #[test]
    fn registry_value_can_be_restored_after_take() {
        let r: Registry<u32> = Registry::default();
        let id = r.insert(7);
        let v = r.take(&id).unwrap();
        r.restore(&id, v);
        assert_eq!(r.take(&id), Some(7));
        assert_eq!(r.take(&id), None);
    }

    #[tokio::test]
    async fn settings_change_is_published_only_after_it_is_saved() {
        let dir = tempfile::tempdir().unwrap();
        let lock = tokio::sync::Mutex::new(());
        let cell = RwLock::new(SettingsFile::default());
        let good = dir.path().join("settings.json");
        let old = commit_json(&cell, &lock, good.clone(), |s| {
            s.download_dir = Some(PathBuf::from("first"));
        })
        .await
        .unwrap();
        assert_eq!(old.download_dir, None);
        assert_eq!(cell.read().download_dir, Some(PathBuf::from("first")));
        let on_disk: SettingsFile = serde_json::from_slice(&std::fs::read(&good).unwrap()).unwrap();
        assert_eq!(on_disk.download_dir, Some(PathBuf::from("first")));

        // A parent that is a regular file makes the save fail.
        let blocker = dir.path().join("not-a-folder");
        std::fs::write(&blocker, b"x").unwrap();
        let bad = blocker.join("settings.json");
        for change in [
            (|s: &mut SettingsFile| s.download_dir = Some(PathBuf::from("second")))
                as fn(&mut SettingsFile),
            |s| s.activity_dir = Some(PathBuf::from("logs")),
            |s| *s = SettingsFile::default(),
            |s| s.preferences.concurrency = 1,
        ] {
            let err = commit_json(&cell, &lock, bad.clone(), change)
                .await
                .unwrap_err();
            assert_eq!(err.code, r2dock_core::ErrorCode::Io);
            assert_eq!(cell.read().download_dir, Some(PathBuf::from("first")));
            assert_eq!(cell.read().activity_dir, None);
            assert_eq!(cell.read().preferences, SettingsFile::default().preferences);
        }
    }
}
