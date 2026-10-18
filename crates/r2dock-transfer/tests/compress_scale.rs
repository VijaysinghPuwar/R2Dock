//! Compression and uploads at scale: many long audiobooks and many small
//! files queued with compression on, against the local S3 server with a
//! shaped (slow) network and a real FFmpeg.
//!
//! Quick versions run by default and skip cleanly without FFmpeg. The
//! realistic sizes are ignored; run them with
//! `cargo test -p r2dock-transfer --release --test compress_scale -- --ignored --nocapture`
//! (measurements are recorded in docs/benchmarks/).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use r2dock_core::AppError;
use r2dock_core::connections::Jurisdiction;
use r2dock_core::parts::MIB;
use r2dock_core::policy::ConflictPolicy;
use r2dock_media::ffmpeg::Ffmpeg;
use r2dock_media::plan::AudioSettings;
use r2dock_r2::{ClientParams, R2Client};
use r2dock_testkit::{ACCESS_KEY, LocalS3, SECRET_KEY};
use r2dock_transfer::engine::{CompressionEvent, MediaRuntime};
use r2dock_transfer::{
    BoxFuture, ClientProvider, CompressionFailure, CompressionRequest, Engine, EngineConfig,
    JobAction, JobId, JobSpec, JobState, TransferEvent, UploadSpec,
};
use uuid::Uuid;

const BUCKET: &str = "scale";

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

fn ffmpeg_bin() -> Option<PathBuf> {
    let bin = std::env::var_os("R2DOCK_TEST_FFMPEG").map(PathBuf::from)?;
    let ok = Command::new(&bin)
        .arg("-version")
        .output()
        .is_ok_and(|o| o.status.success());
    if !ok {
        eprintln!("R2DOCK_TEST_FFMPEG does not run; skipping");
        return None;
    }
    Some(bin)
}

/// A synthetic speech-like mono MP3 like a typical audiobook: 24 kHz,
/// 48 kbps (compresses to the 32 kbps speech preset).
fn audiobook(bin: &Path, path: &Path, secs: u32) {
    let src = format!(
        "sine=f=220:r=24000:d={secs},volume=0.4[a];anoisesrc=r=24000:d={secs}:a=0.05[n];[a][n]amix=inputs=2"
    );
    let out = Command::new(bin)
        .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi"])
        .args(["-i", &src, "-c:a", "libmp3lame", "-b:a", "48k", "-ac", "1"])
        .args(["-ar", "24000"])
        .arg(path)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn upload(local: PathBuf, key: String, compress: bool) -> JobSpec {
    let size = std::fs::metadata(&local).unwrap().len();
    JobSpec::Upload(UploadSpec {
        connection: Uuid::nil(),
        bucket: BUCKET.into(),
        key,
        local,
        size,
        policy: ConflictPolicy::Replace,
        identity: None,
        compression: compress.then(|| CompressionRequest {
            settings: AudioSettings::default(),
            content: Default::default(),
            on_failure: CompressionFailure::UploadOriginal,
            video: Default::default(),
        }),
        compressed: None,
    })
}

struct Harness {
    s3: LocalS3,
    engine: Engine,
    data: tempfile::TempDir,
    files: tempfile::TempDir,
    work: PathBuf,
    events: Arc<Mutex<Vec<TransferEvent>>>,
    compression: Arc<Mutex<Vec<CompressionEvent>>>,
}

async fn harness(bin: Option<&Path>, encode_limit: usize, threshold: u64) -> Harness {
    let s3 = LocalS3::start(&[BUCKET]).await;
    let provider = Arc::new(Provider {
        endpoint: s3.endpoint.clone(),
    });
    let data = tempfile::tempdir().unwrap();
    let mut config = EngineConfig::new(data.path().to_path_buf());
    config.progress_interval = Duration::from_millis(50);
    config.multipart_threshold = threshold;
    let engine = Engine::start(provider, config);
    let work = data.path().join("compress");
    if let Some(bin) = bin {
        engine.set_media(Some(MediaRuntime::new(
            Ffmpeg::new(bin.to_path_buf()),
            work.clone(),
            encode_limit,
        )));
    }
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = events.clone();
    engine.subscribe(Arc::new(move |e| {
        // Progress ticks are many; the invariants here need the rest.
        if !matches!(e, TransferEvent::Progress { .. }) {
            sink.lock().push(e);
        }
    }));
    let compression = Arc::new(Mutex::new(Vec::new()));
    let sink = compression.clone();
    engine.observe_compression(Arc::new(move |e| sink.lock().push(e)));
    Harness {
        s3,
        engine,
        data,
        files: tempfile::tempdir().unwrap(),
        work,
        events,
        compression,
    }
}

fn count(engine: &Engine, ids: &[JobId], pred: impl Fn(JobState) -> bool) -> usize {
    let snap = engine.snapshot();
    ids.iter()
        .filter(|id| snap.iter().any(|j| j.id == **id && pred(j.state)))
        .count()
}

fn is_final(s: JobState) -> bool {
    matches!(
        s,
        JobState::Completed | JobState::Failed | JobState::Cancelled | JobState::Skipped
    )
}

async fn wait_until(limit: Duration, what: &str, mut ok: impl FnMut() -> bool) -> Duration {
    let start = Instant::now();
    while !ok() {
        assert!(start.elapsed() < limit, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    start.elapsed()
}

/// Samples the compression work folder in the background and keeps the
/// peak number and size of compressed copies (sample-test files excluded,
/// they are short-lived and small).
struct WorkWatch {
    stop: Arc<AtomicBool>,
    peak_files: Arc<AtomicU64>,
    peak_bytes: Arc<AtomicU64>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl WorkWatch {
    fn start(dir: PathBuf) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let peak_files = Arc::new(AtomicU64::new(0));
        let peak_bytes = Arc::new(AtomicU64::new(0));
        let thread = {
            let (stop, pf, pb) = (stop.clone(), peak_files.clone(), peak_bytes.clone());
            std::thread::spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    let (files, bytes) = copies(&dir);
                    pf.fetch_max(files, Ordering::Relaxed);
                    pb.fetch_max(bytes, Ordering::Relaxed);
                    std::thread::sleep(Duration::from_millis(5));
                }
            })
        };
        Self {
            stop,
            peak_files,
            peak_bytes,
            thread: Some(thread),
        }
    }

    fn finish(mut self) -> (u64, u64) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            t.join().unwrap();
        }
        (
            self.peak_files.load(Ordering::Relaxed),
            self.peak_bytes.load(Ordering::Relaxed),
        )
    }
}

