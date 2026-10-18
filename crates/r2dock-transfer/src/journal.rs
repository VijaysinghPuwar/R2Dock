//! On-disk journal of resumable work (multipart uploads and partial
//! downloads) so it can continue after a restart (PLAN.md 13).
//!
//! Updates are batched and written by a background task with atomic file
//! replacement, so the async runtime never blocks on disk writes.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use r2dock_core::storage::{Loaded, read_json, write_json_atomic};
use serde::{Deserialize, Serialize};
use tokio::sync::Notify;

use crate::ctx::ResumeState;
use crate::model::{GroupInfo, JobId, JobSpec};

pub const JOURNAL_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JournalEntry {
    pub id: JobId,
    pub spec: JobSpec,
    pub group: Option<GroupInfo>,
    pub resume: ResumeState,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct JournalFile {
    version: u32,
    entries: Vec<JournalEntry>,
}

impl Default for JournalFile {
    fn default() -> Self {
        Self {
            version: JOURNAL_VERSION,
            entries: Vec::new(),
        }
    }
}

pub struct Journal {
    path: PathBuf,
    entries: Mutex<BTreeMap<JobId, JournalEntry>>,
    dirty: Notify,
    /// One write at a time, so an older snapshot never lands after a newer one.
    writing: Mutex<()>,
}

impl Journal {
    /// Load the journal (a corrupt file is backed up and ignored).
    pub fn load(path: PathBuf) -> (Arc<Self>, Vec<JournalEntry>) {
        let loaded: Vec<JournalEntry> = match read_json::<JournalFile>(&path) {
            Ok(Loaded::Found(f)) => f.entries,
            _ => Vec::new(),
        };
        let map = loaded.iter().map(|e| (e.id, e.clone())).collect();
        let journal = Arc::new(Self {
            path,
            entries: Mutex::new(map),
            dirty: Notify::new(),
            writing: Mutex::new(()),
        });
        (journal, loaded)
    }

    /// Start the background writer. Must be called inside a Tokio runtime.
    pub fn spawn_writer(self: &Arc<Self>) {
        let this = Arc::clone(self);
        tokio::spawn(async move {
            loop {
                this.dirty.notified().await;
                // Batch bursts of updates (one per finished part).
                tokio::time::sleep(Duration::from_millis(200)).await;
                let me = Arc::clone(&this);
                let _ = tokio::task::spawn_blocking(move || me.flush()).await;
            }
        });
    }

    pub fn upsert(&self, entry: JournalEntry) {
        self.entries.lock().insert(entry.id, entry);
        self.dirty.notify_one();
    }

    pub fn remove(&self, id: JobId) {
        if self.entries.lock().remove(&id).is_some() {
            self.dirty.notify_one();
        }
    }

    /// Write the journal now. Blocking.
    pub fn flush(&self) {
        let _writing = self.writing.lock();
        let file = JournalFile {
            version: JOURNAL_VERSION,
            entries: self.entries.lock().values().cloned().collect(),
        };
        // On Windows an antivirus or indexer can briefly hold the file and
        // make the replace fail; try once more before giving up.
        if write_json_atomic(&self.path, &file).is_err() {
            std::thread::sleep(Duration::from_millis(50));
            if let Err(e) = write_json_atomic(&self.path, &file) {
                tracing::warn!("could not save the transfer journal: {e}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ctx::DownloadState;
    use crate::model::DownloadSpec;
    use r2dock_core::policy::ConflictPolicy;

    fn entry() -> JournalEntry {
        JournalEntry {
            id: uuid::Uuid::new_v4(),
            spec: JobSpec::Download(DownloadSpec {
                connection: uuid::Uuid::nil(),
                bucket: "bkt".into(),
                key: "k".into(),
                size: 10,
                dest_root: PathBuf::from("/tmp"),
                rel_path: PathBuf::from("k"),
                policy: ConflictPolicy::Ask,
            }),
            group: None,
            resume: ResumeState::Download(DownloadState {
                etag: Some("\"abc\"".into()),
                temp: PathBuf::from("/tmp/.k.r2dock-part"),
                final_path: PathBuf::from("/tmp/k"),
            }),
            created_at: "2026-10-17T00:00:00Z".into(),
        }
    }

    #[test]
    fn persists_and_reloads() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("journal.json");
        let (j, loaded) = Journal::load(path.clone());
        assert!(loaded.is_empty());
        let e = entry();
        j.upsert(e.clone());
        j.flush();
        let (_, loaded) = Journal::load(path.clone());
        assert_eq!(loaded, vec![e.clone()]);
        j.remove(e.id);
        j.flush();
        let (_, loaded) = Journal::load(path);
        assert!(loaded.is_empty());
    }

    /// Regression: `u128` fields cannot be read back inside internally
    /// tagged enums, which made every journal with a multipart upload fail
    /// to load.
    #[test]
    fn multipart_and_compressed_entries_reload() {
        use crate::ctx::MultipartState;
        use crate::model::{CompressedCopy, CompressionResult, UploadSpec};
        let fingerprint = crate::source::Fingerprint {
            len: 1,
            mtime_ns: 1_792_272_545_142_633_478,
        };
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("journal.json");
        let (j, _) = Journal::load(path.clone());
        let e = JournalEntry {
            id: uuid::Uuid::new_v4(),
            spec: JobSpec::Upload(UploadSpec {
                connection: uuid::Uuid::nil(),
                bucket: "bkt".into(),
                key: "k.mp3".into(),
                local: PathBuf::from("/tmp/copy.mp3"),
                size: 10,
                policy: ConflictPolicy::Ask,
                identity: None,
                compression: None,
                compressed: Some(CompressionResult::Compressed(CompressedCopy {
                    temp: PathBuf::from("/tmp/copy.mp3"),
                    original_local: PathBuf::from("/tmp/k.mp3"),
                    original_key: "k.mp3".into(),
                    original_size: 20,
                    original: fingerprint,
                    preset: "mp3-32k".into(),
                })),
            }),
            group: None,
            resume: ResumeState::Multipart(MultipartState {
                upload_id: "u".into(),
                key: "k.mp3".into(),
                part_size: 8,
                fingerprint,
                done: [(
                    1,
                    crate::ctx::PartDone {
                        etag: "\"e1\"".into(),
                        md5: "00".into(),
                        len: 8,
                    },
                )]
                .into(),
            }),
            created_at: "2026-10-17T00:00:00Z".into(),
        };
        j.upsert(e.clone());
        j.flush();
        let (_, loaded) = Journal::load(path);
        assert_eq!(loaded, vec![e]);
    }

    #[test]
    fn corrupt_journal_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("journal.json");
        std::fs::write(&path, b"garbage").unwrap();
        let (_, loaded) = Journal::load(path);
        assert!(loaded.is_empty());
    }
}
