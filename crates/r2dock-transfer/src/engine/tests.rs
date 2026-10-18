//! Ordering between saving accepted jobs to the queue file and a job
//! finishing at the same moment (a finished job must never come back as
//! interrupted after a restart).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use r2dock_core::AppError;
use r2dock_core::policy::ConflictPolicy;
use r2dock_r2::R2Client;
use uuid::Uuid;

use super::{BoxFuture, ClientProvider, Engine, EngineConfig};
use crate::model::{DownloadSpec, JobAction, JobId, JobSpec, JobState};

/// Every client request fails at once, so a started job fails right away.
struct Failing;

impl ClientProvider for Failing {
    fn client(&self, _connection: Uuid) -> BoxFuture<'_, Result<R2Client, AppError>> {
        Box::pin(async { Err(AppError::internal("no client in this test")) })
    }
}

/// Never yields a client: started jobs wait.
struct Pending;

impl ClientProvider for Pending {
    fn client(&self, _connection: Uuid) -> BoxFuture<'_, Result<R2Client, AppError>> {
        Box::pin(std::future::pending())
    }
}

fn download(dest: &std::path::Path, i: usize) -> JobSpec {
    JobSpec::Download(DownloadSpec {
        connection: Uuid::nil(),
        bucket: "bkt".into(),
        key: format!("{i}.txt"),
        size: 1,
        dest_root: dest.to_path_buf(),
        rel_path: PathBuf::from(format!("{i}.txt")),
        policy: ConflictPolicy::Ask,
    })
}

fn state(engine: &Engine, id: JobId) -> JobState {
    engine
        .snapshot()
        .into_iter()
        .find(|j| j.id == id)
        .map(|j| j.state)
        .unwrap()
}

/// While accepted jobs are being saved, another thread cancels one of them
/// (it stands for a worker finishing the job). Whatever the timing, the
/// finished job's record must be gone afterwards.
fn race_finish_with_save(engine: &Engine) {
    let other = engine.clone();
    *engine.inner.queue_save_hook.lock() = Some(Arc::new(move |ids: &[JobId]| {
        let Some(&id) = ids.first() else { return };
        let finisher = other.clone();
        let done = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = done.clone();
        let runtime = tokio::runtime::Handle::current();
        std::thread::spawn(move || {
            let _inside = runtime.enter();
            let _ = finisher.control(id, JobAction::Cancel);
            flag.store(true, std::sync::atomic::Ordering::SeqCst);
        });
        // Give the finish every chance to land before the save; with the
        // save under the jobs lock it cannot, and this just times out.
        let start = Instant::now();
        while !done.load(std::sync::atomic::Ordering::SeqCst)
            && start.elapsed() < Duration::from_millis(300)
        {
            std::thread::sleep(Duration::from_millis(5));
        }
    }));
}

/// A pause or cancel that arrives while a big compressed copy is being
/// checked before upload stops the check between reads and is honored,
/// instead of the job going on (or back to the queue) as if nothing
/// happened.
#[tokio::test(flavor = "multi_thread")]
async fn stop_during_the_copy_check_is_honored() {
    use crate::ctx::RunControl;
    use crate::model::{CompressedCopy, CompressionResult, CopyDigest, UploadSpec};
    use crate::source::{SourceFile, digest_written};

    let data = tempfile::tempdir().unwrap();
    let engine = Engine::start(
        Arc::new(Pending),
        EngineConfig::new(data.path().to_path_buf()),
    );
    let original = data.path().join("book.mp3");
    std::fs::write(&original, b"original audio").unwrap();
    // A big (sparse) copy, so reading it takes a while (hashing is much
    // slower in debug builds).
    let size: u64 = if cfg!(debug_assertions) { 64 } else { 1024 } * 1024 * 1024;
    let copy = data.path().join("copy.mp3");
    std::fs::File::create(&copy).unwrap().set_len(size).unwrap();
    let full = Instant::now();
    let (file, sha256) = digest_written(&copy).unwrap();
    let full = full.elapsed();
    let spec = JobSpec::Upload(UploadSpec {
        connection: Uuid::nil(),
        bucket: "bkt".into(),
        key: "book.mp3".into(),
        local: copy.clone(),
        size: file.len,
        policy: ConflictPolicy::Replace,
        identity: None,
        compression: None,
        compressed: Some(CompressionResult::Compressed(CompressedCopy {
            temp: copy.clone(),
            original_local: original.clone(),
            original_key: "book.mp3".into(),
            original_size: 14,
            original: SourceFile::open(&original, None).unwrap().fingerprint,
            preset: "mp3-32k".into(),
            output: Some(CopyDigest { file, sha256 }),
            original_identity: None,
        })),
    });
    let id = Uuid::new_v4();
    let control = Arc::new(RunControl::new());
    {
        let mut jobs = engine.inner.jobs.lock();
        let mut snap = engine.inner.new_snapshot(id, &spec, None);
        snap.state = JobState::Preparing;
        jobs.order.push(id);
        jobs.map.insert(
            id,
            super::Entry {
                snap,
                spec,
                resume: Arc::new(parking_lot::Mutex::new(crate::ctx::ResumeState::None)),
                counter: Arc::new(crate::progress::Counter::default()),
                control: Some(control.clone()),
                meter: None,
                started: None,
                last_sent: 0,
                seq: 0,
            },
        );
    }
    // Unstopped, the copy passes.
    assert!(matches!(
        super::gate_copy(&engine.inner, id, &Arc::new(RunControl::new())).await,
        super::CopyGate::Go
    ));
    // A pause shortly after the check starts.
    let pauser = control.clone();
    tokio::spawn(async move {
        tokio::time::sleep(full / 10).await;
        pauser.request_pause(false);
    });
    let start = Instant::now();
    let gate = super::gate_copy(&engine.inner, id, &control).await;
    let took = start.elapsed();
    assert!(matches!(gate, super::CopyGate::Stopped));
    assert!(
        took < full / 2,
        "the check stopped after {took:?} (a full read takes {full:?})"
    );
    // The copy is kept for the resume.
    assert!(copy.is_file());
    assert_eq!(state(&engine, id), JobState::Preparing);
}