/// Compressed copies (and encodes in progress) in the work folder.
fn copies(dir: &Path) -> (u64, u64) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return (0, 0);
    };
    let mut files = 0;
    let mut bytes = 0;
    for e in entries.flatten() {
        if e.file_name().to_string_lossy().contains(".sample.") {
            continue;
        }
        if let Ok(m) = e.metadata()
            && m.is_file()
        {
            files += 1;
            bytes += m.len();
        }
    }
    (files, bytes)
}

/// FFmpeg processes started by this test process that are still running.
fn ffmpeg_children() -> usize {
    if !cfg!(unix) {
        return 0;
    }
    let me = std::process::id().to_string();
    let out = Command::new("pgrep").args(["-P", &me]).output().unwrap();
    let pids = String::from_utf8_lossy(&out.stdout).to_string();
    pids.lines()
        .filter(|pid| {
            let comm = Command::new("ps")
                .args(["-o", "comm=", "-p", pid.trim()])
                .output()
                .unwrap();
            String::from_utf8_lossy(&comm.stdout).contains("ffmpeg")
        })
        .count()
}

/// Activity-log rows the app writes per job come from `Updated` events in
/// these states (see `record_job` in src-tauri) plus one row per
/// compression milestone. Progress never writes a row.
fn logged_rows_per_job(h: &Harness) -> HashMap<JobId, usize> {
    let mut rows: HashMap<JobId, usize> = HashMap::new();
    let mut last: HashMap<JobId, JobState> = HashMap::new();
    for e in h.events.lock().iter() {
        if let TransferEvent::Updated { job } = e {
            let logged = matches!(
                job.state,
                JobState::Completed
                    | JobState::Failed
                    | JobState::Cancelled
                    | JobState::Skipped
                    | JobState::NeedsDecision
                    | JobState::Paused
                    | JobState::Retrying
                    | JobState::Interrupted
            );
            if logged {
                *rows.entry(job.id).or_default() += 1;
            }
            last.insert(job.id, job.state);
        }
    }
    for e in h.compression.lock().iter() {
        let id = match e {
            CompressionEvent::Started { id, .. }
            | CompressionEvent::Finished { id, .. }
            | CompressionEvent::Unchanged { id, .. }
            | CompressionEvent::Failed { id, .. } => *id,
        };
        *rows.entry(id).or_default() += 1;
    }
    rows
}

