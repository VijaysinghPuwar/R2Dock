//! Compression before upload (docs/plans/media-compression.md).
//!
//! Compression runs beside the upload scheduler, not inside it: a job that
//! waits for or runs an encode holds no upload slot, so file 2 compresses
//! while file 1 uploads. Encodes are limited separately (default 1).
//!
//! `Queued (needs compression)` -> `Compressing` -> `Queued` -> upload.
//! The compressed copy lives in R2Dock's work folder and replaces the
//! original in the upload spec; the journal keeps it across restarts.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use r2dock_core::keys::{base_name, display_name};
use r2dock_core::{AppError, ErrorCode};
use r2dock_media::ffmpeg::Ffmpeg;
use r2dock_media::pipeline::{self, PrepareJob, Prepared, Unchanged};

use super::Inner;
use crate::ctx::{JobHooks, RunControl, Stop};
use crate::model::{
    CompressedCopy, CompressionFailure, CompressionResult, CompressionView, JobId, JobSpec,
    JobState, TransferEvent, UploadSpec,
};
use crate::source::SourceFile;

/// How often compression progress is sent.
const PROGRESS_EVERY: Duration = Duration::from_millis(250);

/// What compression needs from the app: the verified FFmpeg and a work folder.
#[derive(Debug, Clone)]
pub struct MediaRuntime {
    pub ffmpeg: Ffmpeg,
    /// R2Dock's own folder for compressed copies (inside the app cache).
    pub work_dir: PathBuf,
    /// Encodes at the same time (1 to 4).
    pub encode_limit: usize,
}

/// Compression milestones, for the activity log.
#[derive(Debug, Clone, PartialEq)]
pub enum CompressionEvent {
    Started {
        id: JobId,
        key: String,
    },
    Finished {
        id: JobId,
        key: String,
        original_bytes: u64,
        bytes: u64,
        preset: String,
        secs: f64,
    },
    Unchanged {
        id: JobId,
        key: String,
        reason: Unchanged,
    },
    Failed {
        id: JobId,
        key: String,
        message: String,
    },
}

pub type CompressionObserver = Arc<dyn Fn(CompressionEvent) + Send + Sync>;

impl Inner {
    fn observe(&self, event: CompressionEvent) {
        let observer = self.compression_observer.read().clone();
        if let Some(f) = observer {
            f(event);
        }
    }

    /// Start encodes for queued uploads that need compression, up to the
    /// encode limit. Without tools, those uploads continue with the original.
    pub(super) fn schedule_compression(self: &Arc<Self>) {
        let media = self.media.read().clone();
        let mut to_start = Vec::new();
        let mut unchanged = Vec::new();
        {
            let mut guard = self.jobs.lock();
            let super::Jobs { map, queue, .. } = &mut *guard;
            let mut running = map
                .values()
                .filter(|e| e.snap.state == JobState::Compressing)
                .count();
            let waiting: Vec<JobId> = queue
                .iter()
                .copied()
                .filter(|id| {
                    map.get(id).is_some_and(|e| {
                        e.snap.state == JobState::Queued
                            && matches!(&e.spec, JobSpec::Upload(u) if u.needs_compression())
                    })
                })
                .collect();
            for id in waiting {
                let Some(e) = map.get_mut(&id) else { continue };
                let JobSpec::Upload(u) = &mut e.spec else {
                    continue;
                };
                let Some(media) = &media else {
                    u.compressed = Some(CompressionResult::Unchanged {
                        reason: Unchanged::ToolsMissing,
                    });
                    e.snap.compression = CompressionView::of(u);
                    unchanged.push((id, u.key.clone(), e.snap.clone()));
                    continue;
                };
                if running >= media.encode_limit.max(1) {
                    break;
                }
                running += 1;
                queue.retain(|x| *x != id);
                e.snap.state = JobState::Compressing;
                e.snap.error = None;
                e.snap.pausable = true;
                e.snap.compression = Some(CompressionView::Pending {
                    fraction: Some(0.0),
                });
                let control = Arc::new(RunControl::new());
                e.control = Some(control.clone());
                to_start.push((id, control, u.clone(), media.clone(), e.snap.clone()));
            }
        }
        for (id, key, snap) in unchanged {
            self.emit(TransferEvent::Updated {
                job: Box::new(snap),
            });
            self.observe(CompressionEvent::Unchanged {
                id,
                key,
                reason: Unchanged::ToolsMissing,
            });
        }
        for (id, control, spec, media, snap) in to_start {
            self.emit(TransferEvent::Updated {
                job: Box::new(snap),
            });
            tokio::spawn(compress_job(self.clone(), id, control, spec, media));
        }
    }

