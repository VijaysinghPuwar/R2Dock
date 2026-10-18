//! What a running job needs: client, controls, counters, and hooks back to
//! the engine.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

use parking_lot::Mutex;
use r2dock_r2::R2Client;
use serde::{Deserialize, Serialize};
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;

use crate::model::{JobId, JobState};
use crate::progress::Counter;
use crate::source::Fingerprint;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stop {
    None,
    Pause,
    Cancel,
}

/// Pause and cancel requests for one run of a job.
#[derive(Debug, Default)]
pub struct RunControl {
    pub cancel: CancellationToken,
    /// Fires on any pause or cancel request, so a job waiting for parts in
    /// flight notices a pause without polling.
    pub stopping: CancellationToken,
    stop: AtomicU8,
}

impl RunControl {
    pub fn new() -> Self {
        Self::default()
    }

    /// Multipart uploads finish in-flight parts; downloads stop at once.
    pub fn request_pause(&self, interrupt_streams: bool) {
        let _ = self
            .stop
            .compare_exchange(0, 1, Ordering::SeqCst, Ordering::SeqCst);
        self.stopping.cancel();
        if interrupt_streams {
            self.cancel.cancel();
        }
    }

    pub fn request_cancel(&self) {
        self.stop.store(2, Ordering::SeqCst);
        self.stopping.cancel();
        self.cancel.cancel();
    }

    pub fn stop(&self) -> Stop {
        match self.stop.load(Ordering::SeqCst) {
            1 => Stop::Pause,
            2 => Stop::Cancel,
            _ => Stop::None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PartDone {
    pub etag: String,
    /// Hex MD5 of the part as sent.
    pub md5: String,
    pub len: u64,
}

/// Saved state of a multipart upload, kept across retries, pauses, and restarts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MultipartState {
    pub upload_id: String,
    pub key: String,
    pub part_size: u64,
    pub fingerprint: Fingerprint,
    /// Stored as a list: map keys are JSON strings, which cannot be read back
    /// as numbers inside the journal's internally tagged enums.
    #[serde(with = "parts_as_list")]
    pub done: BTreeMap<i32, PartDone>,
}

mod parts_as_list {
    use std::collections::BTreeMap;

    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    use super::PartDone;

    #[derive(Serialize, Deserialize)]
    struct Item {
        number: i32,
        #[serde(flatten)]
        part: PartDone,
    }

    pub fn serialize<S: Serializer>(
        map: &BTreeMap<i32, PartDone>,
        s: S,
    ) -> Result<S::Ok, S::Error> {
        let items: Vec<Item> = map
            .iter()
            .map(|(n, p)| Item {
                number: *n,
                part: p.clone(),
            })
            .collect();
        items.serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        d: D,
    ) -> Result<BTreeMap<i32, PartDone>, D::Error> {
        let items = Vec::<Item>::deserialize(d)?;
        Ok(items.into_iter().map(|i| (i.number, i.part)).collect())
    }
}

/// Saved state of a partial download.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadState {
    pub etag: Option<String>,
    pub temp: PathBuf,
    pub final_path: PathBuf,
    /// Recorded after the finished partial was synced and checked, just
    /// before it is moved into place. Recovery trusts a final file only when
    /// it matches this record. Older journals have none.
    #[serde(default)]
    pub finalized: Option<crate::source::WrittenFile>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum ResumeState {
    #[default]
    None,
    Multipart(MultipartState),
    Download(DownloadState),
}

/// How a run ended without an error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Completed,
    Skipped,
    NeedsDecision,
    Paused,
}

/// Callbacks from a running job to the engine.
pub trait JobHooks: Send + Sync {
    fn state(&self, id: JobId, state: JobState);
    fn parts(&self, id: JobId, done: u32, total: u32);
    /// The destination changed ("Keep both").
    fn renamed(&self, id: JobId, key: Option<String>, rel_path: Option<PathBuf>);
    fn total(&self, id: JobId, total: u64);
    /// Resume state changed and should be persisted.
    fn checkpoint(&self, id: JobId);
    /// Write the resume journal to disk now (not batched). Blocking.
    fn persist_now(&self);
}

pub struct JobCtx {
    pub id: JobId,
    pub client: R2Client,
    pub control: Arc<RunControl>,
    pub counter: Arc<Counter>,
    /// Global cap on in-flight part requests.
    pub part_slots: Arc<Semaphore>,
    /// Parts in flight per multipart job.
    pub parts_per_job: usize,
    pub multipart_threshold: u64,
    pub resume: Arc<Mutex<ResumeState>>,
    pub hooks: Arc<dyn JobHooks>,
}

impl JobCtx {
    pub fn set_state(&self, state: JobState) {
        self.hooks.state(self.id, state);
    }

    pub fn save_resume(&self, state: ResumeState) {
        *self.resume.lock() = state;
        self.hooks.checkpoint(self.id);
    }

    pub fn clear_resume(&self) {
        self.save_resume(ResumeState::None);
    }
}