/// With uploads slower than encodes, finished copies must not pile up in
/// the work folder: at most one per upload slot plus one per encoder.
#[tokio::test(flavor = "multi_thread")]
async fn copies_waiting_for_upload_stay_bounded() {
    let Some(bin) = ffmpeg_bin() else { return };
    let h = harness(Some(&bin), 1, 100 * MIB).await;
    let first = h.files.path().join("book-00.mp3");
    audiobook(&bin, &first, 200);
    let mut specs = Vec::new();
    for i in 0..12 {
        let path = h.files.path().join(format!("book-{i:02}.mp3"));
        if i > 0 {
            std::fs::copy(&first, &path).unwrap();
        }
        specs.push(upload(path, format!("books/book-{i:02}.mp3"), true));
    }
    // About 4 s per copy upload (800 KB), shared, slower than an encode.
    h.s3.network.set_upload_bps(600_000);
    let watch = WorkWatch::start(h.work.clone());
    let started = Instant::now();
    let ids = h.engine.enqueue(specs, None);
    wait_until(Duration::from_secs(120), "all uploads", || {
        count(&h.engine, &ids, is_final) == ids.len()
    })
    .await;
    let (peak_files, peak_bytes) = watch.finish();
    eprintln!(
        "12 books: {:.1} s, peak {peak_files} copies, {peak_bytes} bytes",
        started.elapsed().as_secs_f64()
    );
    assert_eq!(count(&h.engine, &ids, |s| s == JobState::Completed), 12);
    for j in h.engine.snapshot() {
        assert!(
            matches!(
                j.compression,
                Some(r2dock_transfer::CompressionView::Saved { .. })
            ),
            "{:?}",
            j.compression
        );
    }
    // 3 upload slots + 1 encoder.
    assert!(peak_files <= 4, "peak {peak_files} copies on disk");
    assert_eq!(copies(&h.work).0, 0, "every copy is deleted after upload");
    for (id, rows) in logged_rows_per_job(&h) {
        // compress_started, compress_finished, transfer_completed.
        assert!(rows <= 3, "{id}: {rows} activity rows");
    }
}

/// Uploads that need no compression go ahead while a long encode runs,
/// even when hundreds of files waiting for the encoder are ahead of them.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn a_long_encode_does_not_hold_back_other_uploads() {
    let Some(bin) = ffmpeg_bin() else { return };
    let tools = tempfile::tempdir().unwrap();
    // An "FFmpeg" that never finishes: the encoder stays busy.
    let slow = tools.path().join("ffmpeg");
    std::fs::write(&slow, "#!/bin/sh\nexec sleep 600\n").unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&slow, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let h = harness(Some(&slow), 1, 100 * MIB).await;
    let book = h.files.path().join("book.mp3");
    audiobook(&bin, &book, 200);
    let mut specs = Vec::new();
    for i in 0..300 {
        let path = h.files.path().join(format!("b{i:03}.mp3"));
        std::fs::hard_link(&book, &path).unwrap();
        specs.push(upload(path, format!("books/b{i:03}.mp3"), true));
    }
    let mut small = Vec::new();
    for i in 0..50 {
        let path = h.files.path().join(format!("n{i:03}.txt"));
        std::fs::write(&path, format!("note {i}")).unwrap();
        small.push(upload(path, format!("notes/n{i:03}.txt"), false));
    }
    let _books = h.engine.enqueue(specs, None);
    let notes = h.engine.enqueue(small, None);
    let took = wait_until(Duration::from_secs(30), "the small uploads", || {
        count(&h.engine, &notes, |s| s == JobState::Completed) == notes.len()
    })
    .await;
    eprintln!("50 small files behind 300 waiting encodes: {took:?}");
    h.engine.control_many(&_books, JobAction::Cancel);
    wait_until(Duration::from_secs(10), "cancel", || {
        count(&h.engine, &_books, is_final) == _books.len()
    })
    .await;
    wait_until(Duration::from_secs(5), "FFmpeg to exit", || {
        ffmpeg_children() == 0
    })
    .await;
}

