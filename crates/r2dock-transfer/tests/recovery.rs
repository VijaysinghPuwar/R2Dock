//! Recovery after a restart, and queue progress after early failures.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use r2dock_core::AppError;
use r2dock_core::connections::Jurisdiction;
use r2dock_core::policy::ConflictPolicy;
use r2dock_r2::{ClientParams, R2Client};
use r2dock_testkit::{ACCESS_KEY, LocalS3, SECRET_KEY};
use r2dock_transfer::ctx::{DownloadState, ResumeState};
use r2dock_transfer::journal::{Journal, JournalEntry};
use r2dock_transfer::source::WrittenFile;
use r2dock_transfer::{
    BoxFuture, ClientProvider, CompressionRequest, DownloadSpec, Engine, EngineConfig, JobAction,
    JobId, JobSnapshot, JobSpec, JobState, TransferEvent, UploadSpec,
};
use uuid::Uuid;

const BUCKET: &str = "bkt";

struct Provider {
    endpoint: String,
}

impl ClientProvider for Provider {
    fn client(&self, _connection: Uuid) -> BoxFuture<'_, Result<R2Client, AppError>> {
        let params = ClientParams {
            account_id: "test".into(),
            access_key_id: ACCESS_KEY.into(),
            secret: SECRET_KEY.to_string().into(),
            jurisdiction: Jurisdiction::Default,
            endpoint_override: Some(self.endpoint.clone()),
        };
        Box::pin(async move { Ok(R2Client::new(&params)) })
    }
}

/// Never yields a client, so accepted jobs stay where they are.
struct PendingProvider;

