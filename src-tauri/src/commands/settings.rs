use r2dock_core::AppError;
use r2dock_core::settings::{Preferences, SettingsFile};
use tauri::State;

use super::CmdResult;
use crate::dialogs;
use crate::dto::SettingsView;
use crate::state::AppState;

pub fn view(state: &AppState) -> SettingsView {
    let preferences = state.settings.read().preferences.clone();
    SettingsView {
        preferences,
        download_dir: state
            .download_dir()
            .map(|p| state.display_path(&p))
            .unwrap_or_else(|| "Not set".to_string()),
        credential_store: state.credential_store_label.clone(),
        credential_store_available: state.connections.keychain_available(),
    }
}

/// Push settings that affect running services.
pub fn apply(state: &AppState, log: Option<&crate::logging::ReloadHandle>) {
    let p = state.settings.read().preferences.clone();
    state.engine.set_concurrency(usize::from(p.concurrency));
    state
        .engine
        .set_multipart_threshold(u64::from(p.multipart_threshold_mib) * r2dock_core::parts::MIB);
    if let Some(handle) = log {
        let _ = handle.reload(crate::logging::filter_for(p.log_level));
    }
}

#[tauri::command]
pub fn settings_get(state: State<'_, AppState>) -> SettingsView {
    view(&state)
}

#[tauri::command]
pub async fn settings_set(
    state: State<'_, AppState>,
    log: State<'_, crate::LogHandle>,
    preferences: Preferences,
) -> CmdResult<SettingsView> {
    state.settings.write().preferences = preferences.sanitized();
    state.save_settings().await?;
    apply(&state, log.0.as_ref());
    Ok(view(&state))
}

#[tauri::command]
pub async fn settings_choose_download_dir(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> CmdResult<SettingsView> {
    if let Some(dir) = dialogs::pick_folder(&app, state.download_dir()).await? {
        if !dir.is_dir() {
            return Err(AppError::invalid_input("Choose an existing folder."));
        }
        state.settings.write().download_dir = Some(dir);
        state.save_settings().await?;
    }
    Ok(view(&state))
}

#[tauri::command]
pub async fn settings_reset(
    state: State<'_, AppState>,
    log: State<'_, crate::LogHandle>,
) -> CmdResult<SettingsView> {
    *state.settings.write() = SettingsFile::default();
    state.save_settings().await?;
    apply(&state, log.0.as_ref());
    Ok(view(&state))
}

#[tauri::command]
pub async fn history_clear(state: State<'_, AppState>) -> CmdResult<()> {
    let engine = state.engine.clone();
    tokio::task::spawn_blocking(move || engine.clear_history())
        .await
        .map_err(|_| AppError::internal("A background task stopped unexpectedly."))
}