/// Cancel and pause on a big queue with encodes and multipart uploads in
/// progress, on a slow network where aborting an upload takes seconds.
/// Every job must settle promptly, and nothing may be left behind.
#[tokio::test(flavor = "multi_thread")]
async fn cancel_all_mid_compress_and_mid_upload() {
    let Some(bin) = ffmpeg_bin() else { return };
    let h = harness(Some(&bin), 2, 8 * MIB).await;
    // Long enough that encodes are still running when cancel arrives.
    let book = h.files.path().join("book.mp3");
    audiobook(&bin, &book, 3600);
    // Big files that skip compression keep multipart uploads busy.
    let big = h.files.path().join("big.bin");
    std::fs::write(&big, vec![7u8; usize::try_from(40 * MIB).unwrap()]).unwrap();
    let mut specs = Vec::new();
    for i in 0..6 {
        let path = h.files.path().join(format!("big{i}.bin"));
        std::fs::hard_link(&big, &path).unwrap();
        specs.push(upload(path, format!("big/{i}.bin"), false));
    }
    for i in 0..94 {
        let path = h.files.path().join(format!("b{i:03}.mp3"));
        std::fs::hard_link(&book, &path).unwrap();
        specs.push(upload(path, format!("books/b{i:03}.mp3"), true));
    }
    h.s3.network.set_upload_bps(8_000_000);
    h.s3.network.set_abort_delay(Duration::from_secs(5));
    let ids = h.engine.enqueue(specs, None);
    wait_until(Duration::from_secs(60), "encodes and uploads", || {
        let snap = h.engine.snapshot();
        let compressing = snap
            .iter()
            .filter(|j| j.state == JobState::Compressing)
            .count();
        let parts = snap
            .iter()
            .filter(|j| j.state == JobState::Running && j.parts_total.is_some())
            .count();
        compressing >= 2 && parts >= 3
    })
    .await;

    // Pause everything first (a user who changes their mind), then cancel.
    let paused_at = Instant::now();
    h.engine.pause_all();
    let pause_took = wait_until(Duration::from_secs(60), "pause all", || {
        h.engine
            .snapshot()
            .iter()
            .all(|j| !matches!(j.state, JobState::Compressing) && !j.state.is_active())
    })
    .await;
    eprintln!(
        "pause all on 100 jobs: {pause_took:?} ({} paused)",
        count(&h.engine, &ids, |s| s == JobState::Paused)
    );
    assert!(
        paused_at.elapsed() < Duration::from_secs(8),
        "pause took {pause_took:?}"
    );
    assert_eq!(ffmpeg_children(), 0, "pause stops every encode");

    // Resume, let work start again, then cancel everything.
    h.engine.control_many(&ids, JobAction::Resume);
    // (Running multipart uploads have an upload id that cancel must abort.)
    wait_until(Duration::from_secs(60), "work again", || {
        let snap = h.engine.snapshot();
        snap.iter().any(|j| j.state == JobState::Compressing)
            && snap
                .iter()
                .any(|j| j.state == JobState::Running && j.parts_total.is_some())
    })
    .await;
    let aborts_before = h.s3.network.aborts();
    let cancel_took = {
        let start = Instant::now();
        h.engine.control_many(&ids, JobAction::Cancel);
        wait_until(Duration::from_secs(60), "cancel all", || {
            count(&h.engine, &ids, |s| s == JobState::Cancelled) == ids.len()
        })
        .await;
        start.elapsed()
    };
    eprintln!("cancel all on 100 jobs: {cancel_took:?}");
    assert!(
        cancel_took < Duration::from_secs(2),
        "cancel took {cancel_took:?} with a 5 s abort"
    );
    // The aborts still reach the server, in the background.
    wait_until(Duration::from_secs(20), "aborts", || {
        h.s3.network.aborts() > aborts_before
    })
    .await;
    wait_until(Duration::from_secs(5), "FFmpeg to exit", || {
        ffmpeg_children() == 0
    })
    .await;
    wait_until(Duration::from_secs(5), "copies to go", || {
        copies(&h.work).0 == 0
    })
    .await;
    let _ = &h.data;
}

