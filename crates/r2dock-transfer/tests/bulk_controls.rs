//! Queue-wide controls (pause all, resume many, cancel many) on large queues.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use r2dock_core::AppError;
use r2dock_core::policy::ConflictPolicy;
use r2dock_r2::R2Client;
use r2dock_transfer::{
    BoxFuture, ClientProvider, DownloadSpec, Engine, EngineConfig, JobAction, JobId, JobSpec,
    JobState, TransferEvent,
};
use uuid::Uuid;

/// Never yields a client: started jobs wait in Preparing, the rest stay queued.
struct PendingProvider;

impl ClientProvider for PendingProvider {
    fn client(&self, _connection: Uuid) -> BoxFuture<'_, Result<R2Client, AppError>> {
        Box::pin(std::future::pending())
    }
}

fn downloads(dest: &Path, n: usize) -> Vec<JobSpec> {
    (0..n)
        .map(|i| {
            JobSpec::Download(DownloadSpec {
                connection: Uuid::nil(),
                bucket: "bkt".into(),
                key: format!("{i}.txt"),
                size: 1,
                dest_root: dest.to_path_buf(),
                rel_path: PathBuf::from(format!("{i}.txt")),
                policy: ConflictPolicy::Ask,
            })
        })
        .collect()
}

fn count(engine: &Engine, state: JobState) -> usize {
    engine
        .snapshot()
        .iter()
        .filter(|j| j.state == state)
        .count()
}

type Events = Arc<Mutex<Vec<TransferEvent>>>;

async fn setup(n: usize) -> (tempfile::TempDir, Engine, Vec<JobId>, Events) {
    let data = tempfile::tempdir().unwrap();
    let engine = Engine::start(
        Arc::new(PendingProvider),
        EngineConfig::new(data.path().to_path_buf()),
    );
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = events.clone();
    engine.subscribe(Arc::new(move |e| sink.lock().push(e)));
    let ids = engine.enqueue(downloads(data.path(), n), None);
    // Let the scheduler start its first jobs (they wait for a client).
    tokio::time::sleep(Duration::from_millis(50)).await;
    (data, engine, ids, events)
}

fn updates(events: &Events, id: JobId, state: JobState) -> usize {
    events
        .lock()
        .iter()
        .filter(
            |e| matches!(e, TransferEvent::Updated { job } if job.id == id && job.state == state),
        )
        .count()
}

/// Pause, resume, and cancel keep their per-job meaning on a large queue:
/// every queued job changes once, jobs come back in their original order,
/// and each cancelled job gets exactly one final event.
#[tokio::test]
async fn bulk_controls_keep_per_job_behavior() {
    let (_data, engine, ids, events) = setup(1000).await;
    let started = count(&engine, JobState::Preparing);
    assert_eq!(started, 3);

    engine.pause_all();
    // Started downloads stop at once when interrupted; the rest are paused.
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(count(&engine, JobState::Paused), 1000);
    for id in &ids[3..] {
        assert_eq!(updates(&events, *id, JobState::Paused), 1);
    }

    engine.control_many(&ids, JobAction::Resume);
    tokio::time::sleep(Duration::from_millis(100)).await;
    // The first three (by original order) are started again.
    let snap = engine.snapshot();
    for (i, j) in snap.iter().enumerate() {
        let want = if i < 3 {
            JobState::Preparing
        } else {
            JobState::Queued
        };
        assert_eq!(j.state, want, "job {i}");
    }

    engine.control_many(&ids, JobAction::Cancel);
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(count(&engine, JobState::Cancelled), 1000);
    for id in &ids {
        assert_eq!(
            updates(&events, *id, JobState::Cancelled),
            1,
            "one final event"
        );
    }
    // A second cancel is not applicable and changes nothing.
    engine.control_many(&ids, JobAction::Cancel);
    assert!(engine.control(ids[10], JobAction::Cancel).is_err());
    assert_eq!(updates(&events, ids[10], JobState::Cancelled), 1);
}

/// Timing for queue-wide controls on 5,000 jobs. Run in release:
/// `cargo test -p r2dock-transfer --release --test bulk_controls -- --ignored --nocapture`
#[tokio::test]
#[ignore = "timing; run in release"]
async fn bulk_controls_timing() {
    const N: usize = 5000;
    const RUNS: usize = 5;
    let mut pause = Vec::new();
    let mut resume = Vec::new();
    let mut cancel = Vec::new();
    for _ in 0..RUNS {
        let (_data, engine, ids, _events) = setup(N).await;
        let t = Instant::now();
        engine.pause_all();
        pause.push(t.elapsed());
        tokio::time::sleep(Duration::from_millis(100)).await;
        let t = Instant::now();
        engine.control_many(&ids, JobAction::Resume);
        resume.push(t.elapsed());
        let t = Instant::now();
        engine.control_many(&ids, JobAction::Cancel);
        cancel.push(t.elapsed());
    }
    let median = |v: &mut Vec<Duration>| {
        v.sort();
        v[v.len() / 2]
    };
    eprintln!(
        "{N} jobs, {RUNS} runs, median: pause_all {:?}, resume {:?}, cancel {:?}",
        median(&mut pause),
        median(&mut resume),
        median(&mut cancel)
    );
    eprintln!("pause runs: {pause:?}\nresume runs: {resume:?}\ncancel runs: {cancel:?}");
}
