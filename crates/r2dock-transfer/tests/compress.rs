//! Compression before upload, end to end: local S3 server, real FFmpeg.
//! Tests skip cleanly when FFmpeg is missing (see r2dock-media's tests).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use r2dock_core::AppError;
use r2dock_core::connections::Jurisdiction;
use r2dock_core::policy::ConflictPolicy;
use r2dock_media::ffmpeg::Ffmpeg;
use r2dock_media::pipeline::Unchanged;
use r2dock_media::plan::AudioSettings;
use r2dock_r2::{ClientParams, R2Client};
use r2dock_testkit::{ACCESS_KEY, LocalS3, SECRET_KEY};
use r2dock_transfer::engine::{CompressionEvent, MediaRuntime};
use r2dock_transfer::{
    BoxFuture, ClientProvider, CompressionFailure, CompressionRequest, CompressionView, Engine,
    EngineConfig, JobAction, JobId, JobSnapshot, JobSpec, JobState, UploadSpec,
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

fn ffmpeg_bin() -> Option<PathBuf> {
    let bin = std::env::var_os("R2DOCK_TEST_FFMPEG")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("ffmpeg"));
    let ok = Command::new(&bin)
        .arg("-version")
        .output()
        .is_ok_and(|o| o.status.success());
    if !ok {
        eprintln!("FFmpeg not found; skipping");
        return None;
    }
    if bin.is_absolute() {
        return Some(bin);
    }
    let finder = if cfg!(windows) { "where" } else { "which" };
    let out = Command::new(finder).arg(&bin).output().ok()?;
    let first = String::from_utf8_lossy(&out.stdout)
        .lines()
        .next()?
        .trim()
        .to_string();
    Some(PathBuf::from(first))
}

