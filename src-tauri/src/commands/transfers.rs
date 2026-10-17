use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use r2dock_core::policy::ConflictPolicy;
use r2dock_core::{AppError, ErrorCode};
use r2dock_r2::bulk::collect_objects;
use r2dock_transfer::history::HistoryEntry;
use r2dock_transfer::scan::scan;
use r2dock_transfer::{
    DownloadSpec, GroupInfo, JobAction, JobSpec, JobState, TransferEvent, UploadSpec,
};
use tauri::ipc::Channel;
use tauri::{AppHandle, Manager, State};
use tauri_plugin_notification::NotificationExt;
use tauri_plugin_opener::OpenerExt;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::{CmdResult, bucket_arg, key_arg, prefix_arg, with_client};
use crate::dialogs;
use crate::dto::{DownloadStarted, Selection, UploadPlan};
use crate::planning::{count_conflicts, default_policy, download_items, upload_items};
use crate::state::{AppState, PreparedUpload};

fn blocking_err() -> AppError {
    AppError::internal("A background task stopped unexpectedly.")
}

/// Register local paths and return an opaque selection.
pub fn register_selection(state: &AppState, paths: Vec<PathBuf>) -> Selection {
    let names = paths
        .iter()
        .take(3)
        .filter_map(|p| {
            p.file_name()
                .map(|n| r2dock_core::keys::display_name(&n.to_string_lossy()))
        })
        .collect();
    let count = u32::try_from(paths.len()).unwrap_or(u32::MAX);
    Selection {
        id: state.selections.insert(paths),
        count,
        names,
    }
}

#[tauri::command]
pub async fn pick_upload_files(
    app: AppHandle,
    state: State<'_, AppState>,
) -> CmdResult<Option<Selection>> {
    Ok(dialogs::pick_files(&app)
        .await?
        .filter(|p| !p.is_empty())
        .map(|paths| register_selection(&state, paths)))
}

#[tauri::command]
pub async fn pick_upload_folder(
    app: AppHandle,
    state: State<'_, AppState>,
) -> CmdResult<Option<Selection>> {
    Ok(dialogs::pick_upload_folder(&app)
        .await?
        .map(|path| register_selection(&state, vec![path])))
}

#[tauri::command]
pub async fn upload_prepare(
    state: State<'_, AppState>,
    selection_id: String,
    connection_id: Uuid,
    bucket: String,
    prefix: String,
) -> CmdResult<UploadPlan> {
    let bucket = bucket_arg(&bucket)?;
    let prefix = prefix_arg(&prefix)?;
    let paths = state.selections.get(&selection_id).ok_or_else(|| {
        AppError::not_found("The selected files are no longer available. Choose them again.")
    })?;
    let include_hidden = state.settings.read().preferences.include_hidden_os_files;
    let home = state.paths.home.clone();
    let scanned =
        tokio::task::spawn_blocking(move || scan(&paths, include_hidden, home.as_deref()))
            .await
            .map_err(|_| blocking_err())?;
    let too_many = scanned.too_many;
    let folders = scanned.folders;
    let has_folders = !scanned.top_folders.is_empty();
    let label = match scanned.top_folders.as_slice() {
        [one] => one.clone(),
        _ => String::new(),
    };
    let bytes = scanned.bytes;
    let items = upload_items(scanned, &prefix, state.paths.home.as_deref());
    let keys: Vec<String> = items.files.iter().map(|f| f.key.clone()).collect();
    let (b, p) = (bucket.clone(), prefix.clone());
    let conflicts = with_client(&state, connection_id, |c| async move {
        count_conflicts(&c, &b, &p, &keys).await
    })
    .await?;

    let files = u32::try_from(items.files.len()).unwrap_or(u32::MAX);
    let setting = state.settings.read().preferences.conflict_policy;
    let label = if label.is_empty() {
        if files == 1 {
            "1 file".to_string()
        } else {
            format!("{files} files")
        }
    } else {
        label
    };
    let needs_review =
        has_folders || files > 20 || conflicts > 0 || !items.skipped.is_empty() || too_many;
    let skipped = items.skipped.clone();
    let plan_id = state.uploads.insert(PreparedUpload {
        connection: connection_id,
        bucket,
        grouped: has_folders || items.files.len() > 1,
        files: items.files,
        markers: items.markers,
        label,
    });
    Ok(UploadPlan {
        plan_id,
        files,
        folders,
        bytes,
        conflicts,
        skipped,
        has_folders,
        needs_review,
        too_many,
        default_policy: default_policy(setting, has_folders),
    })
}

