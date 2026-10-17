//! Thin command handlers: validate input, call a service, map errors.
//! Every command here must also be listed in `build.rs` and the capability file.

pub mod app;
pub mod browse;
pub mod connections;
pub mod manage;
pub mod settings;
pub mod transfers;

use r2dock_core::AppError;
use r2dock_r2::{R2Client, R2Error};
use uuid::Uuid;

use crate::state::AppState;

pub type CmdResult<T> = Result<T, AppError>;

/// Run an R2 call for a connection, tracking its online status.
pub async fn with_client<T, F, Fut>(state: &AppState, connection: Uuid, f: F) -> CmdResult<T>
where
    F: FnOnce(R2Client) -> Fut,
    Fut: std::future::Future<Output = Result<T, R2Error>>,
{
    let client = state.connections.client(connection).await?;
    let result = f(client).await;
    state.connections.note_result(connection, &result);
    result.map_err(AppError::from)
}

/// Validate a bucket name argument.
pub fn bucket_arg(bucket: &str) -> CmdResult<String> {
    r2dock_core::connections::validate_bucket_name(bucket)
}

/// Validate a folder prefix argument.
pub fn prefix_arg(prefix: &str) -> CmdResult<String> {
    r2dock_core::keys::validate_prefix(prefix)
        .map_err(|e| AppError::invalid_input(e.to_string()))?;
    Ok(prefix.to_string())
}

/// Validate an object key argument.
pub fn key_arg(key: &str) -> CmdResult<String> {
    r2dock_core::keys::validate_key(key).map_err(|e| AppError::invalid_input(e.to_string()))?;
    Ok(key.to_string())
}

/// Run a command body and record its result in the activity log.
pub async fn logged<T, Fut>(act: crate::activity::Act, body: Fut) -> CmdResult<T>
where
    Fut: std::future::Future<Output = CmdResult<T>>,
{
    let result = body.await;
    act.end(&result);
    result
}
