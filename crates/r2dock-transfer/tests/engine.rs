#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use r2dock_core::connections::Jurisdiction;
use r2dock_core::policy::ConflictPolicy;
use r2dock_core::{AppError, ErrorCode};
use r2dock_r2::{ClientParams, R2Client};
use r2dock_testkit::{ACCESS_KEY, LocalS3, SECRET_KEY};
use r2dock_transfer::{
    BoxFuture, ClientProvider, DownloadSpec, Engine, EngineConfig, JobAction, JobId, JobSnapshot,
    JobSpec, JobState, TransferEvent, UploadSpec,
};
use uuid::Uuid;

const BUCKET: &str = "bkt";
const MIB: u64 = 1024 * 1024;

struct Provider {
    endpoint: String,
    secret: String,
}

impl ClientProvider for Provider {
    fn client(&self, _connection: Uuid) -> BoxFuture<'_, Result<R2Client, AppError>> {
        let params = ClientParams {
            account_id: "test".into(),
            access_key_id: ACCESS_KEY.into(),
            secret: self.secret.clone().into(),
            jurisdiction: Jurisdiction::Default,
            endpoint_override: Some(self.endpoint.clone()),
        };
        Box::pin(async move { Ok(R2Client::new(&params)) })
    }
}

struct Harness {
    s3: LocalS3,
    engine: Engine,
    events: Arc<Mutex<Vec<TransferEvent>>>,
    data: tempfile::TempDir,
    files: tempfile::TempDir,
    client: R2Client,
}

async fn harness_with(secret: &str, tweak: impl FnOnce(&mut EngineConfig)) -> Harness {
    let s3 = LocalS3::start(&[BUCKET]).await;
    harness_on(s3, secret, tempfile::tempdir().unwrap(), tweak).await
}

async fn harness_on(
    s3: LocalS3,
    secret: &str,
    data: tempfile::TempDir,
    tweak: impl FnOnce(&mut EngineConfig),
) -> Harness {
    let provider = Arc::new(Provider {
        endpoint: s3.endpoint.clone(),
        secret: secret.to_string(),
    });
    let client = provider.client(Uuid::nil()).await.unwrap();
    let mut config = EngineConfig::new(data.path().to_path_buf());
    config.progress_interval = Duration::from_millis(20);
    tweak(&mut config);
    let engine = Engine::start(provider, config);
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink_events = events.clone();
    engine.subscribe(Arc::new(move |e| sink_events.lock().push(e)));
    Harness {
        s3,
        engine,
        events,
        data,
        files: tempfile::tempdir().unwrap(),
        client,
    }
}

async fn harness() -> Harness {
    harness_with(SECRET_KEY, |_| {}).await
}

fn pattern(len: u64) -> Vec<u8> {
    (0..len).map(|i| (i % 251) as u8).collect()
}

fn write_file(dir: &Path, name: &str, data: &[u8]) -> PathBuf {
    let p = dir.join(name);
    std::fs::write(&p, data).unwrap();
    p
}

fn upload(local: PathBuf, key: &str, policy: ConflictPolicy) -> JobSpec {
    let size = std::fs::metadata(&local).unwrap().len();
    JobSpec::Upload(UploadSpec {
        connection: Uuid::nil(),
        bucket: BUCKET.into(),
        key: key.into(),
        local,
        size,
        policy,
        identity: None,
        compression: None,
        compressed: None,
    })
}

fn download(dest: &Path, key: &str, rel: &str, size: u64, policy: ConflictPolicy) -> JobSpec {
    JobSpec::Download(DownloadSpec {
        connection: Uuid::nil(),
        bucket: BUCKET.into(),
        key: key.into(),
        size,
        dest_root: dest.to_path_buf(),
        rel_path: PathBuf::from(rel),
        policy,
    })
}

fn job(engine: &Engine, id: JobId) -> JobSnapshot {
    engine.snapshot().into_iter().find(|j| j.id == id).unwrap()
}

