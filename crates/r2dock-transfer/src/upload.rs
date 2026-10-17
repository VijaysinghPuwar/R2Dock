//! Uploads: single PUT below the threshold, parallel multipart above it
//! (PLAN.md 8.4, 8.10).

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;

use aws_sdk_s3::types::{CompletedMultipartUpload, CompletedPart};
use base64::Engine as _;
use futures_util::StreamExt;
use futures_util::stream::FuturesUnordered;
use r2dock_core::etag::{multipart_etag, normalize};
use r2dock_core::keys::{base_name, join_key, parent_prefix};
use r2dock_core::parts::{PartRange, part_size_for, plan_parts, use_multipart};
use r2dock_core::policy::ConflictPolicy;
use r2dock_r2::R2Error;

use crate::body::{range_body, settle};
use crate::ctx::{JobCtx, MultipartState, Outcome, PartDone, ResumeState, Stop};
use crate::model::{JobState, UploadSpec};
use crate::retry::JobError;
use crate::source::SourceFile;

/// Give up looking for a free "Keep both" name after this many tries.
const KEEP_BOTH_LIMIT: u32 = 100;

fn b64(md5: &[u8; 16]) -> String {
    base64::engine::general_purpose::STANDARD.encode(md5)
}

/// Object metadata for a compressed upload (`x-amz-meta-*`): original size,
/// original file name (percent-encoded, since headers are ASCII), and preset.
pub(crate) fn compression_metadata(spec: &UploadSpec) -> Vec<(String, String)> {
    let Some(c) = spec.compressed_copy() else {
        return Vec::new();
    };
    vec![
        ("r2dock-original-size".into(), c.original_size.to_string()),
        (
            "r2dock-original-name".into(),
            percent_encode(base_name(&c.original_key)),
        ),
        ("r2dock-compression".into(), c.preset.clone()),
    ]
}

fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn content_type(key: &str) -> String {
    mime_guess::from_path(key)
        .first_or_octet_stream()
        .essence_str()
        .to_string()
}

async fn blocking<T, F>(f: F) -> Result<T, JobError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, JobError> + Send + 'static,
{
    tokio::task::spawn_blocking(f).await.map_err(|_| {
        JobError::fatal(r2dock_core::AppError::internal(
            "A background task stopped unexpectedly.",
        ))
    })?
}

/// Apply the conflict policy before uploading. Returns the key to write, or
/// an outcome that ends the run.
async fn resolve_key(ctx: &JobCtx, spec: &mut UploadSpec) -> Result<Result<(), Outcome>, JobError> {
    if spec.policy == ConflictPolicy::Replace {
        return Ok(Ok(()));
    }
    if !ctx.client.exists(&spec.bucket, &spec.key).await? {
        return Ok(Ok(()));
    }
    match spec.policy {
        ConflictPolicy::Skip => Ok(Err(Outcome::Skipped)),
        ConflictPolicy::Ask => Ok(Err(Outcome::NeedsDecision)),
        ConflictPolicy::Replace => Ok(Ok(())),
        ConflictPolicy::KeepBoth => {
            let prefix = parent_prefix(&spec.key).to_string();
            let name = base_name(&spec.key).to_string();
            for n in 1..=KEEP_BOTH_LIMIT {
                let candidate = join_key(&prefix, &r2dock_core::keys::keep_both_name(&name, n));
                if !ctx.client.exists(&spec.bucket, &candidate).await? {
                    spec.key = candidate.clone();
                    ctx.hooks.renamed(ctx.id, Some(candidate), None);
                    return Ok(Ok(()));
                }
            }
            Err(JobError::fatal(r2dock_core::AppError::new(
                r2dock_core::ErrorCode::AlreadyExists,
                "Too many files with this name already exist.",
            )))
        }
    }
}

pub async fn run(ctx: &JobCtx, spec: &mut UploadSpec) -> Result<Outcome, JobError> {
    ctx.set_state(JobState::Preparing);
    let path = spec.local.clone();
    let identity = spec.identity;
    let source = Arc::new(blocking(move || SourceFile::open(&path, identity)).await?);
    let size = source.fingerprint.len;
    if size != spec.size {
        spec.size = size;
        ctx.hooks.total(ctx.id, size);
    }

    // Continue a saved multipart upload when the file is unchanged.
    let saved = ctx.resume.lock().clone();
    if let ResumeState::Multipart(state) = saved {
        if state.fingerprint == source.fingerprint && state.key == spec.key {
            return multipart(ctx, spec, source, Some(state)).await;
        }
        let _ = ctx
            .client
            .abort_multipart(&spec.bucket, &state.key, &state.upload_id)
            .await;
        ctx.clear_resume();
    }

    if let Err(outcome) = resolve_key(ctx, spec).await? {
        return Ok(outcome);
    }
    if use_multipart(size, ctx.multipart_threshold) {
        multipart(ctx, spec, source, None).await
    } else {
        single(ctx, spec, source).await
    }
}