fn copy_of(dir: &std::path::Path) -> crate::model::CompressedCopy {
    use crate::source::{SourceFile, digest_written};
    let original = dir.join("talk.wav");
    std::fs::write(&original, b"original audio").unwrap();
    let temp = dir.join("copy.mp3");
    std::fs::write(&temp, b"compressed").unwrap();
    let (file, sha256) = digest_written(&temp).unwrap();
    crate::model::CompressedCopy {
        temp,
        original_local: original.clone(),
        original_key: "talk.wav".into(),
        original_size: 14,
        original: SourceFile::open(&original, None).unwrap().fingerprint,
        preset: "mp3-32k".into(),
        output: Some(crate::model::CopyDigest { file, sha256 }),
        original_identity: crate::source::FileIdentity::of(&std::fs::metadata(&original).unwrap()),
    }
}

/// The SHA-256 decides whether a copy is usable; a new device or inode
/// number (some file systems report new ones after a restart) does not
/// force a new encode, while changed content does.
#[test]
fn copy_check_trusts_content_not_device_numbers() {
    use super::compress::{CopyCheck, check_copy};
    let dir = tempfile::tempdir().unwrap();
    let mut c = copy_of(dir.path());
    if let Some(d) = &mut c.output {
        d.file.identity = Some(crate::source::FileIdentity {
            dev: u64::MAX,
            ino: 1,
        });
    }
    assert!(matches!(check_copy(&c, &|| false), CopyCheck::Usable(_)));
    // Same size and modification time, other bytes.
    let mtime = std::fs::metadata(&c.temp).unwrap().modified().unwrap();
    std::fs::write(&c.temp, b"COMPRESSED").unwrap();
    std::fs::OpenOptions::new()
        .write(true)
        .open(&c.temp)
        .unwrap()
        .set_modified(mtime)
        .unwrap();
    assert!(matches!(check_copy(&c, &|| false), CopyCheck::Unusable));
}

/// Dropping a copy goes back to the original under its scan identity, so a
/// swapped original is still refused.
#[test]
fn dropping_a_copy_restores_the_original_identity() {
    use crate::model::{CompressionResult, UploadSpec};
    let dir = tempfile::tempdir().unwrap();
    let c = copy_of(dir.path());
    let want = c.original_identity;
    let mut u = UploadSpec {
        connection: Uuid::nil(),
        bucket: "bkt".into(),
        key: "talk.mp3".into(),
        local: c.temp.clone(),
        size: 10,
        policy: ConflictPolicy::Replace,
        identity: None,
        compression: None,
        compressed: Some(CompressionResult::Compressed(c)),
    };
    super::compress::restore_original(&mut u);
    assert_eq!(u.key, "talk.wav");
    assert_eq!(u.size, 14);
    assert_eq!(u.identity, want);
    assert!(u.compressed.is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_job_finished_while_it_is_saved_is_not_kept() {
    let data = tempfile::tempdir().unwrap();
    let engine = Engine::start(
        Arc::new(Pending),
        EngineConfig::new(data.path().to_path_buf()),
    );
    // Fill the slots so the next job stays queued (and cancels at once).
    engine.enqueue((0..8).map(|i| download(data.path(), i)).collect(), None);
    race_finish_with_save(&engine);
    let ids = engine.enqueue(vec![download(data.path(), 100)], None);
    let id = ids[0];
    for _ in 0..200 {
        if state(&engine, id) == JobState::Cancelled {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(state(&engine, id), JobState::Cancelled);
    // The state flips under the lock; the record is removed just after.
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(
        !engine.inner.queue.contains(id),
        "a cancelled job was saved after it finished"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_retried_job_finished_while_it_is_saved_is_not_kept() {
    let data = tempfile::tempdir().unwrap();
    let engine = Engine::start(
        Arc::new(Failing),
        EngineConfig::new(data.path().to_path_buf()),
    );
    let id = engine.enqueue(vec![download(data.path(), 1)], None)[0];
    for _ in 0..200 {
        if state(&engine, id) == JobState::Failed {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(state(&engine, id), JobState::Failed);
    assert!(!engine.inner.queue.contains(id), "a failed job is not kept");
    race_finish_with_save(&engine);
    engine.control(id, JobAction::Retry).unwrap();
    for _ in 0..200 {
        if matches!(state(&engine, id), JobState::Cancelled | JobState::Failed) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(
        matches!(state(&engine, id), JobState::Cancelled | JobState::Failed),
        "{:?}",
        state(&engine, id)
    );
    assert!(
        !engine.inner.queue.contains(id),
        "a finished job was saved after it finished"
    );
}