async fn wait_state(engine: &Engine, id: JobId, wanted: &[JobState]) -> JobSnapshot {
    for _ in 0..1500 {
        let j = job(engine, id);
        if wanted.contains(&j.state) {
            return j;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!(
        "timed out waiting for {wanted:?}; job is {:?}",
        job(engine, id)
    );
}

async fn get(client: &R2Client, key: &str) -> Vec<u8> {
    let out = client.get_object(BUCKET, key, None).await.unwrap();
    out.body.collect().await.unwrap().into_bytes().to_vec()
}

#[tokio::test]
async fn single_upload_and_download_round_trip() {
    let h = harness().await;
    let data = pattern(300_000);
    let local = write_file(h.files.path(), "cover.webp", &data);
    let id = h.engine.enqueue(
        vec![upload(local, "art/cover.webp", ConflictPolicy::Ask)],
        None,
    )[0];
    let j = wait_state(&h.engine, id, &[JobState::Completed, JobState::Failed]).await;
    assert_eq!(j.state, JobState::Completed, "{:?}", j.error);
    assert_eq!(j.transferred_bytes, 300_000);
    assert_eq!(get(&h.client, "art/cover.webp").await, data);
    let head = h
        .client
        .head_object(BUCKET, "art/cover.webp")
        .await
        .unwrap();
    assert_eq!(head.content_type.as_deref(), Some("image/webp"));

    let dest = tempfile::tempdir().unwrap();
    let id = h.engine.enqueue(
        vec![download(
            dest.path(),
            "art/cover.webp",
            "art/cover.webp",
            300_000,
            ConflictPolicy::Ask,
        )],
        None,
    )[0];
    let j = wait_state(&h.engine, id, &[JobState::Completed, JobState::Failed]).await;
    assert_eq!(j.state, JobState::Completed, "{:?}", j.error);
    assert_eq!(
        std::fs::read(dest.path().join("art").join("cover.webp")).unwrap(),
        data
    );
    // No temp files left behind.
    let names: Vec<_> = std::fs::read_dir(dest.path().join("art"))
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(names.len(), 1);

    // Events were emitted for the frontend.
    {
        let events = h.events.lock();
        assert!(matches!(events[0], TransferEvent::Snapshot { .. }));
        assert!(
            events
                .iter()
                .any(|e| matches!(e, TransferEvent::Added { .. }))
        );
        assert!(events.iter().any(
            |e| matches!(e, TransferEvent::Updated { job } if job.state == JobState::Completed)
        ));
    }

    // History records both transfers.
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(h.engine.history(10).len(), 2);
}

#[tokio::test]
async fn multipart_upload_is_correct() {
    let h = harness_with(SECRET_KEY, |c| c.multipart_threshold = 16 * MIB).await;
    let data = pattern(40 * MIB + 123);
    let local = write_file(h.files.path(), "big.bin", &data);
    let id = h.engine.enqueue(
        vec![upload(local, "big.bin", ConflictPolicy::Replace)],
        None,
    )[0];
    let j = wait_state(&h.engine, id, &[JobState::Completed, JobState::Failed]).await;
    assert_eq!(j.state, JobState::Completed, "{:?}", j.error);
    assert_eq!(j.parts_total, Some(3));
    assert_eq!(j.parts_done, Some(3));
    assert!(j.pausable);
    assert_eq!(get(&h.client, "big.bin").await, data);
}

#[tokio::test]
async fn conflict_policies_for_uploads() {
    let h = harness().await;
    let local = write_file(h.files.path(), "a.txt", b"new content");
    h.client
        .sdk()
        .put_object()
        .bucket(BUCKET)
        .key("a.txt")
        .body(b"old".to_vec().into())
        .send()
        .await
        .unwrap();

    let skip = h.engine.enqueue(
        vec![upload(local.clone(), "a.txt", ConflictPolicy::Skip)],
        None,
    )[0];
    assert_eq!(
        wait_state(&h.engine, skip, &[JobState::Skipped, JobState::Failed])
            .await
            .state,
        JobState::Skipped
    );
    assert_eq!(get(&h.client, "a.txt").await, b"old");

    let both = h.engine.enqueue(
        vec![upload(local.clone(), "a.txt", ConflictPolicy::KeepBoth)],
        None,
    )[0];
    let j = wait_state(&h.engine, both, &[JobState::Completed, JobState::Failed]).await;
    assert_eq!(j.key, "a (1).txt");
    assert_eq!(get(&h.client, "a (1).txt").await, b"new content");

    let ask = h
        .engine
        .enqueue(vec![upload(local, "a.txt", ConflictPolicy::Ask)], None)[0];
    let j = wait_state(&h.engine, ask, &[JobState::NeedsDecision, JobState::Failed]).await;
    assert_eq!(j.state, JobState::NeedsDecision);
    assert_eq!(get(&h.client, "a.txt").await, b"old");
    h.engine
        .control(ask, JobAction::Resolve(ConflictPolicy::Replace))
        .unwrap();
    wait_state(&h.engine, ask, &[JobState::Completed]).await;
    assert_eq!(get(&h.client, "a.txt").await, b"new content");
}

#[tokio::test]
async fn conflict_policies_for_downloads() {
    let h = harness().await;
    h.client
        .sdk()
        .put_object()
        .bucket(BUCKET)
        .key("d.txt")
        .body(b"remote".to_vec().into())
        .send()
        .await
        .unwrap();
    let dest = tempfile::tempdir().unwrap();
    std::fs::write(dest.path().join("d.txt"), b"local").unwrap();

    let skip = h.engine.enqueue(
        vec![download(
            dest.path(),
            "d.txt",
            "d.txt",
            6,
            ConflictPolicy::Skip,
        )],
        None,
    )[0];
    wait_state(&h.engine, skip, &[JobState::Skipped]).await;
    assert_eq!(std::fs::read(dest.path().join("d.txt")).unwrap(), b"local");

    let both = h.engine.enqueue(
        vec![download(
            dest.path(),
            "d.txt",
            "d.txt",
            6,
            ConflictPolicy::KeepBoth,
        )],
        None,
    )[0];
    wait_state(&h.engine, both, &[JobState::Completed]).await;
    assert_eq!(
        std::fs::read(dest.path().join("d (1).txt")).unwrap(),
        b"remote"
    );
    assert_eq!(std::fs::read(dest.path().join("d.txt")).unwrap(), b"local");

    let ask = h.engine.enqueue(
        vec![download(
            dest.path(),
            "d.txt",
            "d.txt",
            6,
            ConflictPolicy::Ask,
        )],
        None,
    )[0];
    wait_state(&h.engine, ask, &[JobState::NeedsDecision]).await;
    h.engine
        .control(ask, JobAction::Resolve(ConflictPolicy::Replace))
        .unwrap();
    wait_state(&h.engine, ask, &[JobState::Completed]).await;
    assert_eq!(std::fs::read(dest.path().join("d.txt")).unwrap(), b"remote");
}

#[tokio::test]
async fn traversal_keys_never_escape_destination() {
    let h = harness().await;
    h.client
        .sdk()
        .put_object()
        .bucket(BUCKET)
        .key("../evil.txt")
        .body(b"x".to_vec().into())
        .send()
        .await
        .ok();
    let dest = tempfile::tempdir().unwrap();
    let inner = dest.path().join("inner");
    std::fs::create_dir(&inner).unwrap();
    let id = h.engine.enqueue(
        vec![download(
            &inner,
            "../evil.txt",
            "../evil.txt",
            1,
            ConflictPolicy::Ask,
        )],
        None,
    )[0];
    let j = wait_state(&h.engine, id, &[JobState::Failed, JobState::Completed]).await;
    assert_eq!(j.state, JobState::Failed);
    assert!(!dest.path().join("evil.txt").exists());
}

#[tokio::test]
async fn cancel_multipart_leaves_nothing() {
    let h = harness_with(SECRET_KEY, |c| {
        c.multipart_threshold = 16 * MIB;
        c.parts_per_job = 1;
        c.global_parts = 1;
    })
    .await;
    let local = write_file(h.files.path(), "c.bin", &pattern(64 * MIB));
    let id = h
        .engine
        .enqueue(vec![upload(local, "c.bin", ConflictPolicy::Replace)], None)[0];
    wait_state(&h.engine, id, &[JobState::Running]).await;
    h.engine.control(id, JobAction::Cancel).unwrap();
    wait_state(&h.engine, id, &[JobState::Cancelled]).await;
    assert!(!h.client.exists(BUCKET, "c.bin").await.unwrap());
}

#[tokio::test]
async fn pause_and_resume_multipart() {
    let h = harness_with(SECRET_KEY, |c| {
        c.multipart_threshold = 16 * MIB;
        c.parts_per_job = 1;
        c.global_parts = 1;
    })
    .await;
    let data = pattern(64 * MIB);
    let local = write_file(h.files.path(), "p.bin", &data);
    let id = h
        .engine
        .enqueue(vec![upload(local, "p.bin", ConflictPolicy::Replace)], None)[0];
    wait_state(&h.engine, id, &[JobState::Running]).await;
    h.engine.control(id, JobAction::Pause).unwrap();
    let j = wait_state(&h.engine, id, &[JobState::Paused, JobState::Completed]).await;
    if j.state == JobState::Paused {
        assert!(j.parts_done.unwrap_or(0) < 4);
        // The journal keeps the upload for a restart.
        tokio::time::sleep(Duration::from_millis(400)).await;
        let journal =
            std::fs::read_to_string(h.data.path().join("transfers").join("journal.json")).unwrap();
        assert!(journal.contains("uploadId"));
        h.engine.control(id, JobAction::Resume).unwrap();
        wait_state(&h.engine, id, &[JobState::Completed]).await;
    }
    assert_eq!(get(&h.client, "p.bin").await, data);
}

#[tokio::test]
async fn single_put_cannot_pause() {
    let h = harness().await;
    let local = write_file(h.files.path(), "s.txt", b"small");
    let id = h
        .engine
        .enqueue(vec![upload(local, "s.txt", ConflictPolicy::Replace)], None)[0];
    assert!(!job(&h.engine, id).pausable);
    let err = h.engine.control(id, JobAction::Pause).unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidInput);
    wait_state(&h.engine, id, &[JobState::Completed]).await;
}

#[tokio::test]
async fn wrong_credentials_fail_without_retry_storm() {
    let h = harness_with("not-the-secret", |_| {}).await;
    let a = write_file(h.files.path(), "a.txt", b"a");
    let b = write_file(h.files.path(), "b.txt", b"b");
    let ids = h.engine.enqueue(
        vec![
            upload(a, "a.txt", ConflictPolicy::Replace),
            upload(b, "b.txt", ConflictPolicy::Replace),
        ],
        None,
    );
    let j = wait_state(&h.engine, ids[0], &[JobState::Failed]).await;
    // The auth error itself is never retried. On Windows the server can
    // reset the connection while rejecting the request body, so the first
    // attempt may end as a (retryable) network error before the 403 arrives.
    let code = j.error.as_ref().map(|e| e.code);
    assert_eq!(code, Some(ErrorCode::SignatureMismatch), "{:?}", j.error);
    assert!(
        j.attempt < r2dock_transfer::retry::MAX_ATTEMPTS,
        "stopped at the auth error, attempt {}",
        j.attempt
    );
    if !cfg!(windows) {
        assert_eq!(j.attempt, 1, "auth failures are not retried");
    }
    wait_state(&h.engine, ids[1], &[JobState::Failed]).await;
    assert!(
        h.events
            .lock()
            .iter()
            .any(|e| matches!(e, TransferEvent::AuthFailed { .. }))
    );
}

#[tokio::test]
async fn interrupted_download_resumes_after_restart() {
    let s3 = LocalS3::start(&[BUCKET]).await;
    let data = pattern(2 * MIB + 7);
    let data_dir = tempfile::tempdir().unwrap();
    let dest = tempfile::tempdir().unwrap();

    // First run: put the object and simulate a crash mid-download by
    // writing a partial temp file and a journal entry.
    let first = harness_on(s3, SECRET_KEY, data_dir, |_| {}).await;
    first
        .client
        .sdk()
        .put_object()
        .bucket(BUCKET)
        .key("r.bin")
        .body(data.clone().into())
        .send()
        .await
        .unwrap();
    let etag = first
        .client
        .head_object(BUCKET, "r.bin")
        .await
        .unwrap()
        .etag;
    let final_path = dest.path().canonicalize().unwrap().join("r.bin");
    let job_id = Uuid::new_v4();
    let temp = final_path.with_file_name(format!(
        ".r.bin.{}.r2dock-part",
        &job_id.simple().to_string()[..8]
    ));
    std::fs::write(&temp, &data[..MIB as usize]).unwrap();
    let journal = serde_json::json!({
        "version": 1,
        "entries": [{
            "id": job_id,
            "spec": { "kind": "download", "connection": Uuid::nil(), "bucket": BUCKET, "key": "r.bin",
                      "size": data.len(), "destRoot": dest.path(), "relPath": "r.bin", "policy": "ask" },
            "group": null,
            "resume": { "kind": "download", "etag": etag, "temp": temp, "finalPath": final_path },
            "createdAt": "2026-10-17T00:00:00Z"
        }]
    });
    let Harness {
        s3,
        data: data_dir,
        engine,
        ..
    } = first;
    drop(engine);
    std::fs::create_dir_all(data_dir.path().join("transfers")).unwrap();
    std::fs::write(
        data_dir.path().join("transfers").join("journal.json"),
        journal.to_string(),
    )
    .unwrap();

    // Second run: the job comes back as Interrupted and resumes.
    let second = harness_on(s3, SECRET_KEY, data_dir, |_| {}).await;
    let jobs = second.engine.snapshot();
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].state, JobState::Interrupted);
    assert_eq!(jobs[0].transferred_bytes, MIB);
    second
        .engine
        .control(jobs[0].id, JobAction::Resume)
        .unwrap();
    let j = wait_state(
        &second.engine,
        jobs[0].id,
        &[JobState::Completed, JobState::Failed],
    )
    .await;
    assert_eq!(j.state, JobState::Completed, "{:?}", j.error);
    assert_eq!(std::fs::read(&final_path).unwrap(), data);
    assert!(!temp.exists());
}