async fn single(
    ctx: &JobCtx,
    spec: &mut UploadSpec,
    source: Arc<SourceFile>,
) -> Result<Outcome, JobError> {
    let size = source.fingerprint.len;
    let hashing = source.clone();
    let md5 = blocking(move || hashing.md5_range(0, size)).await?;
    ctx.counter.set_committed(0);
    ctx.set_state(JobState::Running);

    let sent = Arc::new(AtomicU64::new(0));
    let body = range_body(source.clone(), 0, size, ctx.counter.clone(), sent.clone());
    let mut req = ctx
        .client
        .sdk()
        .put_object()
        .bucket(&spec.bucket)
        .key(&spec.key)
        .content_length(i64::try_from(size).unwrap_or(i64::MAX))
        .content_md5(b64(&md5))
        .content_type(content_type(&spec.key))
        .body(body);
    for (k, v) in compression_metadata(spec) {
        req = req.metadata(k, v);
    }
    if spec.policy != ConflictPolicy::Replace {
        // Atomic on R2: fails with 412 if the key appeared meanwhile.
        req = req.if_none_match("*");
    }
    let result = tokio::select! {
        () = ctx.control.cancel.cancelled() => {
            settle(&ctx.counter, &sent, None);
            return Err(JobError::cancelled());
        }
        r = req.send() => r.map_err(R2Error::from_sdk),
    };
    match result {
        Ok(_) => {
            settle(&ctx.counter, &sent, Some(size));
            ctx.set_state(JobState::Verifying);
            let check = source.clone();
            blocking(move || check.ensure_unchanged()).await?;
            Ok(Outcome::Completed)
        }
        Err(R2Error::PreconditionFailed { .. }) => {
            settle(&ctx.counter, &sent, None);
            match spec.policy {
                ConflictPolicy::Skip => Ok(Outcome::Skipped),
                _ => Ok(Outcome::NeedsDecision),
            }
        }
        Err(e) => {
            settle(&ctx.counter, &sent, None);
            Err(e.into())
        }
    }
}

async fn create_upload(
    ctx: &JobCtx,
    spec: &UploadSpec,
    source: &SourceFile,
) -> Result<MultipartState, JobError> {
    let part_size = part_size_for(source.fingerprint.len)
        .map_err(|e| JobError::fatal(r2dock_core::AppError::invalid_input(e.to_string())))?;
    let mut req = ctx
        .client
        .sdk()
        .create_multipart_upload()
        .bucket(&spec.bucket)
        .key(&spec.key)
        .content_type(content_type(&spec.key));
    for (k, v) in compression_metadata(spec) {
        req = req.metadata(k, v);
    }
    let out = req.send().await.map_err(R2Error::from_sdk)?;
    let upload_id = out
        .upload_id()
        .ok_or_else(|| {
            JobError::retryable(r2dock_core::AppError::new(
                r2dock_core::ErrorCode::ServerError,
                "R2 did not return an upload id.",
            ))
        })?
        .to_string();
    let state = MultipartState {
        upload_id,
        key: spec.key.clone(),
        part_size,
        fingerprint: source.fingerprint,
        done: BTreeMap::new(),
    };
    ctx.save_resume(ResumeState::Multipart(state.clone()));
    Ok(state)
}

/// Keep only saved parts that R2 still has with the same ETag.
async fn reconcile(
    ctx: &JobCtx,
    spec: &UploadSpec,
    state: &mut MultipartState,
) -> Result<bool, JobError> {
    let mut remote = BTreeMap::new();
    let mut marker: Option<String> = None;
    loop {
        let out = ctx
            .client
            .sdk()
            .list_parts()
            .bucket(&spec.bucket)
            .key(&state.key)
            .upload_id(&state.upload_id)
            .set_part_number_marker(marker.clone())
            .send()
            .await
            .map_err(R2Error::from_sdk);
        let out = match out {
            Ok(o) => o,
            // The upload expired or was aborted: start over.
            Err(R2Error::NoSuchKey { .. }) | Err(R2Error::Service { .. }) => return Ok(false),
            Err(e) => return Err(e.into()),
        };
        for p in out.parts() {
            if let (Some(n), Some(etag)) = (p.part_number(), p.e_tag()) {
                remote.insert(n, normalize(etag));
            }
        }
        match (out.is_truncated(), out.next_part_number_marker()) {
            (Some(true), Some(m)) if Some(m) != marker.as_deref() => marker = Some(m.to_string()),
            _ => break,
        }
    }
    state
        .done
        .retain(|n, p| remote.get(n).is_some_and(|e| *e == normalize(&p.etag)));
    Ok(true)
}

