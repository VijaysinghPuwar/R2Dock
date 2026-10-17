//! Job model and the events sent to the frontend (PLAN.md 12.2, 12.4).

use std::path::PathBuf;

use r2dock_core::AppError;
use r2dock_core::policy::ConflictPolicy;
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

pub type JobId = Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum Direction {
    Upload,
    Download,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum JobState {
    Queued,
    /// Audio is being compressed before upload. Does not hold an upload slot.
    Compressing,
    Preparing,
    Running,
    Paused,
    Retrying,
    Verifying,
    /// A conflict needs the user's choice (policy "Ask").
    NeedsDecision,
    Completed,
    Failed,
    Cancelled,
    Skipped,
    /// Stopped by an app quit or crash; can be resumed or discarded.
    Interrupted,
}

impl JobState {
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Cancelled | Self::Skipped)
    }

    /// States that hold a scheduler slot.
    pub fn is_active(self) -> bool {
        matches!(
            self,
            Self::Preparing | Self::Running | Self::Retrying | Self::Verifying
        )
    }
}

/// What to upload: one local file to one key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadSpec {
    pub connection: Uuid,
    pub bucket: String,
    pub key: String,
    pub local: PathBuf,
    pub size: u64,
    pub policy: ConflictPolicy,
    /// Identity recorded at scan time, when the platform provides one.
    #[serde(default)]
    pub identity: Option<crate::source::FileIdentity>,
    /// Compress audio before uploading (docs/plans/media-compression.md).
    #[serde(default)]
    pub compression: Option<CompressionRequest>,
    /// What compression decided; kept in the journal across restarts.
    #[serde(default)]
    pub compressed: Option<CompressionResult>,
}

impl UploadSpec {
    /// Compression was asked for and has not run yet.
    pub fn needs_compression(&self) -> bool {
        self.compression.is_some() && self.compressed.is_none()
    }

    /// The compressed copy being uploaded, if any.
    pub fn compressed_copy(&self) -> Option<&CompressedCopy> {
        match &self.compressed {
            Some(CompressionResult::Compressed(c)) => Some(c),
            _ => None,
        }
    }
}

/// What to do when compression fails.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum CompressionFailure {
    #[default]
    UploadOriginal,
    SkipFile,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompressionRequest {
    pub settings: r2dock_media::plan::AudioSettings,
    #[serde(default)]
    pub content: r2dock_media::classify::ContentOverride,
    #[serde(default)]
    pub on_failure: CompressionFailure,
}

/// A compressed copy in R2Dock's work folder, uploaded instead of the original.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompressedCopy {
    pub temp: PathBuf,
    pub original_local: PathBuf,
    pub original_key: String,
    pub original_size: u64,
    /// The original as it was when compressed; a change means compress again.
    pub original: crate::source::Fingerprint,
    pub preset: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum CompressionResult {
    Compressed(CompressedCopy),
    Unchanged {
        reason: r2dock_media::pipeline::Unchanged,
    },
    Failed {
        message: String,
    },
}

/// Compression as the frontend sees it.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind"
)]
#[ts(export)]
pub enum CompressionView {
    /// Waiting for an encode slot or running.
    Pending {
        /// 0.0 to 1.0 while compressing.
        fraction: Option<f64>,
    },
    Saved {
        #[ts(type = "number")]
        original_bytes: u64,
        #[ts(type = "number")]
        bytes: u64,
        preset: String,
    },
    Unchanged {
        reason: r2dock_media::pipeline::Unchanged,
    },
    Failed {
        message: String,
    },
}

impl CompressionView {
    pub fn of(spec: &UploadSpec) -> Option<Self> {
        spec.compression.as_ref()?;
        Some(match &spec.compressed {
            None => Self::Pending { fraction: None },
            Some(CompressionResult::Compressed(c)) => Self::Saved {
                original_bytes: c.original_size,
                bytes: spec.size,
                preset: c.preset.clone(),
            },
            Some(CompressionResult::Unchanged { reason }) => Self::Unchanged {
                reason: reason.clone(),
            },
            Some(CompressionResult::Failed { message }) => Self::Failed {
                message: message.clone(),
            },
        })
    }
}