#[tauri::command]
pub async fn upload_start(
    state: State<'_, AppState>,
    plan_id: String,
    policy: ConflictPolicy,
) -> CmdResult<u32> {
    let plan = state
        .uploads
        .take(&plan_id)
        .ok_or_else(|| AppError::not_found("This upload expired. Choose the files again."))?;
    if !plan.markers.is_empty() {
        let (bucket, markers) = (plan.bucket.clone(), plan.markers.clone());
        with_client(&state, plan.connection, |c| async move {
            for m in markers {
                c.create_folder(&bucket, &m).await?;
            }
            Ok(())
        })
        .await?;
    }
    let specs: Vec<JobSpec> = plan
        .files
        .into_iter()
        .map(|f| {
            JobSpec::Upload(UploadSpec {
                connection: plan.connection,
                bucket: plan.bucket.clone(),
                key: f.key,
                local: f.local,
                size: f.size,
                policy,
                identity: f.identity,
            })
        })
        .collect();
    let group = plan.grouped.then(|| GroupInfo {
        id: Uuid::new_v4(),
        label: plan.label.clone(),
    });
    let n = u32::try_from(specs.len()).unwrap_or(u32::MAX);
    state.engine.enqueue(specs, group);
    Ok(n)
}

#[tauri::command]
pub async fn download_start(
    app: AppHandle,
    state: State<'_, AppState>,
    connection_id: Uuid,
    bucket: String,
    keys: Vec<String>,
    prefixes: Vec<String>,
) -> CmdResult<Option<DownloadStarted>> {
    let bucket = bucket_arg(&bucket)?;
    if keys.is_empty() && prefixes.is_empty() {
        return Err(AppError::invalid_input("Nothing is selected."));
    }
    for k in &keys {
        key_arg(k)?;
    }
    for p in &prefixes {
        prefix_arg(p)?;
    }
    let ask = state.settings.read().preferences.ask_where_to_save;
    let dest = if ask {
        match dialogs::pick_folder(&app, state.download_dir()).await? {
            Some(d) => d,
            None => return Ok(None),
        }
    } else {
        state
            .download_dir()
            .ok_or_else(|| AppError::invalid_input("Choose a download folder in Settings."))?
    };
    std::fs::create_dir_all(&dest)
        .map_err(|e| r2dock_core::app_error::io_error(&e, "open the download folder"))?;

    let (b, k, p) = (bucket.clone(), keys.clone(), prefixes.clone());
    let objects = with_client(&state, connection_id, |c| async move {
        collect_objects(&c, &b, &k, &p, &CancellationToken::new()).await
    })
    .await?;
    let (items, dirs) = download_items(&objects, &keys, &prefixes);
    for dir in dirs {
        let _ = r2dock_core::path_guard::ensure_dir_under_root(&dest, &dir);
    }
    let policy = state.settings.read().preferences.conflict_policy;
    let specs: Vec<JobSpec> = items
        .into_iter()
        .map(|i| {
            JobSpec::Download(DownloadSpec {
                connection: connection_id,
                bucket: bucket.clone(),
                key: i.key,
                size: i.size,
                dest_root: dest.clone(),
                rel_path: i.rel_path,
                policy,
            })
        })
        .collect();
    let n = u32::try_from(specs.len()).unwrap_or(u32::MAX);
    let group = (!prefixes.is_empty() || specs.len() > 1).then(|| GroupInfo {
        id: Uuid::new_v4(),
        label: match (prefixes.as_slice(), keys.is_empty()) {
            ([one], true) => r2dock_core::keys::base_name(one).to_string(),
            _ => format!("{n} files"),
        },
    });
    state.engine.enqueue(specs, group);
    Ok(Some(DownloadStarted {
        jobs: n,
        destination: state.display_path(&dest),
    }))
}