async fn upload_part(
    ctx: &JobCtx,
    spec: &UploadSpec,
    state: &MultipartState,
    source: Arc<SourceFile>,
    part: PartRange,
) -> Result<(i32, PartDone), JobError> {
    let hashing = source.clone();
    let md5 = blocking(move || hashing.md5_range(part.offset, part.len)).await?;
    let sent = Arc::new(AtomicU64::new(0));
    let body = range_body(
        source,
        part.offset,
        part.len,
        ctx.counter.clone(),
        sent.clone(),
    );
    let result = ctx
        .client
        .sdk()
        .upload_part()
        .bucket(&spec.bucket)
        .key(&state.key)
        .upload_id(&state.upload_id)
        .part_number(part.number)
        .content_length(i64::try_from(part.len).unwrap_or(i64::MAX))
        .content_md5(b64(&md5))
        .body(body)
        .send()
        .await
        .map_err(R2Error::from_sdk);
    match result {
        Ok(out) => {
            settle(&ctx.counter, &sent, Some(part.len));
            let etag = out
                .e_tag()
                .map(str::to_string)
                .unwrap_or_else(|| format!("\"{}\"", hex::encode(md5)));
            Ok((
                part.number,
                PartDone {
                    etag,
                    md5: hex::encode(md5),
                    len: part.len,
                },
            ))
        }
        Err(e) => {
            settle(&ctx.counter, &sent, None);
            Err(e.into())
        }
    }
}