#[tokio::test]
async fn many_small_files_respect_concurrency() {
    let h = harness_with(SECRET_KEY, |c| c.concurrency = 2).await;
    let mut specs = Vec::new();
    for i in 0..30 {
        let p = write_file(
            h.files.path(),
            &format!("f{i}.txt"),
            format!("file {i}").as_bytes(),
        );
        specs.push(upload(
            p,
            &format!("many/f{i}.txt"),
            ConflictPolicy::Replace,
        ));
    }
    let group = r2dock_transfer::GroupInfo {
        id: Uuid::new_v4(),
        label: "many".into(),
    };
    let ids = h.engine.enqueue(specs, Some(group));
    let mut max_active = 0;
    for _ in 0..1500 {
        let snap = h.engine.snapshot();
        let active = snap.iter().filter(|j| j.state.is_active()).count();
        max_active = max_active.max(active);
        if snap.iter().all(|j| j.state == JobState::Completed) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert!(max_active <= 2, "max active {max_active}");
    for id in ids {
        assert_eq!(job(&h.engine, id).state, JobState::Completed);
    }
    h.engine.clear_finished();
    assert!(h.engine.snapshot().is_empty());
    let _ = h.s3.endpoint.len();
}

/// Regression: a paused multipart upload comes back after a restart and
/// finishes from its saved parts (the journal used to fail to load).
#[tokio::test]
async fn paused_multipart_resumes_after_restart() {
    let s3 = LocalS3::start(&[BUCKET]).await;
    let tweak = |c: &mut EngineConfig| {
        c.multipart_threshold = 16 * MIB;
        c.parts_per_job = 1;
        c.global_parts = 1;
    };
    let first = harness_on(s3, SECRET_KEY, tempfile::tempdir().unwrap(), tweak).await;
    let data = pattern(64 * MIB);
    let local = write_file(first.files.path(), "r.bin", &data);
    let id = first.engine.enqueue(
        vec![upload(local.clone(), "r.bin", ConflictPolicy::Replace)],
        None,
    )[0];
    wait_state(&first.engine, id, &[JobState::Running]).await;
    // Let at least one part finish, then pause.
    for _ in 0..500 {
        if job(&first.engine, id).parts_done.unwrap_or(0) >= 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    first.engine.control(id, JobAction::Pause).unwrap();
    let j = wait_state(&first.engine, id, &[JobState::Paused, JobState::Completed]).await;
    if j.state == JobState::Completed {
        return; // too fast to test on this machine
    }
    first.engine.flush();
    let Harness {
        s3,
        data: dir,
        engine,
        files,
        ..
    } = first;
    drop(engine);

    let second = harness_on(s3, SECRET_KEY, dir, tweak).await;
    let jobs = second.engine.snapshot();
    assert_eq!(jobs.len(), 1, "the paused upload was restored");
    assert_eq!(jobs[0].state, JobState::Interrupted);
    assert!(jobs[0].parts_done.unwrap_or(0) >= 1);
    second.engine.control(id, JobAction::Resume).unwrap();
    let j = wait_state(&second.engine, id, &[JobState::Completed, JobState::Failed]).await;
    assert_eq!(j.state, JobState::Completed, "{:?}", j.error);
    assert_eq!(get(&second.client, "r.bin").await, data);
    drop(files);
}

/// Regression: parts wait for global slots inside their own futures, so
/// several multipart jobs sharing few slots cannot starve each other.
#[tokio::test]
async fn concurrent_multipart_jobs_do_not_deadlock() {
    let h = harness_with(SECRET_KEY, |c| {
        c.multipart_threshold = 16 * MIB;
        c.concurrency = 3;
        c.parts_per_job = 4;
        c.global_parts = 2;
    })
    .await;
    let mut specs = Vec::new();
    let mut datas = Vec::new();
    for i in 0..3u64 {
        let data: Vec<u8> = (0..(40 * MIB)).map(|b| ((b + i) % 251) as u8).collect();
        let p = write_file(h.files.path(), &format!("m{i}.bin"), &data);
        specs.push(upload(p, &format!("m{i}.bin"), ConflictPolicy::Replace));
        datas.push(data);
    }
    let ids = h.engine.enqueue(specs, None);
    for (i, id) in ids.iter().enumerate() {
        let j = wait_state(&h.engine, *id, &[JobState::Completed, JobState::Failed]).await;
        assert_eq!(j.state, JobState::Completed, "{:?}", j.error);
        assert_eq!(get(&h.client, &format!("m{i}.bin")).await, datas[i]);
    }
}

/// Cancelling a queued job never lets it run.
#[tokio::test]
async fn cancel_queued_job_is_final() {
    let h = harness_with(SECRET_KEY, |c| c.concurrency = 1).await;
    let big = write_file(h.files.path(), "first.bin", &pattern(4 * MIB));
    let second = write_file(h.files.path(), "second.txt", b"never");
    let ids = h.engine.enqueue(
        vec![
            upload(big, "first.bin", ConflictPolicy::Replace),
            upload(second, "second.txt", ConflictPolicy::Replace),
        ],
        None,
    );
    h.engine.control(ids[1], JobAction::Cancel).unwrap();
    wait_state(&h.engine, ids[0], &[JobState::Completed]).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(job(&h.engine, ids[1]).state, JobState::Cancelled);
    assert!(!h.client.exists(BUCKET, "second.txt").await.unwrap());
}
