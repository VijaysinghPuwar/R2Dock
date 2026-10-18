//! Cost of keeping a very large queue on disk (queue.json) while it drains.
//! Ignored by default; run with
//! `cargo test -p r2dock-transfer --release --test queue_scale -- --ignored --nocapture`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use r2dock_core::AppError;
use r2dock_core::policy::ConflictPolicy;
use r2dock_r2::R2Client;
use r2dock_transfer::{
    BoxFuture, ClientProvider, DownloadSpec, Engine, EngineConfig, JobAction, JobSpec, JobState,
};
use uuid::Uuid;

/// Never yields a client: started jobs wait, the rest stay queued.
struct Pending;

impl ClientProvider for Pending {
    fn client(&self, _connection: Uuid) -> BoxFuture<'_, Result<R2Client, AppError>> {
        Box::pin(std::future::pending())
    }
}

fn downloads(dest: &Path, n: usize) -> Vec<JobSpec> {
    (0..n)
        .map(|i| {
            JobSpec::Download(DownloadSpec {
                connection: Uuid::nil(),
                bucket: "media-library".into(),
                key: format!("photos/2024/album-{:03}/IMG_{i:06}.jpg", i % 500),
                size: 4_000_000,
                dest_root: dest.to_path_buf(),
                rel_path: PathBuf::from(format!("album-{:03}/IMG_{i:06}.jpg", i % 500)),
                policy: ConflictPolicy::Ask,
            })
        })
        .collect()
}

fn queue_files(data: &Path) -> u64 {
    let dir = data.join("transfers");
    ["queue.json", "queue.log"]
        .iter()
        .filter_map(|f| std::fs::metadata(dir.join(f)).ok())
        .map(|m| m.len())
        .sum()
}

/// Estimates bytes written to the queue files by watching them: a new
/// queue.json counts in full, a longer queue.log counts by what it grew.
/// Works the same for a rewrite-only queue and a logged one.
struct DiskWatch {
    stop: Arc<std::sync::atomic::AtomicBool>,
    thread: std::thread::JoinHandle<u64>,
}

impl DiskWatch {
    fn start(dir: PathBuf) -> Self {
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = stop.clone();
        let thread = std::thread::spawn(move || {
            let stamp = |f: &str| {
                std::fs::metadata(dir.join(f))
                    .ok()
                    .map(|m| (m.modified().ok(), m.len()))
            };
            let mut snap = stamp("queue.json");
            let mut log = stamp("queue.log").map_or(0, |s| s.1);
            let mut written = 0;
            while !flag.load(std::sync::atomic::Ordering::Relaxed) {
                let now = stamp("queue.json");
                if now != snap {
                    written += now.map_or(0, |s| s.1);
                    snap = now;
                }
                let len = stamp("queue.log").map_or(0, |s| s.1);
                if len > log {
                    written += len - log;
                }
                log = len;
                std::thread::sleep(Duration::from_millis(5));
            }
            written
        });
        Self { stop, thread }
    }

    fn finish(self) -> u64 {
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
        self.thread.join().unwrap()
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "large queue benchmark"]
async fn draining_a_huge_queue_keeps_disk_writes_small() {
    let data = tempfile::tempdir().unwrap();
    let engine = Engine::start(
        Arc::new(Pending),
        EngineConfig::new(data.path().to_path_buf()),
    );
    let n = 100_000;
    let t = Instant::now();
    let ids = engine.enqueue(downloads(data.path(), n), None);
    let enqueue = t.elapsed();
    let t = Instant::now();
    engine.flush();
    let flush = t.elapsed();
    let size = queue_files(data.path());

    // Drain half, one job at a time, over about 10 seconds (a queue that
    // finishes 5,000 small files a second), while the writer keeps saving.
    let watch = DiskWatch::start(data.path().join("transfers"));
    let t = Instant::now();
    for (i, id) in ids.iter().skip(10).take(n / 2).enumerate() {
        engine.control(*id, JobAction::Cancel).unwrap();
        if i % 500 == 499 {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
    let drain = t.elapsed();
    tokio::time::sleep(Duration::from_secs(2)).await;
    let written = watch.finish();
    engine.flush();
    let after = queue_files(data.path());
    eprintln!(
        "{n} queued: enqueue {enqueue:?}, full save {flush:?}, {size} bytes on disk; \
         cancel {} one by one over {drain:?}: about {written} bytes written; \
         {after} bytes on disk after",
        n / 2
    );
    // The restart sees exactly the jobs that were not finished.
    drop(engine);
    let again = Engine::start(
        Arc::new(Pending),
        EngineConfig::new(data.path().to_path_buf()),
    );
    let restored = again.snapshot();
    let cancelled: std::collections::HashSet<_> = ids.iter().skip(10).take(n / 2).collect();
    assert_eq!(restored.len(), n - n / 2);
    assert!(restored.iter().all(|j| !cancelled.contains(&j.id)));
    assert!(restored.iter().all(|j| j.state == JobState::Interrupted));
}
