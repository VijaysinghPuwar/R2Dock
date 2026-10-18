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

async fn pick_upload_files_inner(app: AppHandle, state: &AppState) -> CmdResult<Option<Selection>> {
    Ok(dialogs::pick_files(&app)
        .await?
        .filter(|p| !p.is_empty())
        .map(|paths| register_selection(state, paths)))
}

async fn pick_upload_folder_inner(
    app: AppHandle,
    state: &AppState,
) -> CmdResult<Option<Selection>> {
    Ok(dialogs::pick_upload_folder(&app)
        .await?
        .map(|path| register_selection(state, vec![path])))
}

async fn upload_prepare_inner(
    state: &AppState,
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
    let mut items = upload_items(scanned, &prefix, state.paths.home.as_deref());
    let compression_prefs = state.settings.read().preferences.compression;
    let estimate = if compression_prefs.enabled {
        let mut files = std::mem::take(&mut items.files);
        let ffmpeg = state.compression.ffmpeg();
        let (files, estimate) = tokio::task::spawn_blocking(move || {
            let e = crate::planning::compression_estimate(
                &mut files,
                &compression_prefs,
                ffmpeg.as_ref(),
            );
            (files, e)
        })
        .await
        .map_err(|_| blocking_err())?;
        items.files = files;
        estimate
    } else {
        None
    };
    // Check both the original keys and the keys after compression.
    let mut keys: Vec<String> = items.files.iter().map(|f| f.key.clone()).collect();
    keys.extend(items.files.iter().filter_map(|f| match &f.compression {
        Some(r2dock_media::estimate::FileDecision::Compress { new_key, .. })
            if *new_key != f.key =>
        {
            Some(new_key.clone())
        }
        _ => None,
    }));
    let (b, p) = (bucket.clone(), prefix.clone());
    let conflicts = with_client(state, connection_id, |c| async move {
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
    let needs_review = has_folders
        || files > 20
        || conflicts > 0
        || !items.skipped.is_empty()
        || too_many
        || estimate.is_some();
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
        compression: estimate,
    })
}

