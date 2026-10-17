//! IPC data shapes owned by the app layer. None of them carries a secret.

use std::collections::BTreeMap;

use r2dock_core::connections::Jurisdiction;
use r2dock_core::policy::ConflictPolicy;
use r2dock_core::settings::Preferences;
use r2dock_r2::dto::{BucketEntry, KeyFailure, ObjectDetails};
use r2dock_transfer::scan::SkippedItem;
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum Platform {
    Macos,
    Windows,
    Linux,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AppInfo {
    pub name: String,
    pub version: String,
    pub platform: Platform,
    /// For example "macOS Keychain".
    pub credential_store: String,
    /// False when secrets can only be kept for this session.
    pub credential_store_available: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ConnectionStatus {
    Unknown,
    Online,
    Offline,
    NeedsAttention,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum SecretStorage {
    /// In the OS credential store.
    Keychain,
    /// In memory until the app quits.
    Session,
    /// The saved secret is gone (removed outside the app, or session ended).
    Missing,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ConnectionSummary {
    pub id: Uuid,
    pub name: String,
    pub account_id: String,
    pub account_id_short: String,
    /// Shortened access key id, never the full value.
    pub access_key_hint: String,
    pub jurisdiction: Jurisdiction,
    pub secret_storage: SecretStorage,
    pub status: ConnectionStatus,
    pub bucket_scoped: bool,
    pub manual_buckets: Vec<String>,
    pub favorite_buckets: Vec<String>,
    pub public_base_urls: BTreeMap<String, String>,
    pub last_tested_at: Option<String>,
    pub active: bool,
}

/// Connect form input. `secret` is only sent when typed; it is never returned.
#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ConnectionInput {
    /// Set when editing an existing connection.
    pub id: Option<Uuid>,
    pub name: String,
    pub account_id: String,
    pub access_key_id: String,
    /// Required for new connections. When editing, `None` keeps the stored secret.
    pub secret: Option<String>,
    pub bucket: Option<String>,
    pub jurisdiction: Jurisdiction,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TestResult {
    /// Plain sentence for the inline result area.
    pub message: String,
    pub bucket_scoped: bool,
    pub buckets: Vec<BucketEntry>,
    /// A non-blocking warning (for example an unusual account ID).
    pub warning: Option<String>,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ObjectInfo {
    pub details: ObjectDetails,
    /// Built from the bucket's configured public base URL, if any.
    pub public_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TextPreview {
    pub text: String,
    pub truncated: bool,
    /// True when the content does not look like text.
    pub binary: bool,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DeletePreview {
    pub token: String,
    #[ts(type = "number")]
    pub objects: u64,
    #[ts(type = "number")]
    pub bytes: u64,
    /// The user must type `typed_phrase` to confirm.
    pub needs_typed: bool,
    pub typed_phrase: String,
    /// Up to 5 example keys, for the dialog.
    pub sample: Vec<String>,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DeleteResult {
    #[ts(type = "number")]
    pub deleted: u64,
    pub failures: Vec<KeyFailure>,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct MovePreview {
    pub token: String,
    #[ts(type = "number")]
    pub objects: u64,
    #[ts(type = "number")]
    pub bytes: u64,
    pub destination: String,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct MoveResult {
    #[ts(type = "number")]
    pub moved: u64,
    pub failures: Vec<KeyFailure>,
    pub copied_not_deleted: Vec<KeyFailure>,
}

/// Progress for bulk delete and move, sent over a channel.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct OpProgress {
    #[ts(type = "number")]
    pub done: u64,
    #[ts(type = "number")]
    pub total: u64,
}

/// Local files chosen with a picker or drag and drop, referenced by id.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Selection {
    pub id: String,
    pub count: u32,
    /// Up to 3 item names, for display.
    pub names: Vec<String>,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct UploadPlan {
    pub plan_id: String,
    pub files: u32,
    pub folders: u32,
    #[ts(type = "number")]
    pub bytes: u64,
    pub conflicts: u32,
    pub skipped: Vec<SkippedItem>,
    pub has_folders: bool,
    /// Review dialog required: folders, more than 20 files, conflicts, or skipped items.
    pub needs_review: bool,
    pub too_many: bool,
    pub default_policy: ConflictPolicy,
    /// Present when compression is on and the upload has audio files.
    pub compression: Option<CompressionEstimate>,
}

/// What compression would do to an upload (from file headers only).
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CompressionEstimate {
    /// Audio files that would be compressed.
    pub files: u32,
    #[ts(type = "number")]
    pub bytes_before: u64,
    #[ts(type = "number")]
    pub bytes_after: u64,
    /// Audio files that would be uploaded as they are.
    pub unchanged: u32,
    /// How many of `files` are videos.
    pub video_files: u32,
    /// Keys whose extension changes (first 50).
    pub key_changes: Vec<KeyChange>,
    /// Total number of key changes.
    pub key_change_count: u32,
    /// New keys used by more than one file, or already used in this upload.
    pub collisions: Vec<String>,
    /// FFmpeg is ready; otherwise files upload unchanged.
    pub tools_ready: bool,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct KeyChange {
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ToolsState {
    NotInstalled,
    Installing,
    Ready,
    /// The chosen FFmpeg could not be used (see `problem`).
    Unusable,
}

/// Settings > Compression: the tools.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CompressionStatus {
    pub state: ToolsState,
    pub source: Option<r2dock_media::tools::ToolsSource>,
    pub version: Option<String>,
    pub license: Option<r2dock_media::tools::detect::License>,
    /// Download size for this system; `None` when no package exists.
    #[ts(type = "number | null")]
    pub download_bytes: Option<u64>,
    /// A picked system FFmpeg, home shown as `~`.
    pub system_path: Option<String>,
    pub problem: Option<String>,
}

/// One "Compare quality" sample, playable through the media scheme.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PreviewSample {
    pub label: String,
    #[ts(type = "number")]
    pub bytes_per_hour: u64,
    /// Media token; turn it into a URL with `mediaUrl`.
    pub token: String,
}

#[derive(Debug, Clone, Copy, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct InstallProgress {
    #[ts(type = "number")]
    pub done: u64,
    #[ts(type = "number")]
    pub total: u64,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DownloadStarted {
    pub jobs: u32,
    pub destination: String,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SettingsView {
    pub preferences: Preferences,
    pub download_dir: String,
    /// Activity log folder, with the home folder shown as `~`.
    pub activity_dir: String,
    /// Whether the activity folder is the default one.
    pub activity_dir_is_default: bool,
    /// Technical (diagnostic) log folder, with the home folder shown as `~`.
    pub log_dir: String,
    pub credential_store: String,
    pub credential_store_available: bool,
}

/// Which rows to include in an activity export.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ExportRange {
    Today,
    Last7Days,
    Last30Days,
    All,
}

/// Which log folder a command acts on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum LogKind {
    Activity,
    Technical,
}

/// Drag state from the OS, emitted while files are dragged over the window.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase", tag = "type")]
#[ts(export)]
pub enum DragEvent {
    Enter {
        count: u32,
        x: f64,
        y: f64,
    },
    Over {
        x: f64,
        y: f64,
    },
    Leave,
    Drop {
        selection: Selection,
        x: f64,
        y: f64,
    },
}
