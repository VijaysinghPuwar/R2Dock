//! Audio and video compression planning for R2Dock
//! (docs/plans/media-compression.md). No Tauri dependency.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

pub mod classify;
pub mod estimate;
pub mod ffmpeg;
pub mod keys;
pub mod pipeline;
pub mod plan;
pub mod preview;
pub mod probe;
pub mod tools;
pub mod verify;
pub mod video;