/// Subscribe the webview to transfer events. Also sends a system
/// notification when work finishes while the window is in the background.
#[tauri::command]
pub async fn transfer_subscribe(
    app: AppHandle,
    state: State<'_, AppState>,
    on_event: Channel<TransferEvent>,
) -> CmdResult<()> {
    let engine = state.engine.clone();
    let busy = Arc::new(AtomicBool::new(false));
    let failed = Arc::new(std::sync::atomic::AtomicU32::new(0));
    let completed = Arc::new(std::sync::atomic::AtomicU32::new(0));
    let sink_engine = engine.clone();
    engine.subscribe(Arc::new(move |event: TransferEvent| {
        if let TransferEvent::Updated { job } = &event {
            match job.state {
                JobState::Completed => {
                    completed.fetch_add(1, Ordering::Relaxed);
                }
                JobState::Failed => {
                    failed.fetch_add(1, Ordering::Relaxed);
                }
                s if s.is_active() => busy.store(true, Ordering::Relaxed),
                _ => {}
            }
            if job.state.is_terminal() || job.state == JobState::Failed {
                let idle = sink_engine.active_count() == 0;
                if idle && busy.swap(false, Ordering::Relaxed) {
                    let done = completed.swap(0, Ordering::Relaxed);
                    let bad = failed.swap(0, Ordering::Relaxed);
                    // Never block the event path on window queries.
                    let app = app.clone();
                    tauri::async_runtime::spawn(async move {
                        notify_if_background(&app, done, bad);
                    });
                }
            }
        }
        let _ = on_event.send(event);
    }));
    Ok(())
}

fn notify_if_background(app: &AppHandle, done: u32, failed: u32) {
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    let prefs = state.settings.read().preferences.clone();
    let focused = app
        .get_webview_window("main")
        .and_then(|w| w.is_focused().ok())
        .unwrap_or(true);
    if focused {
        return;
    }
    let (title, body) = if failed > 0 {
        if !prefs.notify_on_failure {
            return;
        }
        (
            "Transfers finished with problems",
            format!("{done} completed, {failed} failed."),
        )
    } else {
        if !prefs.notify_on_complete {
            return;
        }
        (
            "Transfers complete",
            if done == 1 {
                "1 file transferred.".to_string()
            } else {
                format!("{done} files transferred.")
            },
        )
    };
    let _ = app.notification().builder().title(title).body(body).show();
}

#[tauri::command]
pub fn transfer_control(state: State<'_, AppState>, id: Uuid, action: JobAction) -> CmdResult<()> {
    state.engine.control(id, action)
}

#[tauri::command]
pub fn transfer_control_many(state: State<'_, AppState>, ids: Vec<Uuid>, action: JobAction) {
    state.engine.control_many(&ids, action);
}

#[tauri::command]
pub fn transfer_pause_all(state: State<'_, AppState>) {
    state.engine.pause_all();
}

#[tauri::command]
pub fn transfer_clear_finished(state: State<'_, AppState>) {
    state.engine.clear_finished();
}

#[tauri::command]
pub async fn transfer_history(
    state: State<'_, AppState>,
    limit: u32,
) -> CmdResult<Vec<HistoryEntry>> {
    let engine = state.engine.clone();
    let limit = usize::try_from(limit.min(1000)).unwrap_or(1000);
    tokio::task::spawn_blocking(move || engine.history(limit))
        .await
        .map_err(|_| blocking_err())
}

/// Reveal a finished download in the system file manager.
#[tauri::command]
pub fn transfer_show(app: AppHandle, state: State<'_, AppState>, id: Uuid) -> CmdResult<()> {
    let path = state
        .engine
        .download_path(id)
        .ok_or_else(|| AppError::not_found("This download is not available."))?;
    if !path.exists() {
        return Err(AppError::new(
            ErrorCode::NotFound,
            "The downloaded file was moved or deleted.",
        ));
    }
    app.opener()
        .reveal_item_in_dir(path)
        .map_err(|_| AppError::internal("Could not open the folder."))
}
