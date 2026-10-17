//! Downloads streamed to a hidden temp file, resumed with Range + If-Match,
//! verified, then moved into place without overwriting (PLAN.md 8.5).

use std::io;
use std::path::{Path, PathBuf};

use r2dock_core::etag::{is_single_part_md5, normalize};
use r2dock_core::path_guard::resolve_file_under_root;
use r2dock_core::policy::{ConflictPolicy, first_free_name};
use r2dock_core::{AppError, ErrorCode};
use r2dock_r2::R2Error;
use tokio::io::AsyncWriteExt;

use crate::ctx::{DownloadState, JobCtx, Outcome, ResumeState, Stop};
use crate::model::{DownloadSpec, JobState};
use crate::retry::JobError;

const KEEP_BOTH_LIMIT: u32 = 1000;

async fn blocking<T, F>(f: F) -> Result<T, JobError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, JobError> + Send + 'static,
{
    tokio::task::spawn_blocking(f).await.map_err(|_| {
        JobError::fatal(AppError::internal(
            "A background task stopped unexpectedly.",
        ))
    })?
}

fn guard_error(err: r2dock_core::path_guard::PathGuardError) -> JobError {
    JobError::fatal(AppError::new(ErrorCode::InvalidInput, err.to_string()))
}

/// Hidden temp file beside the target, unique per job so two downloads (or an
/// unrelated file) never share it.
fn temp_path_for(final_path: &Path, job: crate::model::JobId) -> PathBuf {
    let name = final_path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tag = &job.simple().to_string()[..8];
    final_path.with_file_name(format!(".{name}.{tag}.r2dock-part"))
}

/// Re-check that a saved destination is still inside the root without symlinks.
fn revalidate(spec: &DownloadSpec, final_path: &Path) -> bool {
    resolve_file_under_root(&spec.dest_root, &spec.rel_path).is_ok_and(|p| p == final_path)
}

fn exists_no_follow(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok()
}

/// True when `dir` is the (validated) parent folder for this spec's target.
fn revalidate_dir(spec: &DownloadSpec, dir: &Path) -> bool {
    let parent = spec.rel_path.parent().unwrap_or_else(|| Path::new(""));
    r2dock_core::path_guard::ensure_dir_under_root(&spec.dest_root, parent).is_ok_and(|d| d == dir)
}

/// Bytes free on the volume holding `dir`, when the OS reports it.
fn free_space(dir: &Path) -> Option<u64> {
    fs4::available_space(dir).ok()
}

/// Remove a file, ignoring "not found".
fn remove_quietly(path: &Path) {
    if let Err(e) = std::fs::remove_file(path)
        && e.kind() != io::ErrorKind::NotFound
    {
        tracing::warn!("could not remove a partial download: {:?}", e.kind());
    }
}

/// Decide the final path, applying the conflict policy. `Err(outcome)` ends the run.
fn resolve_target(spec: &mut DownloadSpec) -> Result<Result<PathBuf, Outcome>, JobError> {
    let target = resolve_file_under_root(&spec.dest_root, &spec.rel_path).map_err(guard_error)?;
    if !exists_no_follow(&target) {
        return Ok(Ok(target));
    }
    match spec.policy {
        ConflictPolicy::Replace => Ok(Ok(target)),
        ConflictPolicy::Skip => Ok(Err(Outcome::Skipped)),
        ConflictPolicy::Ask => Ok(Err(Outcome::NeedsDecision)),
        ConflictPolicy::KeepBoth => {
            let dir = target.parent().map(Path::to_path_buf).unwrap_or_default();
            let name = target
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let free = first_free_name(&name, KEEP_BOTH_LIMIT, |n| exists_no_follow(&dir.join(n)))
                .ok_or_else(|| {
                    JobError::fatal(AppError::new(
                        ErrorCode::AlreadyExists,
                        "Too many files with this name already exist.",
                    ))
                })?;
            spec.rel_path = spec.rel_path.with_file_name(&free);
            Ok(Ok(dir.join(free)))
        }
    }
}