/// Pausing a multipart upload on a slow link waits for the parts in flight
/// only for a short grace time, then stops them; resume finishes the file.
#[tokio::test(flavor = "multi_thread")]
async fn pause_on_a_slow_link_is_prompt_and_resume_completes() {
    let h = harness(None, 1, 8 * MIB).await;
    let mut specs = Vec::new();
    let mut originals = Vec::new();
    for i in 0..3u8 {
        let path = h.files.path().join(format!("v{i}.bin"));
        let data: Vec<u8> = (0..64 * MIB).map(|b| (b % 251) as u8 ^ i).collect();
        std::fs::write(&path, &data).unwrap();
        originals.push(data);
        specs.push(upload(path, format!("v/{i}.bin"), false));
    }
    // 8 parts of 16 MiB in flight at 2 MB/s would take a minute to finish.
    h.s3.network.set_upload_bps(2_000_000);
    let ids = h.engine.enqueue(specs, None);
    wait_until(Duration::from_secs(60), "uploads to run", || {
        count(&h.engine, &ids, |s| s == JobState::Running) == 3
    })
    .await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    let start = Instant::now();
    h.engine.pause_all();
    let took = wait_until(Duration::from_secs(120), "pause", || {
        count(&h.engine, &ids, |s| s == JobState::Paused) == 3
    })
    .await;
    eprintln!("pause 3 multipart uploads on a 2 MB/s link: {took:?}");
    assert!(
        start.elapsed() < Duration::from_secs(8),
        "pause took {took:?}"
    );

    h.s3.network.set_upload_bps(0);
    h.engine.control_many(&ids, JobAction::Resume);
    wait_until(Duration::from_secs(120), "resume", || {
        count(&h.engine, &ids, is_final) == 3
    })
    .await;
    assert_eq!(count(&h.engine, &ids, |s| s == JobState::Completed), 3);
    let client = Provider {
        endpoint: h.s3.endpoint.clone(),
    }
    .client(Uuid::nil())
    .await
    .unwrap();
    for (i, data) in originals.iter().enumerate() {
        let out = client
            .get_object(BUCKET, &format!("v/{i}.bin"), None)
            .await
            .unwrap();
        let got = out.body.collect().await.unwrap().into_bytes();
        assert!(got.as_ref() == data.as_slice(), "file {i} differs");
    }
}

fn env_num(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// FFmpeg children of this process: (count, total CPU percent, threads).
#[cfg(unix)]
fn ffmpeg_load() -> (usize, f64, usize) {
    let me = std::process::id().to_string();
    let Ok(out) = Command::new("ps")
        .args(["-A", "-o", "pid=,ppid=,%cpu=,comm="])
        .output()
    else {
        return (0, 0.0, 0);
    };
    let mut n = 0;
    let mut cpu = 0.0;
    let mut threads = 0;
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        let cols: Vec<&str> = line.split_whitespace().collect();
        if cols.len() < 4 || cols[1] != me || !cols[3].contains("ffmpeg") {
            continue;
        }
        n += 1;
        cpu += cols[2].parse::<f64>().unwrap_or(0.0);
        // `ps -M` prints one line per thread after a header.
        if let Ok(t) = Command::new("ps").args(["-M", "-p", cols[0]]).output() {
            threads += String::from_utf8_lossy(&t.stdout)
                .lines()
                .count()
                .saturating_sub(1);
        }
    }
    (n, cpu, threads)
}

