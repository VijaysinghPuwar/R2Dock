//! R2 access through the official AWS S3 SDK, configured for Cloudflare R2.
//!
//! This crate is the only place (besides the secret store) that ever sees a
//! secret: it is handed to the SDK credentials provider when a client is built,
//! and the optional Cloudflare API token is sent only by `cfapi`.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

pub mod bulk;
pub mod cfapi;
pub mod client;
pub mod dto;
pub mod error;
pub mod list;
pub mod ops;

pub use client::{ClientParams, R2Client};
pub use error::R2Error;