/// What to download: one key to a path relative to a destination root.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadSpec {
    pub connection: Uuid,
    pub bucket: String,
    pub key: String,
    pub size: u64,
    pub dest_root: PathBuf,
    pub rel_path: PathBuf,
    pub policy: ConflictPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum JobSpec {
    Upload(UploadSpec),
    Download(DownloadSpec),
}

impl JobSpec {
    pub fn direction(&self) -> Direction {
        match self {
            Self::Upload(_) => Direction::Upload,
            Self::Download(_) => Direction::Download,
        }
    }

    pub fn connection(&self) -> Uuid {
        match self {
            Self::Upload(u) => u.connection,
            Self::Download(d) => d.connection,
        }
    }

    pub fn bucket(&self) -> &str {
        match self {
            Self::Upload(u) => &u.bucket,
            Self::Download(d) => &d.bucket,
        }
    }

    pub fn key(&self) -> &str {
        match self {
            Self::Upload(u) => &u.key,
            Self::Download(d) => &d.key,
        }
    }

    pub fn size(&self) -> u64 {
        match self {
            Self::Upload(u) => u.size,
            Self::Download(d) => d.size,
        }
    }

    pub fn set_policy(&mut self, policy: ConflictPolicy) {
        match self {
            Self::Upload(u) => u.policy = policy,
            Self::Download(d) => d.policy = policy,
        }
    }
}

/// A folder upload or download shown as one group row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct GroupInfo {
    pub id: Uuid,
    pub label: String,
}

/// The job state as the frontend sees it.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct JobSnapshot {
    pub id: JobId,
    pub group: Option<GroupInfo>,
    pub direction: Direction,
    pub connection_id: Uuid,
    pub bucket: String,
    /// The key as it will be written (may change with "Keep both").
    pub key: String,
    pub name: String,
    /// Local location with the home folder shown as `~`.
    pub local_display: String,
    #[ts(type = "number")]
    pub total_bytes: u64,
    #[ts(type = "number")]
    pub transferred_bytes: u64,
    pub state: JobState,
    pub speed_bps: Option<f64>,
    pub eta_secs: Option<f64>,
    pub attempt: u32,
    pub max_attempts: u32,
    pub parts_done: Option<u32>,
    pub parts_total: Option<u32>,
    pub pausable: bool,
    pub error: Option<AppError>,
    /// RFC 3339.
    pub created_at: String,
    pub finished_at: Option<String>,
    /// Present for uploads that asked for compression.
    pub compression: Option<CompressionView>,
}

/// Lightweight progress update for an active job.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ProgressUpdate {
    pub id: JobId,
    #[ts(type = "number")]
    pub transferred_bytes: u64,
    pub speed_bps: Option<f64>,
    pub eta_secs: Option<f64>,
    pub parts_done: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "type"
)]
#[ts(export)]
pub enum TransferEvent {
    /// Full state, sent when the frontend subscribes.
    Snapshot {
        jobs: Vec<JobSnapshot>,
    },
    Added {
        jobs: Vec<JobSnapshot>,
    },
    Updated {
        job: Box<JobSnapshot>,
    },
    Progress {
        updates: Vec<ProgressUpdate>,
    },
    Removed {
        ids: Vec<JobId>,
    },
    /// A connection's credentials were rejected; its jobs stopped.
    AuthFailed {
        connection_id: Uuid,
    },
}

/// User controls for a job.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", tag = "action", content = "policy")]
#[ts(export)]
pub enum JobAction {
    Pause,
    Resume,
    Cancel,
    Retry,
    /// Resolve a conflict for a job waiting in `NeedsDecision`.
    Resolve(ConflictPolicy),
    /// Remove a finished or interrupted job from the list (discarding any partial data).
    Dismiss,
}