pub async fn run(ctx: &JobCtx, spec: &mut DownloadSpec) -> Result<Outcome, JobError> {
    ctx.set_state(JobState::Preparing);

    // Resume a partial download, or pick a fresh target.
    let saved = ctx.resume.lock().clone();
    let saved = match saved {
        ResumeState::Download(d) => {
            let check = spec.clone();
            let fp = d.final_path.clone();
            let valid = d.temp == temp_path_for(&d.final_path, ctx.id)
                && blocking(move || Ok(revalidate(&check, &fp))).await?;
            if valid {
                ResumeState::Download(d)
            } else {
                // The destination changed (for example a folder became a
                // symlink): drop the partial and start over safely.
                if d.temp.parent().is_some_and(|p| revalidate_dir(spec, p)) {
                    remove_quietly(&d.temp);
                }
                ctx.clear_resume();
                ResumeState::None
            }
        }
        other => other,
    };
    let (final_path, mut etag, mut offset) = match saved {
        ResumeState::Download(DownloadState {
            etag,
            temp,
            final_path,
        }) => {
            let len = std::fs::symlink_metadata(&temp)
                .ok()
                .filter(|m| m.is_file())
                .map_or(0, |m| m.len());
            (final_path, etag, len)
        }
        _ => {
            let before = spec.rel_path.clone();
            let mut working = spec.clone();
            let resolved = blocking(move || {
                let r = resolve_target(&mut working)?;
                Ok((r, working))
            })
            .await?;
            let (result, working) = resolved;
            *spec = working;
            let path = match result {
                Ok(p) => p,
                Err(outcome) => return Ok(outcome),
            };
            if spec.rel_path != before {
                ctx.hooks.renamed(ctx.id, None, Some(spec.rel_path.clone()));
            }
            (path, None, 0)
        }
    };
    let temp = temp_path_for(&final_path, ctx.id);
    if let Ok(meta) = std::fs::symlink_metadata(&temp)
        && !meta.is_file()
    {
        return Err(guard_error(
            r2dock_core::path_guard::PathGuardError::SymlinkInPath,
        ));
    }
    if etag.is_none() || offset > spec.size {
        offset = 0;
    }

    // Free space check before transferring.
    if let Some(dir) = final_path.parent()
        && let Some(free) = free_space(dir)
    {
        let needed = spec.size.saturating_sub(offset);
        if free < needed {
            return Err(JobError::fatal(AppError::new(
                ErrorCode::DiskFull,
                format!("Not enough disk space (needs {}).", human_bytes(needed)),
            )));
        }
    }

    // Request, restarting from zero if the object changed since the partial download.
    let mut response = None;
    for _ in 0..2 {
        let mut req = ctx
            .client
            .sdk()
            .get_object()
            .bucket(&spec.bucket)
            .key(&spec.key);
        if offset > 0 {
            req = req.range(format!("bytes={offset}-"));
            if let Some(e) = &etag {
                req = req.if_match(e);
            }
        }
        let sent = tokio::select! {
            () = ctx.control.cancel.cancelled() => return stopped(ctx, &temp),
            r = req.send() => r.map_err(R2Error::from_sdk),
        };
        match sent {
            Ok(out) => {
                response = Some(out);
                break;
            }
            Err(R2Error::PreconditionFailed { .. }) | Err(R2Error::Service { status: 416, .. })
                if offset > 0 =>
            {
                offset = 0;
                etag = None;
            }
            Err(e) => return Err(e.into()),
        }
    }
    let out = response.ok_or_else(|| {
        JobError::retryable(AppError::new(
            ErrorCode::ServerError,
            "R2 did not return the file.",
        ))
    })?;

    let remote_etag = out.e_tag().map(str::to_string);
    let sse_c = out.sse_customer_algorithm().is_some();
    let body_len = u64::try_from(out.content_length().unwrap_or(0)).unwrap_or(0);
    let total = offset + body_len;
    if total != spec.size {
        spec.size = total;
        ctx.hooks.total(ctx.id, total);
    }
    // Reset or reopen the partial file first, then record what it holds.
    let mut file = open_temp(&temp, offset > 0).await?;
    ctx.save_resume(ResumeState::Download(DownloadState {
        etag: remote_etag.clone(),
        temp: temp.clone(),
        final_path: final_path.clone(),
    }));
    ctx.counter.reset_in_flight();
    ctx.counter.set_committed(offset);
    ctx.set_state(JobState::Running);

    let mut body = out.body;
    let mut written = offset;
    loop {
        let chunk = tokio::select! {
            () = ctx.control.cancel.cancelled() => {
                let _ = file.flush().await;
                drop(file);
                return stopped(ctx, &temp);
            }
            c = body.next() => c,
        };
        match chunk {
            None => break,
            Some(Ok(bytes)) => {
                file.write_all(&bytes)
                    .await
                    .map_err(|e| JobError::io(&e, "save the download"))?;
                written += bytes.len() as u64;
                ctx.counter.set_committed(written);
            }
            Some(Err(_)) => {
                let _ = file.flush().await;
                return Err(JobError::retryable(AppError::new(
                    ErrorCode::Network,
                    "Connection lost during download.",
                )));
            }
        }
    }
    file.flush()
        .await
        .map_err(|e| JobError::io(&e, "save the download"))?;
    file.sync_all()
        .await
        .map_err(|e| JobError::io(&e, "save the download"))?;
    drop(file);

    ctx.set_state(JobState::Verifying);
    if written != total {
        return Err(JobError::retryable(AppError::new(
            ErrorCode::Network,
            "Download ended early.",
        )));
    }
    if let Some(e) = remote_etag.as_deref()
        && !sse_c
        && is_single_part_md5(e)
    {
        let expected = normalize(e);
        let check = temp.clone();
        let actual = blocking(move || md5_file(&check)).await?;
        if actual != expected {
            remove_quietly(&temp);
            ctx.clear_resume();
            return Err(JobError::retryable(AppError::new(
                ErrorCode::ServerError,
                "Downloaded data did not match the file in R2.",
            )));
        }
    }

    if ctx.control.stop() != Stop::None {
        return stopped(ctx, &temp);
    }
    let policy = spec.policy;
    let temp_c = temp.clone();
    let final_c = final_path.clone();
    let check = spec.clone();
    let placed = blocking(move || {
        if !revalidate(&check, &final_c) {
            return Err(guard_error(
                r2dock_core::path_guard::PathGuardError::SymlinkInPath,
            ));
        }
        place(&temp_c, &final_c, policy)
    })
    .await?;
    match placed {
        Placed::Done(path) => {
            if path != final_path
                && let (Some(name), Some(parent)) = (path.file_name(), spec.rel_path.parent())
            {
                spec.rel_path = parent.join(name);
                ctx.hooks.renamed(ctx.id, None, Some(spec.rel_path.clone()));
            }
            ctx.clear_resume();
            Ok(Outcome::Completed)
        }
        Placed::Skipped => {
            remove_quietly(&temp);
            ctx.clear_resume();
            Ok(Outcome::Skipped)
        }
        Placed::Ask => Ok(Outcome::NeedsDecision),
    }
}