/// The realistic case: several multi-hour mono audiobooks queued at once on
/// a limited uplink. Needs a long MP3 made by the caller (synthetic only):
/// `R2DOCK_SCALE_BOOK=/path/book.mp3`, optionally `R2DOCK_SCALE_COUNT`
/// (default 3), `R2DOCK_SCALE_BPS` (uplink bytes per second, default
/// 5,000,000), `R2DOCK_SCALE_ENCODERS` (default 1).
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
#[ignore = "realistic sizes: hours of audio"]
async fn realistic_long_audiobooks() {
    let Some(bin) = ffmpeg_bin() else { return };
    let Some(book) = std::env::var_os("R2DOCK_SCALE_BOOK").map(PathBuf::from) else {
        eprintln!("R2DOCK_SCALE_BOOK not set; skipping");
        return;
    };
    let count = usize::try_from(env_num("R2DOCK_SCALE_COUNT", 3)).unwrap();
    let bps = env_num("R2DOCK_SCALE_BPS", 5_000_000);
    let encoders = usize::try_from(env_num("R2DOCK_SCALE_ENCODERS", 1)).unwrap();
    let h = harness(Some(&bin), encoders, 100 * MIB).await;
    let mut specs = Vec::new();
    for i in 0..count {
        let path = h.files.path().join(format!("book-{i:03}.mp3"));
        std::fs::hard_link(&book, &path)
            .or_else(|_| std::fs::copy(&book, &path).map(|_| ()))
            .unwrap();
        specs.push(upload(path, format!("books/book-{i:03}.mp3"), true));
    }
    h.s3.network.set_upload_bps(bps);
    let original = std::fs::metadata(&book).unwrap().len();

    let stop = Arc::new(AtomicBool::new(false));
    let sampler = {
        let stop = stop.clone();
        std::thread::spawn(move || {
            let mut samples = Vec::new();
            while !stop.load(Ordering::Relaxed) {
                samples.push(ffmpeg_load());
                std::thread::sleep(Duration::from_millis(1000));
            }
            samples
        })
    };
    let watch = WorkWatch::start(h.work.clone());
    let started = Instant::now();
    let ids = h.engine.enqueue(specs, None);
    wait_until(Duration::from_secs(6 * 3600), "all uploads", || {
        count_final(&h.engine, &ids) == ids.len()
    })
    .await;
    let total = started.elapsed();
    let (peak_files, peak_bytes) = watch.finish();
    stop.store(true, Ordering::Relaxed);
    let samples = sampler.join().unwrap();

    let busy: Vec<&(usize, f64, usize)> = samples.iter().filter(|s| s.0 > 0).collect();
    let peak_procs = samples.iter().map(|s| s.0).max().unwrap_or(0);
    #[allow(clippy::cast_precision_loss)]
    let avg_cpu = busy.iter().map(|s| s.1).sum::<f64>() / busy.len().max(1) as f64;
    let max_threads = samples.iter().map(|s| s.2).max().unwrap_or(0);
    let finished: Vec<(f64, u64)> = h
        .compression
        .lock()
        .iter()
        .filter_map(|e| match e {
            CompressionEvent::Finished { secs, bytes, .. } => Some((*secs, *bytes)),
            _ => None,
        })
        .collect();
    eprintln!(
        "{count} books of {original} bytes, uplink {bps} B/s, {encoders} encoder(s): total {total:?}"
    );
    for (secs, bytes) in &finished {
        eprintln!("  compressed to {bytes} bytes in {secs:.1} s");
    }
    eprintln!(
        "  peak {peak_files} copies ({peak_bytes} bytes) in the work folder; \
         peak {peak_procs} FFmpeg processes, average {avg_cpu:.0}% CPU while encoding, \
         at most {max_threads} FFmpeg threads"
    );
    let rows = logged_rows_per_job(&h);
    let most = rows.values().copied().max().unwrap_or(0);
    eprintln!(
        "  activity rows: {} for {count} uploads (at most {most} per job)",
        rows.values().sum::<usize>()
    );
    assert_eq!(
        count_where(&h.engine, &ids, JobState::Completed),
        count,
        "every book uploads"
    );
    assert!(
        peak_files <= 3 + encoders as u64,
        "peak {peak_files} copies"
    );
    assert!(peak_procs <= encoders, "{peak_procs} FFmpeg at once");
}

fn count_final(engine: &Engine, ids: &[JobId]) -> usize {
    count(engine, ids, is_final)
}

fn count_where(engine: &Engine, ids: &[JobId], state: JobState) -> usize {
    count(engine, ids, |s| s == state)
}