fn make(bin: &Path, args: &[&str]) {
    let out = Command::new(bin)
        .args(["-hide_banner", "-loglevel", "error", "-y"])
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Mono speech-like MP3 at 64 kbps.
fn speech_mp3(bin: &Path, path: &Path, secs: u32) {
    let src = format!(
        "sine=f=220:r=22050:d={secs},volume=0.4[a];anoisesrc=r=22050:d={secs}:a=0.05[n];[a][n]amix=inputs=2"
    );
    make(
        bin,
        &[
            "-f",
            "lavfi",
            "-i",
            &src,
            "-c:a",
            "libmp3lame",
            "-b:a",
            "64k",
            "-ac",
            "1",
            path.to_str().unwrap(),
        ],
    );
}

struct Harness {
    _s3: LocalS3,
    engine: Engine,
    client: R2Client,
    data: tempfile::TempDir,
    files: tempfile::TempDir,
    work: PathBuf,
    events: Arc<Mutex<Vec<CompressionEvent>>>,
}

async fn harness_with(
    bin: Option<PathBuf>,
    data: Option<tempfile::TempDir>,
    s3: Option<LocalS3>,
) -> Harness {
    let s3 = match s3 {
        Some(s) => s,
        None => LocalS3::start(&[BUCKET]).await,
    };
    let provider = Arc::new(Provider {
        endpoint: s3.endpoint.clone(),
    });
    let client = provider.client(Uuid::nil()).await.unwrap();
    let data = data.unwrap_or_else(|| tempfile::tempdir().unwrap());
    let mut config = EngineConfig::new(data.path().to_path_buf());
    config.progress_interval = Duration::from_millis(20);
    let engine = Engine::start(provider, config);
    let work = data.path().join("compress");
    if let Some(bin) = bin {
        engine.set_media(Some(MediaRuntime {
            ffmpeg: Ffmpeg::new(bin),
            work_dir: work.clone(),
            encode_limit: 1,
        }));
    }
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = events.clone();
    engine.observe_compression(Arc::new(move |e| sink.lock().push(e)));
    Harness {
        _s3: s3,
        engine,
        client,
        data,
        files: tempfile::tempdir().unwrap(),
        work,
        events,
    }
}

fn upload(
    local: PathBuf,
    key: &str,
    policy: ConflictPolicy,
    on_failure: CompressionFailure,
) -> JobSpec {
    let size = std::fs::metadata(&local).unwrap().len();
    JobSpec::Upload(UploadSpec {
        connection: Uuid::nil(),
        bucket: BUCKET.into(),
        key: key.into(),
        local,
        size,
        policy,
        identity: None,
        compression: Some(CompressionRequest {
            settings: AudioSettings::default(),
            content: Default::default(),
            on_failure,
            video: Default::default(),
        }),
        compressed: None,
    })
}

fn job(engine: &Engine, id: JobId) -> JobSnapshot {
    engine.snapshot().into_iter().find(|j| j.id == id).unwrap()
}

async fn wait_state(engine: &Engine, id: JobId, wanted: &[JobState]) -> JobSnapshot {
    for _ in 0..3000 {
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

async fn object(client: &R2Client, key: &str) -> Vec<u8> {
    let out = client.get_object(BUCKET, key, None).await.unwrap();
    out.body.collect().await.unwrap().into_bytes().to_vec()
}

fn work_files(dir: &Path) -> Vec<String> {
    std::fs::read_dir(dir)
        .map(|r| {
            r.flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default()
}

fn started(events: &Mutex<Vec<CompressionEvent>>) -> usize {
    events
        .lock()
        .iter()
        .filter(|e| matches!(e, CompressionEvent::Started { .. }))
        .count()
}

#[tokio::test(flavor = "multi_thread")]
async fn compresses_then_uploads_with_metadata() {
    let Some(bin) = ffmpeg_bin() else { return };
    let h = harness_with(Some(bin.clone()), None, None).await;
    let input = h.files.path().join("Talk One.mp3");
    speech_mp3(&bin, &input, 400);
    let original = std::fs::read(&input).unwrap();
    let ids = h.engine.enqueue(
        vec![upload(
            input.clone(),
            "talks/Talk One.mp3",
            ConflictPolicy::Replace,
            CompressionFailure::UploadOriginal,
        )],
        None,
    );
    let j = wait_state(&h.engine, ids[0], &[JobState::Completed, JobState::Failed]).await;
    assert_eq!(j.state, JobState::Completed, "{:?}", j.error);
    assert_eq!(j.key, "talks/Talk One.mp3", "MP3 keeps its key");
    let Some(CompressionView::Saved {
        original_bytes,
        bytes,
        preset,
    }) = j.compression.clone()
    else {
        panic!("{:?}", j.compression);
    };
    assert_eq!(original_bytes, original.len() as u64);
    assert!(
        bytes * 100 / original_bytes < 60,
        "{bytes} of {original_bytes}"
    );
    assert_eq!(preset, "mp3-32k");
    assert!(
        j.local_display.ends_with("Talk One.mp3"),
        "shows the original: {}",
        j.local_display
    );

    let stored = object(&h.client, "talks/Talk One.mp3").await;
    assert_eq!(stored.len() as u64, bytes);
    let head = h
        .client
        .head_object(BUCKET, "talks/Talk One.mp3")
        .await
        .unwrap();
    assert_eq!(
        head.metadata.get("r2dock-compression").map(String::as_str),
        Some("mp3-32k")
    );
    assert_eq!(
        head.metadata
            .get("r2dock-original-size")
            .map(String::as_str),
        Some(original.len().to_string().as_str())
    );
    assert_eq!(
        head.metadata
            .get("r2dock-original-name")
            .map(String::as_str),
        Some("Talk%20One.mp3")
    );

    assert_eq!(
        std::fs::read(&input).unwrap(),
        original,
        "the original is never modified"
    );
    assert!(
        work_files(&h.work).is_empty(),
        "copy deleted: {:?}",
        work_files(&h.work)
    );
    let events = h.events.lock().clone();
    assert!(matches!(
        events.first(),
        Some(CompressionEvent::Started { .. })
    ));
    assert!(
        events
            .iter()
            .any(|e| matches!(e, CompressionEvent::Finished { preset, .. } if preset == "mp3-32k"))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn wav_becomes_mp3_under_a_new_key() {
    let Some(bin) = ffmpeg_bin() else { return };
    let h = harness_with(Some(bin.clone()), None, None).await;
    let input = h.files.path().join("Take.WAV");
    make(
        &bin,
        &[
            "-f",
            "lavfi",
            "-i",
            "sine=f=440:r=44100:d=30",
            "-ac",
            "2",
            "-c:a",
            "pcm_s16le",
            input.to_str().unwrap(),
        ],
    );
    let ids = h.engine.enqueue(
        vec![upload(
            input,
            "raw/Take.WAV",
            ConflictPolicy::Ask,
            CompressionFailure::UploadOriginal,
        )],
        None,
    );
    let j = wait_state(&h.engine, ids[0], &[JobState::Completed, JobState::Failed]).await;
    assert_eq!(j.state, JobState::Completed, "{:?}", j.error);
    assert_eq!(j.key, "raw/Take.mp3");
    assert!(!object(&h.client, "raw/Take.mp3").await.is_empty());
    assert!(!h.client.exists(BUCKET, "raw/Take.WAV").await.unwrap());
}

#[tokio::test(flavor = "multi_thread")]
async fn non_audio_and_missing_tools_upload_the_original() {
    let bin = ffmpeg_bin();
    let h = harness_with(bin.clone(), None, None).await;
    let text = h.files.path().join("notes.txt");
    std::fs::write(&text, vec![b'x'; 2_000_000]).unwrap();
    let ids = h.engine.enqueue(
        vec![upload(
            text.clone(),
            "notes.txt",
            ConflictPolicy::Replace,
            CompressionFailure::UploadOriginal,
        )],
        None,
    );
    let j = wait_state(&h.engine, ids[0], &[JobState::Completed, JobState::Failed]).await;
    assert_eq!(j.state, JobState::Completed);
    let want = if bin.is_some() {
        Unchanged::NotAudio
    } else {
        Unchanged::ToolsMissing
    };
    assert_eq!(
        j.compression,
        Some(CompressionView::Unchanged { reason: want })
    );
    assert_eq!(
        object(&h.client, "notes.txt").await,
        std::fs::read(&text).unwrap()
    );

    // Without tools at all.
    let h = harness_with(None, None, None).await;
    let f = h.files.path().join("a.mp3");
    std::fs::write(&f, vec![1u8; 1000]).unwrap();
    let ids = h.engine.enqueue(
        vec![upload(
            f,
            "a.mp3",
            ConflictPolicy::Replace,
            CompressionFailure::UploadOriginal,
        )],
        None,
    );
    let j = wait_state(&h.engine, ids[0], &[JobState::Completed, JobState::Failed]).await;
    assert_eq!(j.state, JobState::Completed);
    assert_eq!(
        j.compression,
        Some(CompressionView::Unchanged {
            reason: Unchanged::ToolsMissing
        })
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn cancel_and_pause_while_compressing() {
    let Some(bin) = ffmpeg_bin() else { return };
    let h = harness_with(Some(bin.clone()), None, None).await;
    let input = h.files.path().join("hour.wav");
    make(
        &bin,
        &[
            "-f",
            "lavfi",
            "-i",
            "sine=f=300:r=44100:d=1800",
            "-ac",
            "2",
            input.to_str().unwrap(),
        ],
    );
    let spec = || {
        upload(
            input.clone(),
            "hour.wav",
            ConflictPolicy::Replace,
            CompressionFailure::UploadOriginal,
        )
    };

    let ids = h.engine.enqueue(vec![spec()], None);
    wait_state(&h.engine, ids[0], &[JobState::Compressing]).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    h.engine.control(ids[0], JobAction::Cancel).unwrap();
    wait_state(&h.engine, ids[0], &[JobState::Cancelled]).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(work_files(&h.work).is_empty(), "{:?}", work_files(&h.work));
    assert!(!h.client.exists(BUCKET, "hour.wav").await.unwrap());
    assert!(!h.client.exists(BUCKET, "hour.mp3").await.unwrap());

    let ids = h.engine.enqueue(vec![spec()], None);
    wait_state(&h.engine, ids[0], &[JobState::Compressing]).await;
    h.engine.control(ids[0], JobAction::Pause).unwrap();
    wait_state(&h.engine, ids[0], &[JobState::Paused]).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(work_files(&h.work).is_empty(), "partial encode deleted");
    h.engine.control(ids[0], JobAction::Resume).unwrap();
    let j = wait_state(&h.engine, ids[0], &[JobState::Completed, JobState::Failed]).await;
    assert_eq!(j.state, JobState::Completed, "{:?}", j.error);
    assert_eq!(j.key, "hour.mp3");
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn failure_policies() {
    let Some(bin) = ffmpeg_bin() else { return };
    // A "tool" that always fails stands in for a broken FFmpeg.
    let h = harness_with(Some(PathBuf::from("/usr/bin/false")), None, None).await;
    let input = h.files.path().join("talk.mp3");
    speech_mp3(&bin, &input, 150);
    let original = std::fs::read(&input).unwrap();
    let ids = h.engine.enqueue(
        vec![
            upload(
                input.clone(),
                "upload-original.mp3",
                ConflictPolicy::Replace,
                CompressionFailure::UploadOriginal,
            ),
            upload(
                input.clone(),
                "skip-file.mp3",
                ConflictPolicy::Replace,
                CompressionFailure::SkipFile,
            ),
        ],
        None,
    );
    let a = wait_state(&h.engine, ids[0], &[JobState::Completed, JobState::Failed]).await;
    assert_eq!(a.state, JobState::Completed);
    assert!(
        matches!(a.compression, Some(CompressionView::Failed { .. })),
        "{:?}",
        a.compression
    );
    assert_eq!(object(&h.client, "upload-original.mp3").await, original);
    let b = wait_state(&h.engine, ids[1], &[JobState::Completed, JobState::Failed]).await;
    assert_eq!(b.state, JobState::Failed);
    let msg = b.error.unwrap().message;
    assert!(msg.starts_with("Compression failed"), "{msg}");
    assert!(
        !msg.contains(h.files.path().to_str().unwrap()),
        "no folders in messages: {msg}"
    );
    assert!(!h.client.exists(BUCKET, "skip-file.mp3").await.unwrap());
}

/// After compression, a conflict holds the job (policy Ask), the app
/// restarts, and the upload continues from the saved copy without
/// compressing again. Then an edited original is compressed again.
#[tokio::test(flavor = "multi_thread")]
async fn restart_reuses_the_copy_and_edits_compress_again() {
    let Some(bin) = ffmpeg_bin() else { return };
    let h = harness_with(Some(bin.clone()), None, None).await;
    h.client
        .sdk()
        .put_object()
        .bucket(BUCKET)
        .key("book.mp3")
        .body(b"old".to_vec().into())
        .send()
        .await
        .unwrap();
    let input = h.files.path().join("book.mp3");
    speech_mp3(&bin, &input, 150);
    let ids = h.engine.enqueue(
        vec![upload(
            input.clone(),
            "book.mp3",
            ConflictPolicy::Ask,
            CompressionFailure::UploadOriginal,
        )],
        None,
    );
    wait_state(&h.engine, ids[0], &[JobState::NeedsDecision]).await;
    assert_eq!(work_files(&h.work).len(), 1, "copy kept while waiting");
    h.engine.flush();

    // "Restart": a new engine on the same data folder and server.
    let Harness {
        _s3: s3,
        engine,
        data,
        files,
        ..
    } = h;
    drop(engine);
    let h2 = harness_with(Some(bin.clone()), Some(data), Some(s3)).await;
    let j = wait_state(&h2.engine, ids[0], &[JobState::Interrupted]).await;
    assert!(matches!(j.compression, Some(CompressionView::Saved { .. })));
    h2.engine.control(ids[0], JobAction::Resume).unwrap();
    wait_state(&h2.engine, ids[0], &[JobState::NeedsDecision]).await;
    h2.engine
        .control(ids[0], JobAction::Resolve(ConflictPolicy::Replace))
        .unwrap();
    let j = wait_state(&h2.engine, ids[0], &[JobState::Completed, JobState::Failed]).await;
    assert_eq!(j.state, JobState::Completed, "{:?}", j.error);
    assert_eq!(started(&h2.events), 0, "the saved copy was reused");
    assert!(object(&h2.client, "book.mp3").await.len() > 3);

    // An original edited after compression is compressed again.
    let input2 = files.path().join("edited.mp3");
    speech_mp3(&bin, &input2, 150);
    h2.client
        .sdk()
        .put_object()
        .bucket(BUCKET)
        .key("edited.mp3")
        .body(b"old".to_vec().into())
        .send()
        .await
        .unwrap();
    let ids = h2.engine.enqueue(
        vec![upload(
            input2.clone(),
            "edited.mp3",
            ConflictPolicy::Ask,
            CompressionFailure::UploadOriginal,
        )],
        None,
    );
    wait_state(&h2.engine, ids[0], &[JobState::NeedsDecision]).await;
    std::thread::sleep(Duration::from_millis(20));
    speech_mp3(&bin, &input2, 160);
    h2.engine
        .control(ids[0], JobAction::Resolve(ConflictPolicy::Replace))
        .unwrap();
    let j = wait_state(&h2.engine, ids[0], &[JobState::Completed, JobState::Failed]).await;
    assert_eq!(j.state, JobState::Completed, "{:?}", j.error);
    assert_eq!(started(&h2.events), 2, "compressed again after the edit");
}

/// Hold a compressed upload at a conflict (policy Ask) and return the path
/// and bytes of its copy.
async fn held_copy(h: &Harness, bin: &Path, key: &str) -> (JobId, PathBuf, Vec<u8>) {
    held_copy_with(h, bin, key, CompressionFailure::UploadOriginal).await
}

async fn held_copy_with(
    h: &Harness,
    bin: &Path,
    key: &str,
    on_failure: CompressionFailure,
) -> (JobId, PathBuf, Vec<u8>) {
    h.client
        .sdk()
        .put_object()
        .bucket(BUCKET)
        .key(key)
        .body(b"old".to_vec().into())
        .send()
        .await
        .unwrap();
    let input = h.files.path().join(key);
    speech_mp3(bin, &input, 150);
    let ids = h.engine.enqueue(
        vec![upload(input, key, ConflictPolicy::Ask, on_failure)],
        None,
    );
    wait_state(&h.engine, ids[0], &[JobState::NeedsDecision]).await;
    let names = work_files(&h.work);
    assert_eq!(names.len(), 1);
    let copy = h.work.join(&names[0]);
    let bytes = std::fs::read(&copy).unwrap();
    (ids[0], copy, bytes)
}

/// A copy that changed after it was checked (replaced by other bytes,
/// altered without changing its size, or altered with its modification time
/// put back) is never uploaded: it is compressed again from the unchanged
/// original. The job never fails because of it, even with "Skip file".
#[tokio::test(flavor = "multi_thread")]
async fn a_changed_copy_is_compressed_again() {
    let Some(bin) = ffmpeg_bin() else { return };
    for (case, on_failure) in [
        ("replaced", CompressionFailure::UploadOriginal),
        ("same size", CompressionFailure::UploadOriginal),
        ("same size and time", CompressionFailure::SkipFile),
    ] {
        let h = harness_with(Some(bin.clone()), None, None).await;
        let states = Arc::new(Mutex::new(Vec::new()));
        let sink = states.clone();
        h.engine.subscribe(Arc::new(move |e| {
            if let r2dock_transfer::TransferEvent::Updated { job } = e {
                sink.lock().push(job.state);
            }
        }));
        let (id, copy, valid) = held_copy_with(&h, &bin, "talk.mp3", on_failure).await;
        let mtime = std::fs::metadata(&copy).unwrap().modified().unwrap();
        let broken = if case == "replaced" {
            b"BROKEN MEDIA".to_vec()
        } else {
            let mut b = valid.clone();
            let mid = b.len() / 2;
            b[mid] ^= 0xFF;
            b
        };
        std::fs::write(&copy, &broken).unwrap();
        if case == "same size and time" {
            let f = std::fs::OpenOptions::new().write(true).open(&copy).unwrap();
            f.set_modified(mtime).unwrap();
        }
        h.engine
            .control(id, JobAction::Resolve(ConflictPolicy::Replace))
            .unwrap();
        let j = wait_state(&h.engine, id, &[JobState::Completed, JobState::Failed]).await;
        assert_eq!(j.state, JobState::Completed, "{case}: {:?}", j.error);
        assert!(
            !states.lock().contains(&JobState::Failed),
            "{case}: never failed"
        );
        let uploaded = object(&h.client, "talk.mp3").await;
        assert_ne!(uploaded, broken, "{case}");
        assert_eq!(uploaded, valid, "{case}");
        assert_eq!(started(&h.events), 2, "{case}: compressed again");
    }
}

/// A copy saved by an older version has no digest; after a restart it is
/// compressed again instead of being trusted.
#[tokio::test(flavor = "multi_thread")]
async fn a_copy_without_a_digest_is_compressed_again() {
    let Some(bin) = ffmpeg_bin() else { return };
    let h = harness_with(Some(bin.clone()), None, None).await;
    let (id, _copy, valid) = held_copy(&h, &bin, "book.mp3").await;
    h.engine.flush();
    let Harness {
        _s3: s3,
        engine,
        data,
        files,
        ..
    } = h;
    drop(engine);
    // Remove the digest, as an older version would have written it.
    let path = data.path().join("transfers").join("journal.json");
    let mut journal: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    let copy = &mut journal["entries"][0]["spec"]["compressed"];
    assert!(copy.get("output").is_some(), "the digest is saved");
    copy.as_object_mut().unwrap().remove("output");
    std::fs::write(&path, serde_json::to_vec(&journal).unwrap()).unwrap();

    let h2 = harness_with(Some(bin.clone()), Some(data), Some(s3)).await;
    let j = wait_state(&h2.engine, id, &[JobState::Interrupted]).await;
    assert!(matches!(
        j.compression,
        Some(CompressionView::Pending { .. })
    ));
    h2.engine
        .control(id, JobAction::Resolve(ConflictPolicy::Replace))
        .unwrap_err();
    h2.engine.control(id, JobAction::Resume).unwrap();
    wait_state(&h2.engine, id, &[JobState::NeedsDecision]).await;
    h2.engine
        .control(id, JobAction::Resolve(ConflictPolicy::Replace))
        .unwrap();
    let j = wait_state(&h2.engine, id, &[JobState::Completed, JobState::Failed]).await;
    assert_eq!(j.state, JobState::Completed, "{:?}", j.error);
    assert_eq!(started(&h2.events), 1, "compressed again");
    assert_eq!(object(&h2.client, "book.mp3").await, valid);
    drop(files);
}

#[tokio::test(flavor = "multi_thread")]
async fn video_upload_becomes_av1_mp4() {
    let Some(bin) = ffmpeg_bin() else { return };
    let probe = bin.with_file_name(if cfg!(windows) {
        "ffprobe.exe"
    } else {
        "ffprobe"
    });
    if !probe.is_file() {
        return;
    }
    let listed = {
        let out = Command::new(&bin)
            .args(["-hide_banner", "-encoders"])
            .output()
            .unwrap();
        r2dock_media::tools::detect::parse_encoders(&String::from_utf8_lossy(&out.stdout))
    };
    let encoders = r2dock_media::video::encoders::detect(&bin, &listed).await;
    if !encoders.av1 {
        eprintln!("no AV1 encoder; skipping");
        return;
    }
    let h = harness_with(None, None, None).await;
    h.engine.set_media(Some(MediaRuntime {
        ffmpeg: Ffmpeg::new(bin.clone()).with_video(probe, encoders),
        work_dir: h.work.clone(),
        encode_limit: 1,
    }));
    let input = h.files.path().join("Holiday.mkv");
    make(
        &bin,
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=s=1280x720:r=30:d=15",
            "-f",
            "lavfi",
            "-i",
            "sine=d=15",
            "-c:v",
            "mpeg4",
            "-q:v",
            "2",
            "-c:a",
            "aac",
            "-b:a",
            "256k",
            input.to_str().unwrap(),
        ],
    );
    let original = std::fs::metadata(&input).unwrap().len();
    let ids = h.engine.enqueue(
        vec![upload(
            input,
            "videos/Holiday.mkv",
            ConflictPolicy::Replace,
            CompressionFailure::UploadOriginal,
        )],
        None,
    );
    let j = wait_state(&h.engine, ids[0], &[JobState::Completed, JobState::Failed]).await;
    assert_eq!(j.state, JobState::Completed, "{:?}", j.error);
    assert_eq!(j.key, "videos/Holiday.mp4");
    let Some(CompressionView::Saved { bytes, preset, .. }) = j.compression else {
        panic!("{:?}", j.compression);
    };
    assert_eq!(preset, "av1-crf35");
    assert!(bytes * 2 < original);
    let head = h
        .client
        .head_object(BUCKET, "videos/Holiday.mp4")
        .await
        .unwrap();
    assert_eq!(
        head.metadata.get("r2dock-compression").map(String::as_str),
        Some("av1-crf35")
    );
    assert!(work_files(&h.work).is_empty());
}
