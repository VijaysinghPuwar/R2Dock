use tauri::State;
use uuid::Uuid;

use super::CmdResult;
use crate::dto::{ConnectionInput, ConnectionSummary, TestResult};
use crate::state::AppState;

#[tauri::command]
pub fn connection_list(state: State<'_, AppState>) -> Vec<ConnectionSummary> {
    state.connections.list()
}

/// The secret in `input` is used for this call only and never returned or logged.
#[tauri::command]
pub async fn connection_test(
    state: State<'_, AppState>,
    input: ConnectionInput,
) -> CmdResult<TestResult> {
    state.connections.test(&input).await
}

#[tauri::command]
pub async fn connection_save(
    state: State<'_, AppState>,
    input: ConnectionInput,
) -> CmdResult<ConnectionSummary> {
    state.connections.save(&input).await
}

#[tauri::command]
pub async fn connection_remove(state: State<'_, AppState>, id: Uuid) -> CmdResult<()> {
    state.connections.remove(id).await?;
    state
        .engine
        .fail_connection(id, "This connection was removed.");
    Ok(())
}

#[tauri::command]
pub async fn connection_set_active(state: State<'_, AppState>, id: Uuid) -> CmdResult<()> {
    state.connections.set_active(id).await
}

#[tauri::command]
pub async fn connection_add_bucket(
    state: State<'_, AppState>,
    id: Uuid,
    bucket: String,
) -> CmdResult<ConnectionSummary> {
    state.connections.add_bucket(id, &bucket).await
}

#[tauri::command]
pub async fn connection_remove_bucket(
    state: State<'_, AppState>,
    id: Uuid,
    bucket: String,
) -> CmdResult<ConnectionSummary> {
    state.connections.remove_bucket(id, &bucket).await
}

#[tauri::command]
pub async fn connection_set_public_url(
    state: State<'_, AppState>,
    id: Uuid,
    bucket: String,
    url: Option<String>,
) -> CmdResult<ConnectionSummary> {
    state
        .connections
        .set_public_url(id, &bucket, url.as_deref())
        .await
}

#[tauri::command]
pub async fn connection_toggle_favorite(
    state: State<'_, AppState>,
    id: Uuid,
    bucket: String,
) -> CmdResult<ConnectionSummary> {
    state.connections.toggle_favorite(id, &bucket).await
}
