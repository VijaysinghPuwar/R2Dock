//! Video compression (docs/plans/media-compression.md, step 6): ffprobe
//! probing, the decisions, encoder detection, and FFmpeg arguments.

pub mod args;
pub mod encoders;
pub mod plan;
pub mod probe;
pub mod run;
