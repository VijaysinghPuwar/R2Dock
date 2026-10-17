//! Stress tests for the transfer engine against the local S3 server: many
//! small files and several multipart files at once, with pause and resume
//! in the middle, then a full download and byte-for-byte check.
//!
//! The default size keeps CI fast. Run the heavy version with
//! `cargo test -p r2dock-transfer --release --test stress -- --ignored`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use r2dock_core::AppError;
use r2dock_core::connections::Jurisdiction;
use r2dock_core::policy::ConflictPolicy;
use r2dock_r2::{ClientParams, R2Client};
use r2dock_testkit::{ACCESS_KEY, LocalS3, SECRET_KEY};
use r2dock_transfer::{
    BoxFuture, ClientProvider, DownloadSpec, Engine, EngineConfig, JobAction, JobId, JobSpec,
    JobState, UploadSpec,
};
use uuid::Uuid;

const BUCKET: &str = "stress";
const MIB: u64 = 1024 * 1024;

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

/// Deterministic content that differs per file.
fn content(seed: u64, len: u64) -> Vec<u8> {
    (0..len)
        .map(|i| ((i.wrapping_mul(31) ^ seed.wrapping_mul(2_654_435_761)) % 251) as u8)
        .collect()
}

/// Wait until every job is in a final state; panic on a timeout.
async fn wait_all(engine: &Engine, ids: &[JobId], limit: Duration) {
    let start = Instant::now();
    loop {
        let snap = engine.snapshot();
        let done = ids.iter().all(|id| {
            snap.iter().any(|j| {
                j.id == *id
                    && matches!(
                        j.state,
                        JobState::Completed
                            | JobState::Failed
                            | JobState::Cancelled
                            | JobState::Skipped
                    )
            })
        });
        if done {
            return;
        }
        assert!(start.elapsed() < limit, "transfers did not finish in time");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

fn assert_all_completed(engine: &Engine, ids: &[JobId]) {
    let snap = engine.snapshot();
    for id in ids {
        let j = snap.iter().find(|j| j.id == *id).unwrap();
        assert_eq!(j.state, JobState::Completed, "{} {:?}", j.key, j.error);
    }
}

async fn stress(small: u64, large: u64, large_size: u64) {
    let s3 = LocalS3::start(&[BUCKET]).await;
    let provider = Arc::new(Provider {
        endpoint: s3.endpoint.clone(),
    });
    let data = tempfile::tempdir().unwrap();
    let files = tempfile::tempdir().unwrap();
    let mut config = EngineConfig::new(data.path().to_path_buf());
    config.concurrency = 8;
    config.multipart_threshold = 8 * MIB;
    config.progress_interval = Duration::from_millis(50);
    let engine = Engine::start(provider, config);

    // Files: many small ones (including empty and odd names), a few large.
    let mut expected: Vec<(String, Vec<u8>)> = Vec::new();
    let mut specs = Vec::new();
    let add = |name: String, bytes: Vec<u8>, specs: &mut Vec<JobSpec>| {
        let local: PathBuf = files.path().join(format!("f{}", specs.len()));
        std::fs::write(&local, &bytes).unwrap();
        specs.push(JobSpec::Upload(UploadSpec {
            connection: Uuid::nil(),
            bucket: BUCKET.into(),
            key: name.clone(),
            local,
            size: bytes.len() as u64,
            policy: ConflictPolicy::Replace,
            identity: None,
            compression: None,
            compressed: None,
        }));
        (name, bytes)
    };
    for i in 0..small {
        let name = match i % 4 {
            0 => format!("small/{i}.txt"),
            1 => format!("small/deep/a b,c/{i} (copy).txt"),
            2 => format!("small/Ünïcødé/{i}-名前.txt"),
            _ => format!("small/{i}.bin"),
        };
        let len = if i == 0 { 0 } else { (i * 997) % 65_536 };
        expected.push(add(name, content(i, len), &mut specs));
    }
    for i in 0..large {
        // Not a multiple of the part size, so the last part is short.
        let len = large_size + i * 12_345;
        expected.push(add(
            format!("large/{i}.bin"),
            content(10_000 + i, len),
            &mut specs,
        ));
    }
    let total_bytes: u64 = expected.iter().map(|(_, b)| b.len() as u64).sum();

    let started = Instant::now();
    let ids = engine.enqueue(specs, None);

    // Pause everything while work is in flight, then resume it all.
    tokio::time::sleep(Duration::from_millis(300)).await;
    engine.pause_all();
    tokio::time::sleep(Duration::from_millis(300)).await;
    let paused: Vec<JobId> = engine
        .snapshot()
        .into_iter()
        .filter(|j| j.state == JobState::Paused)
        .map(|j| j.id)
        .collect();
    engine.control_many(&paused, JobAction::Resume);

    wait_all(&engine, &ids, Duration::from_secs(600)).await;
    assert_all_completed(&engine, &ids);
    let upload_time = started.elapsed();

    // Download everything into a fresh folder and compare bytes.
    let dest = tempfile::tempdir().unwrap();
    let downloads: Vec<JobSpec> = expected
        .iter()
        .map(|(key, bytes)| {
            JobSpec::Download(DownloadSpec {
                connection: Uuid::nil(),
                bucket: BUCKET.into(),
                key: key.clone(),
                size: bytes.len() as u64,
                dest_root: dest.path().to_path_buf(),
                rel_path: PathBuf::from(key),
                policy: ConflictPolicy::Replace,
            })
        })
        .collect();
    let started = Instant::now();
    let ids = engine.enqueue(downloads, None);
    wait_all(&engine, &ids, Duration::from_secs(600)).await;
    assert_all_completed(&engine, &ids);
    let download_time = started.elapsed();

    for (key, bytes) in &expected {
        let got = std::fs::read(Path::new(dest.path()).join(key)).unwrap();
        assert!(got == *bytes, "content differs for {key}");
    }
    // Finished jobs can all be cleared.
    engine.clear_finished();
    assert!(engine.snapshot().is_empty());

    eprintln!(
        "transfer stress: {} files, {:.1} MB; upload {:.2} s, download {:.2} s",
        expected.len(),
        total_bytes as f64 / 1e6,
        upload_time.as_secs_f64(),
        download_time.as_secs_f64()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn many_files_with_pause_and_resume() {
    stress(400, 4, 20 * MIB).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "heavy; run with --ignored"]
async fn many_files_with_pause_and_resume_heavy() {
    stress(3_000, 12, 96 * MIB).await;
}
