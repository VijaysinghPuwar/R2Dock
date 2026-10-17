//! Getting FFmpeg onto the computer: a pinned download from this project's
//! own GitHub release, verified by SHA-256 before it is unpacked and again
//! before use; or an FFmpeg the user picked ("Use FFmpeg from my system").
//!
//! R2Dock never runs a bare `ffmpeg` from `PATH` and never uses third-party
//! mirrors at runtime.

pub mod detect;
pub mod install;
pub mod manifest;

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ToolsError {
    #[error("compression tools are not available for this system")]
    UnsupportedPlatform,
    #[error("compression tools are not installed")]
    NotInstalled,
    #[error("the download failed")]
    Download(String),
    #[error("the downloaded file did not match its checksum and was deleted")]
    ChecksumMismatch,
    #[error("an installed tool was changed and must be installed again")]
    Tampered,
    #[error("could not write the compression tools")]
    Io(#[from] std::io::Error),
    #[error("cancelled")]
    Cancelled,
    #[error("this FFmpeg cannot be used: {0}")]
    Unusable(String),
}

/// Paths of a ready-to-use FFmpeg and ffprobe.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Tools {
    pub ffmpeg: PathBuf,
    pub ffprobe: PathBuf,
    /// Version text, for Settings ("9.0.2").
    pub version: String,
    pub source: ToolsSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ToolsSource {
    /// Downloaded and verified by R2Dock.
    Bundled,
    /// Picked by the user in Settings.
    System,
}
