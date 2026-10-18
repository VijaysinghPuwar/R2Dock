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
use r2dock_media::ffmpeg::{EncodeError, Ffmpeg};
use r2dock_media::pipeline::{self, PrepareJob, Prepared, Unchanged};

use super::Inner;
use crate::ctx::{JobHooks, RunControl, Stop};
use crate::model::{
    CompressedCopy, CompressionFailure, CompressionResult, CompressionView, CopyDigest, JobId,
    JobSpec, JobState, TransferEvent, UploadSpec,
};
use crate::source::{SourceFile, digest_written};

/// How often compression progress is sent.
const PROGRESS_EVERY: Duration = Duration::from_millis(250);

/// How many times a file that changed during its encode is encoded again.
const CHANGED_RETRIES: u32 = 2;

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
        // The identity was the copy's; the original's was not kept.
        u.identity = None;
    }
    u.compressed = None;
}

/// Whether a compressed copy can no longer be uploaded: its original
/// changed since it was compressed, or the copy is not the exact file that
/// passed verification (missing, replaced, truncated, or altered, even
/// without a size change). A copy saved without a digest is not trusted.
/// Blocking: reads the whole copy.
pub(super) fn copy_unusable(c: &CompressedCopy) -> bool {
    let original_ok =
        SourceFile::open(&c.original_local, None).is_ok_and(|s| s.fingerprint == c.original);
    if !original_ok {
        return true;
    }
    let Some(want) = &c.output else {
        return true;
    };
    if !want.file.matches(&c.temp) {
        return true;
    }
    !digest_written(&c.temp).is_ok_and(|(file, sha256)| file == want.file && sha256 == want.sha256)
}

fn failure_error(message: &str) -> AppError {
    AppError::new(
        ErrorCode::Internal,
        format!("Compression failed: {message}"),
    )
}

/// How a compression run ended, for [`compress_job`]'s single exit path.
enum End {
    /// The job reached its own state (failed, cancelled, paused).
    Settled,
    /// Back to the upload queue, with or without a copy.
    Upload,
}

/// Runs on every exit of [`compress_job`], including a panic: releases the
/// run's control (only if no newer run replaced it) and wakes the
/// scheduler, so the next job can take the encode slot. A job still shown
/// as compressing at that point ended abnormally and is marked failed.
struct Finalize {
    inner: Arc<Inner>,
    id: JobId,
    control: Arc<RunControl>,
}

impl Drop for Finalize {
    fn drop(&mut self) {
        let stuck = {
            let mut jobs = self.inner.jobs.lock();
            match jobs.map.get_mut(&self.id) {
                Some(e)
                    if e.control
                        .as_ref()
                        .is_some_and(|c| Arc::ptr_eq(c, &self.control)) =>
                {
                    e.control = None;
                    e.snap.state == JobState::Compressing
                }
                _ => false,
            }
        };
        if stuck {
            self.inner.finish(
                self.id,
                JobState::Failed,
                Some(failure_error("a background task stopped")),
            );
        }
        self.inner.wake.notify_one();
    }
}

async fn compress_job(
    inner: Arc<Inner>,
    id: JobId,
    control: Arc<RunControl>,
    spec: UploadSpec,
    media: MediaRuntime,
) {
    let _finalize = Finalize {
        inner: inner.clone(),
        id,
        control: control.clone(),
    };
    if let End::Upload = compress(&inner, id, &control, spec, media).await {
        clear_control(&inner, id, &control);
        super::Hooks(inner.clone()).checkpoint(id);
        inner.requeue_front(id);
    }
}