impl ClientProvider for PendingProvider {
    fn client(&self, _connection: Uuid) -> BoxFuture<'_, Result<R2Client, AppError>> {
        Box::pin(std::future::pending())
    }
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
    for _ in 0..500 {
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

fn temp_for(final_path: &Path, id: JobId) -> PathBuf {
    let name = final_path
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    final_path.with_file_name(format!(
        ".{name}.{}.r2dock-part",
        &id.simple().to_string()[..8]
    ))
}

/// Write a journal holding one download whose partial file is gone.
fn journal_download(data: &Path, id: JobId, spec: JobSpec, state: DownloadState) {
    let (journal, _) = Journal::load(data.join("transfers").join("journal.json"));
    journal.upsert(JournalEntry {
        id,
        spec,
        group: None,
        resume: ResumeState::Download(state),
        created_at: "2026-10-18T00:00:00Z".into(),
        seq: 0,
    });
    journal.flush();
}

struct Restarted {
    _s3: LocalS3,
    engine: Engine,
    client: R2Client,
}

async fn start_on(data: &Path) -> Restarted {
    let s3 = LocalS3::start(&[BUCKET]).await;
    let provider = Arc::new(Provider {
        endpoint: s3.endpoint.clone(),
    });
    let client = provider.client(Uuid::nil()).await.unwrap();
    let engine = Engine::start(provider, EngineConfig::new(data.to_path_buf()));
    Restarted {
        _s3: s3,
        engine,
        client,
    }
}

async fn put(client: &R2Client, key: &str, body: &[u8]) {
    client
        .sdk()
        .put_object()
        .bucket(BUCKET)
        .key(key)
        .body(body.to_vec().into())
        .send()
        .await
        .unwrap();
}

/// A journal entry from an older version has no finalization record. A
/// missing partial beside a same-size file is not proof that the download
/// finished: with Replace the object is downloaded again.
#[tokio::test]
async fn missing_partial_without_proof_downloads_again() {
    let data = tempfile::tempdir().unwrap();
    let dest = tempfile::tempdir().unwrap();
    let root = dest.path().canonicalize().unwrap();
    let final_path = root.join("same.txt");
    std::fs::write(&final_path, b"OLD!").unwrap();
    let id = Uuid::new_v4();
    journal_download(
        data.path(),
        id,
        download(&root, "remote.txt", "same.txt", 4, ConflictPolicy::Replace),
        DownloadState {
            etag: Some("\"remote-etag\"".into()),
            temp: temp_for(&final_path, id),
            final_path: final_path.clone(),
            finalized: None,
        },
    );
    let h = start_on(data.path()).await;
    put(&h.client, "remote.txt", b"NEW!").await;
    assert_eq!(job(&h.engine, id).state, JobState::Interrupted);
    h.engine.control(id, JobAction::Resume).unwrap();
    let j = wait_state(&h.engine, id, &[JobState::Completed, JobState::Failed]).await;
    assert_eq!(j.state, JobState::Completed, "{:?}", j.error);
    assert_eq!(std::fs::read(&final_path).unwrap(), b"NEW!");
}

/// Without proof and with policy Ask, the existing file is a conflict and
/// is left alone.
#[tokio::test]
async fn missing_partial_without_proof_keeps_a_user_file() {
    for (policy, wanted) in [
        (ConflictPolicy::Ask, JobState::NeedsDecision),
        (ConflictPolicy::Skip, JobState::Skipped),
    ] {
        let data = tempfile::tempdir().unwrap();
        let dest = tempfile::tempdir().unwrap();
        let root = dest.path().canonicalize().unwrap();
        let final_path = root.join("same.txt");
        std::fs::write(&final_path, b"OLD!").unwrap();
        let id = Uuid::new_v4();
        journal_download(
            data.path(),
            id,
            download(&root, "remote.txt", "same.txt", 4, policy),
            DownloadState {
                etag: Some("\"remote-etag\"".into()),
                temp: temp_for(&final_path, id),
                final_path: final_path.clone(),
                finalized: None,
            },
        );
        let h = start_on(data.path()).await;
        put(&h.client, "remote.txt", b"NEW!").await;
        h.engine.control(id, JobAction::Resume).unwrap();
        let j = wait_state(&h.engine, id, &[wanted, JobState::Completed]).await;
        assert_eq!(j.state, wanted, "{policy:?}");
        assert_eq!(std::fs::read(&final_path).unwrap(), b"OLD!");
    }
}

/// The app stopped right after moving a finished download into place: the
/// recorded file is there, so the job completes without downloading again
/// (the object no longer even exists) and the file is left as it is. This
/// also holds when "Keep both" placed it under a new name.
#[tokio::test]
async fn finalized_download_completes_after_restart() {
    for (policy, placed) in [
        (ConflictPolicy::Replace, "same.txt"),
        (ConflictPolicy::KeepBoth, "same (1).txt"),
    ] {
        let data = tempfile::tempdir().unwrap();
        let dest = tempfile::tempdir().unwrap();
        let root = dest.path().canonicalize().unwrap();
        let final_path = root.join("same.txt");
        if placed != "same.txt" {
            std::fs::write(&final_path, b"USER").unwrap();
        }
        let ours = root.join(placed);
        std::fs::write(&ours, b"NEW!").unwrap();
        let proof = WrittenFile::of(&std::fs::symlink_metadata(&ours).unwrap());
        let id = Uuid::new_v4();
        journal_download(
            data.path(),
            id,
            download(&root, "gone.txt", "same.txt", 4, policy),
            DownloadState {
                etag: Some("\"remote-etag\"".into()),
                temp: temp_for(&final_path, id),
                final_path: final_path.clone(),
                finalized: Some(proof),
            },
        );
        let h = start_on(data.path()).await;
        h.engine.control(id, JobAction::Resume).unwrap();
        let j = wait_state(&h.engine, id, &[JobState::Completed, JobState::Failed]).await;
        assert_eq!(j.state, JobState::Completed, "{policy:?}: {:?}", j.error);
        assert!(j.local_display.ends_with(placed), "{}", j.local_display);
        assert_eq!(std::fs::read(&ours).unwrap(), b"NEW!");
        if placed != "same.txt" {
            assert_eq!(std::fs::read(&final_path).unwrap(), b"USER");
        }
    }
}

/// A record that does not match the file at the destination (another file
/// of the same size took its place) is not proof: the object is downloaded
/// again under the user's policy.
#[tokio::test]
async fn finalization_record_must_match_the_file() {
    let data = tempfile::tempdir().unwrap();
    let dest = tempfile::tempdir().unwrap();
    let root = dest.path().canonicalize().unwrap();
    let final_path = root.join("same.txt");
    let other = root.join("other.txt");
    std::fs::write(&other, b"NEW!").unwrap();
    let proof = WrittenFile::of(&std::fs::symlink_metadata(&other).unwrap());
    std::fs::remove_file(&other).unwrap();
    std::thread::sleep(Duration::from_millis(20));
    std::fs::write(&final_path, b"OLD!").unwrap();
    let id = Uuid::new_v4();
    journal_download(
        data.path(),
        id,
        download(&root, "remote.txt", "same.txt", 4, ConflictPolicy::Replace),
        DownloadState {
            etag: Some("\"remote-etag\"".into()),
            temp: temp_for(&final_path, id),
            final_path: final_path.clone(),
            finalized: Some(proof),
        },
    );
    let h = start_on(data.path()).await;
    put(&h.client, "remote.txt", b"NEW!").await;
    h.engine.control(id, JobAction::Resume).unwrap();
    let j = wait_state(&h.engine, id, &[JobState::Completed, JobState::Failed]).await;
    assert_eq!(j.state, JobState::Completed, "{:?}", j.error);
    assert_eq!(std::fs::read(&final_path).unwrap(), b"NEW!");
}

/// Accepted work that never started (here: 5,000 paused downloads) is
/// listed again after a restart, waiting for the user.
#[tokio::test]
async fn accepted_queue_survives_restart() {
    let data = tempfile::tempdir().unwrap();
    let dest = data.path().join("dest");
    std::fs::create_dir_all(&dest).unwrap();
    let ids = {
        let engine = Engine::start(
            Arc::new(PendingProvider),
            EngineConfig::new(data.path().to_path_buf()),
        );
        let specs = (0..5000)
            .map(|i| {
                download(
                    &dest,
                    &format!("{i}.txt"),
                    &format!("{i}.txt"),
                    1,
                    ConflictPolicy::Ask,
                )
            })
            .collect();
        let ids = engine.enqueue(specs, None);
        engine.pause_all();
        // The scheduler may have started up to three before the pause.
        let paused = engine
            .snapshot()
            .iter()
            .filter(|j| j.state == JobState::Paused)
            .count();
        assert!(paused >= 4997, "{paused}");
        engine.flush();
        ids
    };
    let engine = Engine::start(
        Arc::new(PendingProvider),
        EngineConfig::new(data.path().to_path_buf()),
    );
    let jobs = engine.snapshot();
    assert_eq!(jobs.len(), 5000);
    assert!(jobs.iter().all(|j| j.state == JobState::Interrupted));
    let restored: Vec<JobId> = jobs.iter().map(|j| j.id).collect();
    assert_eq!(restored, ids, "restored in the order they were added");
    assert_eq!(jobs[42].key, "42.txt");
}

/// A job that finished is not restored; one that failed is not either.
#[tokio::test]
async fn finished_jobs_are_not_restored() {
    let data = tempfile::tempdir().unwrap();
    let dest = tempfile::tempdir().unwrap();
    let root = dest.path().canonicalize().unwrap();
    let (cancelled, done) = {
        let h = start_on(data.path()).await;
        put(&h.client, "a.txt", b"hello").await;
        let ids = h.engine.enqueue(
            vec![
                download(&root, "a.txt", "a.txt", 5, ConflictPolicy::Replace),
                download(&root, "b.txt", "b.txt", 5, ConflictPolicy::Replace),
            ],
            None,
        );
        wait_state(&h.engine, ids[0], &[JobState::Completed]).await;
        // b.txt does not exist on the server: it fails (or is cancelled).
        let _ = h.engine.control(ids[1], JobAction::Cancel);
        wait_state(
            &h.engine,
            ids[1],
            &[JobState::Cancelled, JobState::Failed, JobState::Retrying],
        )
        .await;
        let _ = h.engine.control(ids[1], JobAction::Cancel);
        wait_state(&h.engine, ids[1], &[JobState::Cancelled, JobState::Failed]).await;
        h.engine.flush();
        (ids[1], ids[0])
    };
    let engine = Engine::start(
        Arc::new(PendingProvider),
        EngineConfig::new(data.path().to_path_buf()),
    );
    let ids: Vec<JobId> = engine.snapshot().iter().map(|j| j.id).collect();
    assert!(!ids.contains(&cancelled));
    assert!(!ids.contains(&done));
}

/// A compression that fails before encoding (the file is gone) frees its
/// slot at once: the next job starts without any other activity, and each
/// failure is announced once.
#[tokio::test]
async fn early_compression_failure_starts_the_next_job() {
    use r2dock_transfer::engine::MediaRuntime;
    let data = tempfile::tempdir().unwrap();
    let engine = Engine::start(
        Arc::new(PendingProvider),
        EngineConfig::new(data.path().to_path_buf()),
    );
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = events.clone();
    engine.subscribe(Arc::new(move |e| sink.lock().push(e)));
    engine.set_media(Some(MediaRuntime::new(
        // Never run: the source cannot be opened.
        r2dock_media::ffmpeg::Ffmpeg::new(data.path().join("no-ffmpeg")),
        data.path().join("work"),
        1,
    )));
    tokio::time::sleep(Duration::from_millis(50)).await;
    let specs = (0..3)
        .map(|i| {
            JobSpec::Upload(UploadSpec {
                connection: Uuid::nil(),
                bucket: BUCKET.into(),
                key: format!("missing{i}.wav"),
                local: data.path().join(format!("missing{i}.wav")),
                size: 100,
                policy: ConflictPolicy::Ask,
                identity: None,
                compression: Some(CompressionRequest {
                    settings: Default::default(),
                    content: Default::default(),
                    on_failure: Default::default(),
                    video: Default::default(),
                }),
                compressed: None,
            })
        })
        .collect();
    let ids = engine.enqueue(specs, None);
    for id in &ids {
        wait_state(&engine, *id, &[JobState::Failed]).await;
    }
    tokio::time::sleep(Duration::from_millis(100)).await;
    for id in &ids {
        let failed = events
            .lock()
            .iter()
            .filter(|e| {
                matches!(e, TransferEvent::Updated { job }
                    if job.id == *id && job.state == JobState::Failed)
            })
            .count();
        assert_eq!(failed, 1, "one final event per job");
    }
    assert_eq!(engine.active_count(), 0);
}
