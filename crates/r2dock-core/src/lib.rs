//! Pure, I/O-light logic shared by the R2Dock crates.
//!
//! Nothing in this crate talks to R2 or depends on Tauri, so every rule here
//! can be unit tested directly.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

pub mod app_error;
pub mod compression;
pub mod connections;
pub mod etag;
pub mod keys;
pub mod parts;
pub mod path_guard;
pub mod policy;
pub mod redact;
pub mod settings;
pub mod storage;

/// Product name, kept in one place so a rename stays small.
pub const APP_NAME: &str = "R2Dock";

/// Service name used for entries in the OS credential store.
pub const CREDENTIAL_SERVICE: &str = "app.r2dock.credentials";

pub use app_error::{AppError, ErrorCode};
