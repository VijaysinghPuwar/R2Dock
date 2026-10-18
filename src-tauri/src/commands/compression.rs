//! Settings > Compression: the tools (FFmpeg) used to compress audio.

use tauri::State;
use tauri::ipc::Channel;

use super::CmdResult;
use crate::dialogs;
use crate::dto::{CompressionStatus, InstallProgress, PreviewSample};
use crate::state::AppState;

#[tauri::command]
pub fn compression_status(state: State<'_, AppState>) -> CompressionStatus {
    state.compression.status(&state)
}

/// Download and install the pinned tools, sending progress about four times
/// a second.
#[tauri::command]
pub async fn compression_install(
    state: State<'_, AppState>,
    on_progress: Channel<InstallProgress>,
) -> CmdResult<CompressionStatus> {
    let act = state.activity.start_action("compression_tools_install");
    let mut last = std::time::Instant::now() - std::time::Duration::from_secs(1);
    let result = state
        .compression
        .install(&state, |done, total| {
            if done == total || last.elapsed() >= std::time::Duration::from_millis(250) {
                last = std::time::Instant::now();
                let _ = on_progress.send(InstallProgress { done, total });
            }
        })
        .await;
    act.end(&result);
    result?;
    Ok(state.compression.status(&state))
}

#[tauri::command]
pub fn compression_cancel_install(state: State<'_, AppState>) {
    state.compression.cancel_install();
}

#[tauri::command]
pub async fn compression_remove(state: State<'_, AppState>) -> CmdResult<CompressionStatus> {
    let result = state.compression.remove(&state).await;
    state
        .activity
        .start_action("compression_tools_remove")
        .end(&result);
    result?;
    Ok(state.compression.status(&state))
}

/// Pick an FFmpeg program with the file picker and use it if it works.
/// Returns `None` when the picker is cancelled.
#[tauri::command]
pub async fn compression_use_system(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> CmdResult<Option<CompressionStatus>> {
    let Some(path) = dialogs::pick_ffmpeg(&app).await? else {
        return Ok(None);
    };
    let result = state.compression.use_system(&state, path).await;
    state
        .activity
        .start_action("compression_tools_use_system")
        .end(&result);
    result?;
    Ok(Some(state.compression.status(&state)))
}

#[tauri::command]
pub async fn compression_use_downloaded(
    state: State<'_, AppState>,
) -> CmdResult<CompressionStatus> {
    state.compression.use_downloaded(&state).await?;
    Ok(state.compression.status(&state))
}

/// "Compare quality": pick an audio file and get 30 second samples at each
/// preset. Returns `None` when the picker is cancelled.
#[tauri::command]
pub async fn compression_preview(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> CmdResult<Option<Vec<PreviewSample>>> {
    let ff = state.compression.ffmpeg().ok_or_else(|| {
        r2dock_core::AppError::invalid_input("Install the compression tools first.")
    })?;
    let Some(input) = dialogs::pick_audio(&app).await? else {
        return Ok(None);
    };
    let act = state.activity.start_action("compression_preview");
    // A new run stops the one before it, and each run writes to its own
    // folder so the two never touch the same files.
    let cancel = state.compression.start_preview();
    let root = state.paths.cache.join("preview");
    let out = root.join(uuid::Uuid::new_v4().simple().to_string());
    let sweep_root = root.clone();
    let _ = tokio::task::spawn_blocking(move || sweep_previews(&sweep_root)).await;
    let result = r2dock_media::preview::compare(&ff, &input, &out, &cancel)
        .await
        .map_err(|e| {
            if e.is_cancelled() {
                r2dock_core::AppError::new(r2dock_core::ErrorCode::Cancelled, "Cancelled.")
            } else {
                r2dock_core::AppError::internal(format!("Could not make the samples: {e}."))
            }
        });
    act.end(&result);
    let samples = result?
        .into_iter()
        .map(|s| PreviewSample {
            label: s.label,
            bytes_per_hour: s.bytes_per_hour,
            token: state
                .media
                .issue(crate::media::MediaTarget::Local { path: s.path }),
        })
        .collect();
    Ok(Some(samples))
}

/// Delete earlier preview runs (and samples from older versions, which were
/// written straight into `root`). Best effort: a run that was just stopped
/// may still hold a file, and is cleaned up next time.
fn sweep_previews(root: &std::path::Path) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let _ = if entry.file_type().is_ok_and(|t| t.is_dir()) {
            std::fs::remove_dir_all(path)
        } else {
            std::fs::remove_file(path)
        };
    }
}