    /// Put a job back in the upload queue (at the front, keeping its turn).
    fn requeue_front(&self, id: JobId) {
        let snap = {
            let mut jobs = self.jobs.lock();
            let snap = jobs.map.get_mut(&id).map(|e| {
                e.snap.state = JobState::Queued;
                e.snap.finished_at = None;
                e.snap.clone()
            });
            if snap.is_some() && !jobs.queue.contains(&id) {
                jobs.queue.push_front(id);
            }
            snap
        };
        if let Some(job) = snap {
            self.emit(TransferEvent::Updated { job: Box::new(job) });
        }
    }

    /// Record the compression result on the job and refresh its snapshot.
    fn set_result(&self, id: JobId, update: impl FnOnce(&mut UploadSpec)) {
        let threshold = *self.threshold.lock();
        let mut jobs = self.jobs.lock();
        let Some(e) = jobs.map.get_mut(&id) else {
            return;
        };
        let JobSpec::Upload(u) = &mut e.spec else {
            return;
        };
        update(u);
        e.snap.compression = CompressionView::of(u);
        e.snap.key = u.key.clone();
        e.snap.name = display_name(base_name(&u.key));
        e.snap.total_bytes = u.size;
        e.snap.pausable = r2dock_core::parts::use_multipart(u.size, threshold);
    }

    /// Delete the compressed copy of a job. `forget` also clears the result,
    /// so a retry compresses again.
    pub(super) fn drop_compressed(&self, id: JobId, forget: bool) {
        let temp = {
            let mut jobs = self.jobs.lock();
            let Some(e) = jobs.map.get_mut(&id) else {
                return;
            };
            let JobSpec::Upload(u) = &mut e.spec else {
                return;
            };
            let temp = u.compressed_copy().map(|c| c.temp.clone());
            if forget {
                restore_original(u);
                e.snap.compression = CompressionView::of(u);
            }
            temp
        };
        if let Some(t) = temp {
            let _ = std::fs::remove_file(t);
        }
    }

    /// Delete files in the work folder that no job refers to (left by a
    /// crash). Blocking.
    pub(super) fn sweep_work_dir(&self, work_dir: &std::path::Path) {
        let keep: std::collections::HashSet<PathBuf> = {
            let jobs = self.jobs.lock();
            jobs.map
                .values()
                .filter_map(|e| match &e.spec {
                    JobSpec::Upload(u) => u.compressed_copy().map(|c| c.temp.clone()),
                    JobSpec::Download(_) => None,
                })
                .collect()
        };
        let Ok(entries) = std::fs::read_dir(work_dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if entry.file_type().is_ok_and(|t| t.is_file()) && !keep.contains(&path) {
                let _ = std::fs::remove_file(path);
            }
        }
    }
}

/// Undo compression in a spec: upload the original again.
pub(super) fn restore_original(u: &mut UploadSpec) {
    if let Some(CompressionResult::Compressed(c)) = u.compressed.take() {
        u.local = c.original_local;
        u.key = c.original_key;
        u.size = c.original_size;
    }
    u.compressed = None;
}

/// Whether the original of a compressed copy changed since it was
/// compressed. Blocking.
pub(super) fn original_changed(c: &CompressedCopy) -> bool {
    match SourceFile::open(&c.original_local, None) {
        Ok(s) => s.fingerprint != c.original || !c.temp.is_file(),
        Err(_) => true,
    }
}

fn failure_error(message: &str) -> AppError {
    AppError::new(
        ErrorCode::Internal,
        format!("Compression failed: {message}"),
    )
}