fn stopped(ctx: &JobCtx, temp: &Path) -> Result<Outcome, JobError> {
    match ctx.control.stop() {
        Stop::Pause => Ok(Outcome::Paused),
        _ => {
            remove_quietly(temp);
            ctx.clear_resume();
            Err(JobError::cancelled())
        }
    }
}

async fn open_temp(temp: &Path, append: bool) -> Result<tokio::fs::File, JobError> {
    let mut opts = tokio::fs::OpenOptions::new();
    opts.write(true).create(true);
    if append {
        opts.append(true);
    } else {
        opts.truncate(true);
    }
    #[cfg(unix)]
    opts.custom_flags(libc::O_NOFOLLOW);
    opts.open(temp)
        .await
        .map_err(|e| JobError::io(&e, "save the download"))
}

fn md5_file(path: &Path) -> Result<String, JobError> {
    use md5::{Digest, Md5};
    use std::io::Read;
    let mut f = std::fs::File::open(path).map_err(|e| JobError::io(&e, "check the download"))?;
    let mut h = Md5::new();
    let mut buf = vec![0u8; 1024 * 1024];
    loop {
        let n = f
            .read(&mut buf)
            .map_err(|e| JobError::io(&e, "check the download"))?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(hex::encode(h.finalize()))
}

enum Placed {
    Done(PathBuf),
    Skipped,
    Ask,
}

/// Move the finished temp file into place. Never overwrites unless the policy is Replace.
fn place(temp: &Path, target: &Path, policy: ConflictPolicy) -> Result<Placed, JobError> {
    if policy == ConflictPolicy::Replace {
        if let Ok(meta) = std::fs::symlink_metadata(target)
            && (meta.file_type().is_symlink() || meta.is_dir())
        {
            return Err(guard_error(
                r2dock_core::path_guard::PathGuardError::SymlinkInPath,
            ));
        }
        std::fs::rename(temp, target).map_err(|e| JobError::io(&e, "save the download"))?;
        return Ok(Placed::Done(target.to_path_buf()));
    }
    let mut candidate = target.to_path_buf();
    let dir = target.parent().map(Path::to_path_buf).unwrap_or_default();
    let name = target
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    for n in 0..=KEEP_BOTH_LIMIT {
        if n > 0 {
            candidate = dir.join(r2dock_core::keys::keep_both_name(&name, n));
        }
        let tmp = tempfile::TempPath::try_from_path(temp)
            .map_err(|e| JobError::io(&e, "save the download"))?;
        match tmp.persist_noclobber(&candidate) {
            Ok(()) => return Ok(Placed::Done(candidate)),
            Err(err) => {
                // Keep the temp file: we still need it.
                let _ = err.path.keep();
                if err.error.kind() != io::ErrorKind::AlreadyExists {
                    return Err(JobError::io(&err.error, "save the download"));
                }
                match policy {
                    ConflictPolicy::Skip => return Ok(Placed::Skipped),
                    ConflictPolicy::Ask => return Ok(Placed::Ask),
                    _ => {}
                }
            }
        }
    }
    Err(JobError::fatal(AppError::new(
        ErrorCode::AlreadyExists,
        "Too many files with this name already exist.",
    )))
}

/// Short human size for messages, e.g. "2.4 GB".
pub fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1000.0 && unit < UNITS.len() - 1 {
        value /= 1000.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn temp_name_is_hidden_unique_and_beside_target() {
        let id = uuid::Uuid::parse_str("0123abcd-0000-0000-0000-000000000000").unwrap();
        let t = temp_path_for(Path::new("/x/y/report.pdf"), id);
        assert_eq!(t, PathBuf::from("/x/y/.report.pdf.0123abcd.r2dock-part"));
        assert_ne!(
            t,
            temp_path_for(Path::new("/x/y/report.pdf"), uuid::Uuid::new_v4())
        );
    }

    #[test]
    fn human_sizes() {
        assert_eq!(human_bytes(999), "999 B");
        assert_eq!(human_bytes(2_400_000_000), "2.4 GB");
    }

    #[test]
    fn place_never_overwrites_without_replace() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("a.txt");
        std::fs::write(&target, b"original").unwrap();
        let temp = dir.path().join(".a.txt.r2dock-part");

        std::fs::write(&temp, b"new").unwrap();
        assert!(matches!(
            place(&temp, &target, ConflictPolicy::Skip).unwrap(),
            Placed::Skipped
        ));
        assert_eq!(std::fs::read(&target).unwrap(), b"original");

        let Placed::Done(p) = place(&temp, &target, ConflictPolicy::KeepBoth).unwrap() else {
            panic!()
        };
        assert_eq!(p, dir.path().join("a (1).txt"));
        assert_eq!(std::fs::read(&p).unwrap(), b"new");
        assert_eq!(std::fs::read(&target).unwrap(), b"original");

        std::fs::write(&temp, b"replace").unwrap();
        let Placed::Done(p) = place(&temp, &target, ConflictPolicy::Replace).unwrap() else {
            panic!()
        };
        assert_eq!(p, target);
        assert_eq!(std::fs::read(&target).unwrap(), b"replace");
        assert!(!temp.exists());
    }

    #[test]
    fn keep_both_target_resolution() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), b"x").unwrap();
        let mut spec = DownloadSpec {
            connection: uuid::Uuid::nil(),
            bucket: "b".into(),
            key: "a.txt".into(),
            size: 1,
            dest_root: dir.path().to_path_buf(),
            rel_path: PathBuf::from("a.txt"),
            policy: ConflictPolicy::KeepBoth,
        };
        let p = resolve_target(&mut spec).unwrap().unwrap();
        assert!(p.ends_with("a (1).txt"));
        assert_eq!(spec.rel_path, PathBuf::from("a (1).txt"));
        spec.policy = ConflictPolicy::Ask;
        spec.rel_path = PathBuf::from("a.txt");
        assert_eq!(
            resolve_target(&mut spec).unwrap(),
            Err(Outcome::NeedsDecision)
        );
    }
}
