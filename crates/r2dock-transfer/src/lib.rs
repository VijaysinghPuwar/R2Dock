//! The R2Dock transfer engine (PLAN.md section 12).
//!
//! Streaming uploads and downloads with bounded concurrency, parallel
//! multipart parts, pause, resume, cancel, retries, a restart journal, and
//! throttled progress events. Nothing here depends on Tauri.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

mod body;
pub mod ctx;
mod download;
pub mod engine;
pub mod history;
pub mod journal;
pub mod model;
pub mod progress;
pub mod retry;
pub mod scan;
pub mod source;
mod upload;

pub use download::human_bytes;
pub use engine::{BoxFuture, ClientProvider, Engine, EngineConfig, EventSink};
pub use model::*;