async fn compress_job(
    inner: Arc<Inner>,
    id: JobId,
    control: Arc<RunControl>,
    spec: UploadSpec,
    media: MediaRuntime,
) {
    let started = Instant::now();
    let Some(request) = spec.compression.clone() else {
        return;
    };
    inner.observe(CompressionEvent::Started {
        id,
        key: spec.key.clone(),
    });

    // Open the original the safe way (no symlinks, same file as scanned).
    let local = spec.local.clone();
    let identity = spec.identity;
    let source = tokio::task::spawn_blocking(move || SourceFile::open(&local, identity)).await;
    let source = match source {
        Ok(Ok(s)) => s,
        Ok(Err(e)) => {
            inner.finish(id, JobState::Failed, Some(e.app));
            return;
        }
        Err(_) => {
            inner.finish(
                id,
                JobState::Failed,
                Some(failure_error("a background task stopped")),
            );
            return;
        }
    };
    let fingerprint = source.fingerprint;

    let stem = id.simple().to_string();
    let progress_inner = inner.clone();
    let mut last = Instant::now() - PROGRESS_EVERY;
    let on_fraction = move |f: f64| {
        if last.elapsed() < PROGRESS_EVERY {
            return;
        }
        last = Instant::now();
        let snap = {
            let mut jobs = progress_inner.jobs.lock();
            jobs.map.get_mut(&id).map(|e| {
                e.snap.compression = Some(CompressionView::Pending { fraction: Some(f) });
                e.snap.clone()
            })
        };
        if let Some(job) = snap {
            progress_inner.emit(TransferEvent::Updated { job: Box::new(job) });
        }
    };
    let result = pipeline::prepare(
        &media.ffmpeg,
        PrepareJob {
            input: &spec.local,
            work_dir: &media.work_dir,
            stem: &stem,
            settings: &request.settings,
            video: &request.video,
            content: request.content,
        },
        &control.cancel,
        on_fraction,
    )
    .await;

    match result {
        Ok(Prepared::Compressed(c)) => {
            // The original must not have changed while it was compressed.
            let check = source;
            let unchanged = tokio::task::spawn_blocking(move || check.ensure_unchanged().is_ok())
                .await
                .unwrap_or(false);
            if !unchanged {
                let _ = std::fs::remove_file(&c.output);
                inner.requeue_front(id);
                inner.wake.notify_one();
                return;
            }
            let key = pipeline::upload_key(&spec.key, &c);
            let copy = CompressedCopy {
                temp: c.output.clone(),
                original_local: spec.local.clone(),
                original_key: spec.key.clone(),
                original_size: c.original_size,
                original: fingerprint,
                preset: c.preset.clone(),
            };
            inner.set_result(id, |u| {
                u.local = c.output.clone();
                u.key = key.clone();
                u.size = c.size;
                u.identity = None;
                u.compressed = Some(CompressionResult::Compressed(copy));
            });
            inner.observe(CompressionEvent::Finished {
                id,
                key,
                original_bytes: c.original_size,
                bytes: c.size,
                preset: c.preset,
                secs: started.elapsed().as_secs_f64(),
            });
        }
        Ok(Prepared::Unchanged(reason)) => {
            inner.set_result(id, |u| {
                u.compressed = Some(CompressionResult::Unchanged {
                    reason: reason.clone(),
                });
            });
            inner.observe(CompressionEvent::Unchanged {
                id,
                key: spec.key.clone(),
                reason,
            });
        }
        Err(e) if e.is_cancelled() => {
            match control.stop() {
                Stop::Pause => {
                    inner.set_result(id, |_| {});
                    inner.set_state(id, JobState::Paused, None);
                }
                _ => {
                    inner.set_result(id, |_| {});
                    inner.finish(id, JobState::Cancelled, None);
                }
            }
            clear_control(&inner, id, &control);
            inner.wake.notify_one();
            return;
        }
        Err(e) => {
            let message = e.to_string();
            inner.set_result(id, |u| {
                u.compressed = Some(CompressionResult::Failed {
                    message: message.clone(),
                });
            });
            inner.observe(CompressionEvent::Failed {
                id,
                key: spec.key.clone(),
                message: message.clone(),
            });
            if request.on_failure == CompressionFailure::SkipFile {
                inner.finish(id, JobState::Failed, Some(failure_error(&message)));
                clear_control(&inner, id, &control);
                inner.wake.notify_one();
                return;
            }
        }
    }
    clear_control(&inner, id, &control);
    super::Hooks(inner.clone()).checkpoint(id);
    inner.requeue_front(id);
    inner.wake.notify_one();
}

fn clear_control(inner: &Inner, id: JobId, control: &Arc<RunControl>) {
    if let Some(e) = inner.jobs.lock().map.get_mut(&id)
        && e.control.as_ref().is_some_and(|c| Arc::ptr_eq(c, control))
    {
        e.control = None;
    }
}