async fn multipart(
    ctx: &JobCtx,
    spec: &mut UploadSpec,
    source: Arc<SourceFile>,
    saved: Option<MultipartState>,
) -> Result<Outcome, JobError> {
    let mut state = match saved {
        Some(mut s) => {
            if reconcile(ctx, spec, &mut s).await? {
                ctx.save_resume(ResumeState::Multipart(s.clone()));
                s
            } else {
                ctx.clear_resume();
                if let Err(outcome) = resolve_key(ctx, spec).await? {
                    return Ok(outcome);
                }
                create_upload(ctx, spec, &source).await?
            }
        }
        None => create_upload(ctx, spec, &source).await?,
    };

    let size = source.fingerprint.len;
    let parts = plan_parts(size, state.part_size)
        .map_err(|e| JobError::fatal(r2dock_core::AppError::invalid_input(e.to_string())))?;
    let total = u32::try_from(parts.len()).unwrap_or(u32::MAX);
    let done_bytes: u64 = state.done.values().map(|p| p.len).sum();
    ctx.counter.set_committed(done_bytes);
    ctx.counter.reset_in_flight();
    ctx.hooks
        .parts(ctx.id, u32::try_from(state.done.len()).unwrap_or(0), total);
    ctx.set_state(JobState::Running);

    let snapshot = state.clone();
    let todo: Vec<PartRange> = parts
        .into_iter()
        .filter(|p| !state.done.contains_key(&p.number))
        .collect();
    let mut pending = todo.into_iter();
    let mut in_flight = FuturesUnordered::new();
    let mut failure: Option<JobError> = None;

    loop {
        // Queue parts up to the per-job limit. Each part waits for a global
        // slot inside its own future, so waiting never stops this job from
        // polling (and finishing) the parts it already started.
        while failure.is_none()
            && ctx.control.stop() == Stop::None
            && in_flight.len() < ctx.parts_per_job
        {
            let Some(part) = pending.next() else { break };
            let slots = ctx.part_slots.clone();
            let control = ctx.control.clone();
            let fut = upload_part(ctx, spec, &snapshot, source.clone(), part);
            in_flight.push(async move {
                let Ok(permit) = slots.acquire_owned().await else {
                    return None;
                };
                if control.stop() != Stop::None {
                    return None;
                }
                let r = fut.await;
                drop(permit);
                Some(r)
            });
        }
        if in_flight.is_empty() {
            break;
        }
        let next = tokio::select! {
            () = ctx.control.cancel.cancelled(), if ctx.control.stop() == Stop::Cancel => None,
            r = in_flight.next() => r,
        };
        match next {
            None => break,
            // Not started because of a pause or cancel.
            Some(None) => {}
            Some(Some(Ok((number, done)))) => {
                state.done.insert(number, done);
                ctx.save_resume(ResumeState::Multipart(state.clone()));
                ctx.hooks
                    .parts(ctx.id, u32::try_from(state.done.len()).unwrap_or(0), total);
            }
            Some(Some(Err(e))) => {
                if failure.is_none() {
                    failure = Some(e);
                }
            }
        }
    }
    drop(in_flight);
    ctx.counter.reset_in_flight();

    match ctx.control.stop() {
        Stop::Cancel => {
            let _ = ctx
                .client
                .abort_multipart(&spec.bucket, &state.key, &state.upload_id)
                .await;
            ctx.clear_resume();
            return Err(JobError::cancelled());
        }
        // A pause wins over a part failure: the saved parts are reconciled on resume.
        Stop::Pause
            if failure
                .as_ref()
                .is_none_or(|e| e.app.code != r2dock_core::ErrorCode::FileChanged) =>
        {
            return Ok(Outcome::Paused);
        }
        _ => {}
    }
    if let Some(err) = failure {
        if err.app.code == r2dock_core::ErrorCode::FileChanged {
            let _ = ctx
                .client
                .abort_multipart(&spec.bucket, &state.key, &state.upload_id)
                .await;
            ctx.clear_resume();
        }
        return Err(err);
    }
    if u32::try_from(state.done.len()).unwrap_or(0) != total {
        // Pause raced with the last parts; treat as paused.
        return Ok(Outcome::Paused);
    }

    ctx.set_state(JobState::Verifying);
    let check = source.clone();
    if let Err(e) = blocking(move || check.ensure_unchanged()).await {
        let _ = ctx
            .client
            .abort_multipart(&spec.bucket, &state.key, &state.upload_id)
            .await;
        ctx.clear_resume();
        return Err(e);
    }
    // Multipart completion has no atomic conditional on R2: check again.
    if spec.policy != ConflictPolicy::Replace && ctx.client.exists(&spec.bucket, &state.key).await?
    {
        let _ = ctx
            .client
            .abort_multipart(&spec.bucket, &state.key, &state.upload_id)
            .await;
        ctx.clear_resume();
        return Ok(match spec.policy {
            ConflictPolicy::Skip => Outcome::Skipped,
            _ => Outcome::NeedsDecision,
        });
    }

    // Last chance to honour a cancel before the object becomes visible.
    if ctx.control.stop() == Stop::Cancel {
        let _ = ctx
            .client
            .abort_multipart(&spec.bucket, &state.key, &state.upload_id)
            .await;
        ctx.clear_resume();
        return Err(JobError::cancelled());
    }
    let completed: Vec<CompletedPart> = state
        .done
        .iter()
        .map(|(n, p)| {
            CompletedPart::builder()
                .part_number(*n)
                .e_tag(&p.etag)
                .build()
        })
        .collect();
    let out = ctx
        .client
        .sdk()
        .complete_multipart_upload()
        .bucket(&spec.bucket)
        .key(&state.key)
        .upload_id(&state.upload_id)
        .multipart_upload(
            CompletedMultipartUpload::builder()
                .set_parts(Some(completed))
                .build(),
        )
        .send()
        .await
        .map_err(R2Error::from_sdk)?;

    let digests: Vec<[u8; 16]> = state
        .done
        .values()
        .filter_map(|p| {
            hex::decode(&p.md5)
                .ok()
                .and_then(|v| <[u8; 16]>::try_from(v).ok())
        })
        .collect();
    if let Some(etag) = out.e_tag() {
        let expected = multipart_etag(&digests);
        if r2dock_core::etag::multipart_part_count(etag).is_some() && normalize(etag) != expected {
            tracing::warn!("multipart ETag differs from the locally computed value");
        }
    }
    ctx.clear_resume();
    Ok(Outcome::Completed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_encoding_keeps_headers_ascii() {
        assert_eq!(percent_encode("Book One.wav"), "Book%20One.wav");
        assert_eq!(percent_encode("名前.mp3"), "%E5%90%8D%E5%89%8D.mp3");
        assert_eq!(percent_encode("a\r\nb"), "a%0D%0Ab");
        assert!(percent_encode("x\"y;z").is_ascii());
    }
}