async fn upload_start_inner(
    state: &AppState,
    plan_id: String,
    policy: ConflictPolicy,
    compress: bool,
) -> CmdResult<u32> {
    let plan = state
        .uploads
        .take(&plan_id)
        .ok_or_else(|| AppError::not_found("This upload expired. Choose the files again."))?;
    if !plan.markers.is_empty() {
        let (bucket, markers) = (plan.bucket.clone(), plan.markers.clone());
        with_client(state, plan.connection, |c| async move {
            for m in markers {
                c.create_folder(&bucket, &m).await?;
            }
            Ok(())
        })
        .await?;
    }
    let prefs = state.settings.read().preferences.compression;
    let request = r2dock_transfer::CompressionRequest {
        settings: prefs.audio,
        content: r2dock_media::classify::ContentOverride::Auto,
        on_failure: prefs.on_failure,
        video: prefs.video,
    };
    let specs: Vec<JobSpec> = plan
        .files
        .into_iter()
        .map(|f| {
            use r2dock_media::estimate::FileDecision;
            // Only audio files get a compression request; the engine makes
            // the final decision. Files the estimate left alone show why.
            let (compression, compressed) = match f.compression {
                Some(FileDecision::Compress { .. }) if compress && prefs.enabled => {
                    (Some(request.clone()), None)
                }
                Some(FileDecision::Unchanged(reason)) if compress && prefs.enabled => (
                    Some(request.clone()),
                    Some(r2dock_transfer::CompressionResult::Unchanged { reason }),
                ),
                _ => (None, None),
            };
            JobSpec::Upload(UploadSpec {
                connection: plan.connection,
                bucket: plan.bucket.clone(),
                key: f.key,
                local: f.local,
                size: f.size,
                policy,
                identity: f.identity,
                compression,
                compressed,
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

async fn download_start_inner(
    app: AppHandle,
    state: &AppState,
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
    let objects = with_client(state, connection_id, |c| async move {
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

#[tauri::command]
pub async fn pick_upload_files(
    app: AppHandle,
    state: State<'_, AppState>,
) -> CmdResult<Option<Selection>> {
    let mut act = state.activity.start_action("pick_files");
    let result = pick_upload_files_inner(app, &state).await;
    if let Ok(sel) = &result {
        act.set("count", sel.as_ref().map_or(0, |s| s.count));
        act.set("cancelled", sel.is_none());
    }
    act.end(&result);
    result
}

#[tauri::command]
pub async fn pick_upload_folder(
    app: AppHandle,
    state: State<'_, AppState>,
) -> CmdResult<Option<Selection>> {
    let mut act = state.activity.start_action("pick_folder");
    let result = pick_upload_folder_inner(app, &state).await;
    if let Ok(sel) = &result {
        act.set("cancelled", sel.is_none());
    }
    act.end(&result);
    result
}

#[tauri::command]
pub async fn upload_prepare(
    state: State<'_, AppState>,
    selection_id: String,
    connection_id: Uuid,
    bucket: String,
    prefix: String,
) -> CmdResult<UploadPlan> {
    let mut act = state
        .activity
        .start_action("upload_prepare")
        .connection(connection_id)
        .name("bucket", &bucket)
        .name("destination", &prefix);
    let result = upload_prepare_inner(&state, selection_id, connection_id, bucket, prefix).await;
    if let Ok(p) = &result {
        act.set("files", p.files);
        act.set("folders", p.folders);
        act.set("bytes", p.bytes);
        act.set("conflicts", p.conflicts);
        act.set("skipped", p.skipped.len());
        act.set("too_many", p.too_many);
    }
    act.end(&result);
    result
}

#[tauri::command]
pub async fn upload_start(
    state: State<'_, AppState>,
    plan_id: String,
    policy: ConflictPolicy,
    compress: Option<bool>,
) -> CmdResult<u32> {
    // `compress` is the review dialog's switch; it defaults to on.
    let compress = compress.unwrap_or(true);
    let mut act = state
        .activity
        .start_action("upload_start")
        .field("policy", format!("{policy:?}"))
        .field("compress", compress);
    let result = upload_start_inner(&state, plan_id, policy, compress).await;
    if let Ok(n) = &result {
        act.set("files", *n);
    }
    act.end(&result);
    result
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
    let mut act = state
        .activity
        .start_action("download_start")
        .connection(connection_id)
        .name("bucket", &bucket)
        .field("files_selected", keys.len())
        .field("folders_selected", prefixes.len());
    let result = download_start_inner(app, &state, connection_id, bucket, keys, prefixes).await;
    if let Ok(started) = &result {
        act.set("files", started.as_ref().map_or(0, |s| s.jobs));
        act.set("cancelled", started.is_none());
    }
    act.end(&result);
    result
}

/// Record a transfer reaching a state worth keeping in the activity log.
fn record_job(state: &AppState, job: &r2dock_transfer::JobSnapshot) {
    let action = match job.state {
        JobState::Completed => "transfer_completed",
        JobState::Failed => "transfer_failed",
        JobState::Cancelled => "transfer_cancelled",
        JobState::Skipped => "transfer_skipped",
        JobState::NeedsDecision => "transfer_needs_decision",
        JobState::Paused => "transfer_paused",
        JobState::Retrying => "transfer_retrying",
        JobState::Interrupted => "transfer_interrupted",
        _ => return,
    };
    let act = state
        .activity
        .start_action(action)
        .connection(job.connection_id)
        .field("job", job.id.simple().to_string()[..8].to_string())
        .field("direction", format!("{:?}", job.direction))
        .name("key", &job.key)
        .field("bytes", job.total_bytes)
        .field("transferred", job.transferred_bytes)
        .field("attempt", job.attempt);
    let act = match (job.parts_done, job.parts_total) {
        (Some(d), Some(t)) => act.field("parts", format!("{d}/{t}")),
        _ => act,
    };
    match &job.error {
        Some(e) if job.state != JobState::Completed => act.end::<()>(&Err(e.clone())),
        _ => act.end::<()>(&Ok(())),
    }
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
    let sink_app = app.clone();
    engine.subscribe(Arc::new(move |event: TransferEvent| {
        if let TransferEvent::Updated { job } = &event {
            if let Some(state) = sink_app.try_state::<AppState>() {
                record_job(&state, job);
                state.connections.note_transfer(job);
            }
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
    let act = state
        .activity
        .start_action("transfer_control")
        .field("job", id.simple().to_string()[..8].to_string())
        .field("control", format!("{action:?}"));
    let result = state.engine.control(id, action);
    act.end(&result);
    result
}

#[tauri::command]
pub fn transfer_control_many(state: State<'_, AppState>, ids: Vec<Uuid>, action: JobAction) {
    state
        .activity
        .start_action("transfer_control_many")
        .field("jobs", ids.len())
        .field("control", format!("{action:?}"))
        .end::<()>(&Ok(()));
    state.engine.control_many(&ids, action);
}

#[tauri::command]
pub fn transfer_pause_all(state: State<'_, AppState>) {
    state
        .activity
        .start_action("transfer_pause_all")
        .end::<()>(&Ok(()));
    state.engine.pause_all();
}

#[tauri::command]
pub fn transfer_clear_finished(state: State<'_, AppState>) {
    state
        .activity
        .start_action("transfer_clear_finished")
        .end::<()>(&Ok(()));
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