/// A restart with many compressed copies waiting: nothing is read at
/// startup; each copy is checked (SHA-256) only right before its own
/// upload, off the async runtime, so the first upload starts after one
/// check, not after all of them.
/// `R2DOCK_SCALE_COPIES` (default 20) copies of `R2DOCK_SCALE_COPY_MB`
/// (default 353) MB of synthetic data.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "realistic sizes: gigabytes of copies"]
async fn restart_with_many_held_copies() {
    use r2dock_transfer::ctx::ResumeState;
    use r2dock_transfer::journal::{Journal, JournalEntry};
    use r2dock_transfer::source::{SourceFile, digest_written};
    use r2dock_transfer::{CompressedCopy, CompressionResult, CopyDigest};

    let copies_n = usize::try_from(env_num("R2DOCK_SCALE_COPIES", 20)).unwrap();
    let mb = env_num("R2DOCK_SCALE_COPY_MB", 353);
    let s3 = LocalS3::start(&[BUCKET]).await;
    let data = tempfile::tempdir().unwrap();
    let files = tempfile::tempdir().unwrap();
    let work = data.path().join("compress");
    std::fs::create_dir_all(&work).unwrap();

    // One copy's bytes, written once and linked for the rest (same digest).
    let first = work.join("copy-000.mp3");
    {
        use std::io::Write as _;
        let mut f = std::io::BufWriter::new(std::fs::File::create(&first).unwrap());
        let chunk: Vec<u8> = (0..MIB).map(|i| (i * 2_654_435_761 % 251) as u8).collect();
        for _ in 0..mb {
            f.write_all(&chunk).unwrap();
        }
    }
    let t = Instant::now();
    let (written, sha256) = digest_written(&first).unwrap();
    let one_hash = t.elapsed();
    let mut entries = Vec::new();
    for i in 0..copies_n {
        let copy = work.join(format!("copy-{i:03}.mp3"));
        if i > 0 {
            std::fs::hard_link(&first, &copy).unwrap();
        }
        let (written, _) = (written, ());
        let original = files.path().join(format!("book-{i:03}.mp3"));
        std::fs::write(&original, format!("original {i}")).unwrap();
        let fingerprint = SourceFile::open(&original, None).unwrap().fingerprint;
        let key = format!("books/book-{i:03}.mp3");
        let spec = JobSpec::Upload(UploadSpec {
            connection: Uuid::nil(),
            bucket: BUCKET.into(),
            key: key.clone(),
            local: copy.clone(),
            size: written.len,
            policy: ConflictPolicy::Replace,
            identity: written.identity,
            compression: Some(CompressionRequest {
                settings: AudioSettings::default(),
                content: Default::default(),
                on_failure: CompressionFailure::UploadOriginal,
                video: Default::default(),
            }),
            compressed: Some(CompressionResult::Compressed(CompressedCopy {
                temp: copy,
                original_local: original,
                original_key: key,
                original_size: 530_000_000,
                original: fingerprint,
                preset: "mp3-32k".into(),
                output: Some(CopyDigest {
                    file: written,
                    sha256: sha256.clone(),
                }),
                original_identity: None,
            })),
        });
        entries.push(JournalEntry {
            id: Uuid::new_v4(),
            spec,
            group: None,
            resume: ResumeState::None,
            created_at: "2026-10-18T00:00:00Z".into(),
            seq: u64::try_from(i).unwrap() + 1,
        });
    }
    let transfers = data.path().join("transfers");
    let (journal, _) = Journal::load(transfers.join("journal.json"));
    journal.upsert_many(entries.clone());
    journal.flush();
    drop(journal);

    // "Restart".
    let provider = Arc::new(Provider {
        endpoint: s3.endpoint.clone(),
    });
    let t = Instant::now();
    let mut config = EngineConfig::new(data.path().to_path_buf());
    config.progress_interval = Duration::from_millis(50);
    let engine = Engine::start(provider, config);
    let restore = t.elapsed();
    let ids: Vec<JobId> = entries.iter().map(|e| e.id).collect();
    assert_eq!(count_where(&engine, &ids, JobState::Interrupted), copies_n);

    let t = Instant::now();
    engine.control_many(&ids, JobAction::Resume);
    let mut ticks = 0u32;
    let mut late = Duration::ZERO;
    let mut first_done = None;
    while count_final(&engine, &ids) < copies_n {
        let before = Instant::now();
        tokio::time::sleep(Duration::from_millis(10)).await;
        // How late the runtime wakes us: hashing must not block it.
        late = late.max(before.elapsed().saturating_sub(Duration::from_millis(10)));
        ticks += 1;
        if first_done.is_none() && count_where(&engine, &ids, JobState::Completed) > 0 {
            first_done = Some(t.elapsed());
        }
        assert!(t.elapsed() < Duration::from_secs(1800), "timed out");
    }
    let total = t.elapsed();
    eprintln!(
        "restart with {copies_n} copies of {mb} MiB: restore {restore:?}; one SHA-256 check {one_hash:?}; \
         first upload done after {first_done:?}, all checked and uploaded in {total:?}; \
         worst runtime wake delay {late:?} over {ticks} ticks"
    );
    assert_eq!(count_where(&engine, &ids, JobState::Completed), copies_n);
    assert!(restore < Duration::from_secs(1), "restore read the copies");
    assert!(
        late < Duration::from_millis(500),
        "runtime blocked {late:?}"
    );
}