async fn compress(
    inner: &Arc<Inner>,
    id: JobId,
    control: &Arc<RunControl>,
    spec: UploadSpec,
    media: MediaRuntime,
) -> End {
    let started = Instant::now();
    let Some(request) = spec.compression.clone() else {
        return End::Upload;
    };
    inner.observe(CompressionEvent::Started {
        id,
        key: spec.key.clone(),
    });

    let stem = id.simple().to_string();
    let progress_inner = inner.clone();
    let mut last = Instant::now() - PROGRESS_EVERY;
    let mut on_fraction = move |f: f64| {
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

    // A file that changes while it is compressed is compressed again, a
    // limited number of times; one that keeps changing counts as a failure.
    let mut attempts = 0;
    let (result, fingerprint, digest) = loop {
        // Open the original the safe way (no symlinks, same file as scanned).
        let local = spec.local.clone();
        let identity = spec.identity;
        let source = tokio::task::spawn_blocking(move || SourceFile::open(&local, identity)).await;
        let source = match source {
            Ok(Ok(s)) => s,
            Ok(Err(e)) => {
                inner.finish(id, JobState::Failed, Some(e.app));
                return End::Settled;
            }
            Err(_) => {
                inner.finish(
                    id,
                    JobState::Failed,
                    Some(failure_error("a background task stopped")),
                );
                return End::Settled;
            }
        };
        let fingerprint = source.fingerprint;
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
            &mut on_fraction,
        )
        .await;
        let Ok(Prepared::Compressed(c)) = &result else {
            break (result, fingerprint, None);
        };
        // The original must not have changed while it was compressed, and
        // the verified copy is recorded exactly (size, identity, SHA-256)
        // so a later change to it is noticed before upload.
        let output = c.output.clone();
        let checked = tokio::task::spawn_blocking(move || {
            source
                .ensure_unchanged()
                .ok()
                .map(|()| digest_written(&output))
        })
        .await
        .ok()
        .flatten();
        match checked {
            Some(Ok((file, sha256))) if file.len == c.size => {
                break (result, fingerprint, Some(CopyDigest { file, sha256 }));
            }
            Some(_) => {
                let _ = std::fs::remove_file(&c.output);
                break (
                    Err(EncodeError::Failed(
                        "the compressed copy could not be checked".into(),
                    )),
                    fingerprint,
                    None,
                );
            }
            None => {}
        }
        let _ = std::fs::remove_file(&c.output);
        attempts += 1;
        if attempts > CHANGED_RETRIES {
            break (
                Err(EncodeError::Failed("the file is still changing".into())),
                fingerprint,
                None,
            );
        }
    };

    match result {
        Ok(Prepared::Compressed(c)) => {
            let key = pipeline::upload_key(&spec.key, &c);
            let identity = digest.as_ref().and_then(|d| d.file.identity);
            let copy = CompressedCopy {
                temp: c.output.clone(),
                original_local: spec.local.clone(),
                original_key: spec.key.clone(),
                original_size: c.original_size,
                original: fingerprint,
                preset: c.preset.clone(),
                output: digest,
            };
            inner.set_result(id, |u| {
                u.local = c.output.clone();
                u.key = key.clone();
                u.size = c.size;
                // The upload opens only this exact copy.
                u.identity = identity;
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
            End::Upload
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
            End::Upload
        }
        Err(e) if e.is_cancelled() => {
            inner.set_result(id, |_| {});
            match control.stop() {
                Stop::Pause => inner.set_state(id, JobState::Paused, None),
                _ => inner.finish(id, JobState::Cancelled, None),
            }
            End::Settled
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
                return End::Settled;
            }
            End::Upload
        }
    }
}

fn clear_control(inner: &Inner, id: JobId, control: &Arc<RunControl>) {
    if let Some(e) = inner.jobs.lock().map.get_mut(&id)
        && e.control.as_ref().is_some_and(|c| Arc::ptr_eq(c, control))
    {
        e.control = None;
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};

    use r2dock_core::policy::ConflictPolicy;
    use r2dock_media::plan::AudioSettings;
    use r2dock_r2::R2Client;
    use uuid::Uuid;

    use super::*;
    use crate::engine::{BoxFuture, ClientProvider, Engine, EngineConfig};
    use crate::model::CompressionRequest;

    /// Never reached: the job fails before it uploads.
    struct NoClient;

    impl ClientProvider for NoClient {
        fn client(&self, _connection: Uuid) -> BoxFuture<'_, Result<R2Client, AppError>> {
            Box::pin(async { Err(AppError::internal("no client in this test")) })
        }
    }

    fn ffmpeg_bin() -> Option<PathBuf> {
        let bin = std::env::var_os("R2DOCK_TEST_FFMPEG").map(PathBuf::from)?;
        let ok = std::process::Command::new(&bin)
            .arg("-version")
            .output()
            .is_ok_and(|o| o.status.success());
        ok.then_some(bin)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_file_that_keeps_changing_fails_after_retries() {
        let Some(bin) = ffmpeg_bin() else {
            eprintln!("R2DOCK_TEST_FFMPEG not set; skipping");
            return;
        };
        let data = tempfile::tempdir().unwrap();
        let input = data.path().join("talk.mp3");
        let out = std::process::Command::new(&bin)
            .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi"])
            .args(["-i", "sine=f=220:r=22050:d=600", "-c:a", "libmp3lame"])
            .args(["-b:a", "64k", "-ac", "1"])
            .arg(&input)
            .output()
            .unwrap();
        assert!(out.status.success());

        let engine = Engine::start(
            Arc::new(NoClient),
            EngineConfig::new(data.path().to_path_buf()),
        );
        engine.set_media(Some(MediaRuntime {
            ffmpeg: Ffmpeg::new(bin),
            work_dir: data.path().join("compress"),
            encode_limit: 1,
        }));

        // Something keeps writing to the file the whole time.
        let stop = Arc::new(AtomicBool::new(false));
        let writer = {
            let stop = stop.clone();
            let path = input.clone();
            std::thread::spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    if let Ok(mut f) = std::fs::OpenOptions::new().append(true).open(&path) {
                        let _ = std::io::Write::write_all(&mut f, &[0]);
                    }
                    std::thread::sleep(Duration::from_millis(20));
                }
            })
        };

        let size = std::fs::metadata(&input).unwrap().len();
        let ids = engine.enqueue(
            vec![JobSpec::Upload(UploadSpec {
                connection: Uuid::nil(),
                bucket: "bkt".into(),
                key: "talk.mp3".into(),
                local: input.clone(),
                size,
                policy: ConflictPolicy::Replace,
                identity: None,
                compression: Some(CompressionRequest {
                    settings: AudioSettings::default(),
                    content: Default::default(),
                    on_failure: CompressionFailure::SkipFile,
                    video: Default::default(),
                }),
                compressed: None,
            })],
            None,
        );
        let mut job = None;
        for _ in 0..3000 {
            let j = engine
                .snapshot()
                .into_iter()
                .find(|j| j.id == ids[0])
                .unwrap();
            if j.state == JobState::Failed {
                job = Some(j);
                break;
            }
            assert_ne!(j.state, JobState::Completed);
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        stop.store(true, Ordering::Relaxed);
        writer.join().unwrap();
        let job = job.expect("the job never stopped compressing");
        let message = job.error.unwrap().message;
        assert!(message.contains("still changing"), "{message}");
    }
}
