//! Integration tests with a real FFmpeg. They generate short test files with
//! FFmpeg's built-in sources, compress them, and check the results.
//!
//! FFmpeg is found through `R2DOCK_TEST_FFMPEG` or on `PATH` (tests only; the
//! app never runs FFmpeg from `PATH`). Tests skip cleanly when it is missing.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use r2dock_media::ffmpeg::run::{RunError, RunOptions, run};
use r2dock_media::ffmpeg::{EncodeError, Ffmpeg};
use r2dock_media::plan::{
    AudioDecision, AudioInputs, AudioSettings, AudioTarget, LosslessAction, Mode, SkipReason,
    plan_audio,
};
use r2dock_media::probe::{AudioCodec, AudioProbe, probe_audio};
use tokio_util::sync::CancellationToken;

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
    // Absolute path, as the app always uses.
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

/// Run FFmpeg directly to make a test input.
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

fn s(p: &Path) -> &str {
    p.to_str().unwrap()
}

/// Speech-like mono audio: a tone with noise.
const SPEECH: &str =
    "sine=f=220:r=22050:d=150,volume=0.4[a];anoisesrc=r=22050:d=150:a=0.05[n];[a][n]amix=inputs=2";

fn ffprobe_json(bin: &Path, file: &Path, what: &str) -> String {
    let probe = bin.with_file_name(if cfg!(windows) {
        "ffprobe.exe"
    } else {
        "ffprobe"
    });
    let out = Command::new(probe)
        .args(["-v", "error", "-print_format", "json", what, s(file)])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn compress(p: &AudioProbe, settings: &AudioSettings) -> AudioTarget {
    match plan_audio(p, settings, AudioInputs::default()) {
        AudioDecision::Compress(t) => t,
        AudioDecision::Skip(r) => panic!("unexpected skip {r:?}"),
    }
}

fn out_path(dir: &Path, name: &str, target: &AudioTarget) -> PathBuf {
    let key = r2dock_media::keys::output_key(name, target.extension.as_deref());
    dir.join(format!("out-{key}"))
}

async fn encode(
    ff: &Ffmpeg,
    input: &Path,
    out: &Path,
    t: &AudioTarget,
    p: &AudioProbe,
) -> AudioProbe {
    let mut fractions = Vec::new();
    let result = ff
        .encode(
            input,
            out,
            t,
            p.duration_secs.unwrap(),
            &CancellationToken::new(),
            |f| fractions.push(f),
        )
        .await
        .unwrap();
    assert!(
        fractions.windows(2).all(|w| w[0] <= w[1]),
        "progress only grows"
    );
    assert!(fractions.iter().all(|f| (0.0..=1.0).contains(f)));
    result
}

#[tokio::test]
async fn speech_mp3_keeps_tags_and_art_and_gets_a_marker() {
    let Some(bin) = ffmpeg_bin() else { return };
    let dir = tempfile::tempdir().unwrap();
    let art = dir.path().join("art.png");
    make(
        &bin,
        &[
            "-f",
            "lavfi",
            "-i",
            "color=c=orange:s=300x300",
            "-frames:v",
            "1",
            s(&art),
        ],
    );
    let input = dir.path().join("Chapter One.mp3");
    make(
        &bin,
        &[
            "-f",
            "lavfi",
            "-i",
            SPEECH,
            "-i",
            s(&art),
            "-map",
            "0:a",
            "-map",
            "1:v",
            "-c:a",
            "libmp3lame",
            "-b:a",
            "64k",
            "-ac",
            "1",
            "-c:v",
            "copy",
            "-disposition:v",
            "attached_pic",
            "-id3v2_version",
            "3",
            "-metadata",
            "title=Chapter One",
            "-metadata",
            "artist=Sample Reader",
            s(&input),
        ],
    );
    let p = probe_audio(&input).unwrap();
    assert_eq!((p.codec, p.channels), (AudioCodec::Mp3, Some(1)));
    assert!(p.picture_bytes > 0, "cover art found");
    let t = compress(&p, &AudioSettings::default());
    let out = out_path(dir.path(), "Chapter One.mp3", &t);
    let ff = Ffmpeg::new(bin.clone());
    let result = encode(&ff, &input, &out, &t, &p).await;

    assert_eq!(result.codec, AudioCodec::Mp3);
    assert_eq!(
        (result.channels, result.sample_rate),
        (Some(1), Some(22_050))
    );
    assert_eq!(
        result.marker.as_deref(),
        Some("mp3-32k"),
        "marker written and read back"
    );
    assert!(result.picture_bytes > 0, "cover art kept");
    let saved = 1.0 - result.size as f64 / p.size as f64;
    assert!((0.40..0.60).contains(&saved), "saved {saved}");
    let tags = ffprobe_json(&bin, &out, "-show_format");
    assert!(
        tags.contains("Chapter One") && tags.contains("Sample Reader"),
        "{tags}"
    );

    // A second pass leaves the file alone.
    assert!(matches!(
        plan_audio(&result, &AudioSettings::default(), AudioInputs::default()),
        AudioDecision::Skip(SkipReason::AlreadyCompressedByR2dock { .. })
    ));
}

#[tokio::test]
async fn aac_m4b_keeps_chapters() {
    let Some(bin) = ffmpeg_bin() else { return };
    let dir = tempfile::tempdir().unwrap();
    let meta = dir.path().join("chapters.txt");
    std::fs::write(
        &meta,
        ";FFMETADATA1\ntitle=Sample Book\n[CHAPTER]\nTIMEBASE=1/1000\nSTART=0\nEND=60000\ntitle=Part 1\n[CHAPTER]\nTIMEBASE=1/1000\nSTART=60000\nEND=240000\ntitle=Part 2\n",
    )
    .unwrap();
    let input = dir.path().join("book.m4b");
    make(
        &bin,
        &[
            "-f",
            "lavfi",
            "-i",
            "sine=f=330:r=44100:d=240",
            "-i",
            s(&meta),
            "-map",
            "0:a",
            "-map_metadata",
            "1",
            "-map_chapters",
            "1",
            "-c:a",
            "aac",
            "-b:a",
            "160k",
            "-ac",
            "1",
            "-f",
            "ipod",
            s(&input),
        ],
    );
    let p = probe_audio(&input).unwrap();
    assert_eq!(p.codec, AudioCodec::Aac);
    let t = compress(&p, &AudioSettings::default());
    assert_eq!(t.extension, None, "m4b stays m4b");
    let out = out_path(dir.path(), "book.m4b", &t);
    let result = encode(&Ffmpeg::new(bin.clone()), &input, &out, &t, &p).await;
    assert_eq!(result.codec, AudioCodec::Aac);
    let chapters = ffprobe_json(&bin, &out, "-show_chapters");
    assert!(
        chapters.contains("Part 1") && chapters.contains("Part 2"),
        "{chapters}"
    );
    let tags = ffprobe_json(&bin, &out, "-show_format");
    assert!(
        tags.contains("Sample Book"),
        "title kept as a normal tag: {tags}"
    );
    // No marker in MP4; the bitrate checks stop a second compression.
    assert_eq!(result.marker, None);
    let again = plan_audio(&result, &AudioSettings::default(), AudioInputs::default());
    eprintln!("second pass on m4b: {again:?}, probe {result:?}");
    assert!(matches!(
        again,
        AudioDecision::Skip(
            SkipReason::AlreadyCompressed { .. } | SkipReason::SavingTooSmall { .. }
        )
    ));
}

#[tokio::test]
async fn wav_to_mp3_flac_and_opus() {
    let Some(bin) = ffmpeg_bin() else { return };
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("Take 1.WAV");
    make(
        &bin,
        &[
            "-f",
            "lavfi",
            "-i",
            "sine=f=440:r=44100:d=20",
            "-f",
            "lavfi",
            "-i",
            "sine=f=660:r=44100:d=20",
            "-filter_complex",
            "[0:a][1:a]join=inputs=2:channel_layout=stereo",
            "-c:a",
            "pcm_s16le",
            s(&input),
        ],
    );
    let p = probe_audio(&input).unwrap();
    assert_eq!((p.codec, p.channels), (AudioCodec::Pcm, Some(2)));
    let ff = Ffmpeg::new(bin.clone());
    let cases = [
        (AudioSettings::default(), AudioCodec::Mp3, "mp3"),
        (
            AudioSettings {
                lossless: LosslessAction::Flac,
                ..AudioSettings::default()
            },
            AudioCodec::Flac,
            "flac",
        ),
        (
            AudioSettings {
                mode: Mode::Smallest,
                ..AudioSettings::default()
            },
            AudioCodec::Opus,
            "opus",
        ),
    ];
    for (settings, codec, ext) in cases {
        let t = compress(&p, &settings);
        assert_eq!(t.extension.as_deref(), Some(ext));
        let out = out_path(dir.path(), "Take 1.WAV", &t);
        assert!(out.to_string_lossy().ends_with(&format!("Take 1.{ext}")));
        let r = encode(&ff, &input, &out, &t, &p).await;
        assert_eq!(r.codec, codec, "{ext}");
        assert!(r.size < p.size, "{ext} is smaller");
        assert!(r.marker.is_some(), "{ext} carries the marker");
    }
}

#[tokio::test]
async fn detects_duplicated_stereo() {
    let Some(bin) = ffmpeg_bin() else { return };
    let dir = tempfile::tempdir().unwrap();
    let same = dir.path().join("same.wav");
    make(
        &bin,
        &["-f", "lavfi", "-i", "sine=f=440:d=40", "-ac", "2", s(&same)],
    );
    let real = dir.path().join("real.wav");
    make(
        &bin,
        &[
            "-f",
            "lavfi",
            "-i",
            "sine=f=440:d=40",
            "-f",
            "lavfi",
            "-i",
            "sine=f=550:d=40",
            "-filter_complex",
            "[0:a][1:a]join=inputs=2:channel_layout=stereo",
            s(&real),
        ],
    );
    let ff = Ffmpeg::new(bin);
    let token = CancellationToken::new();
    assert!(ff.is_dual_mono(&same, 40.0, &token).await.unwrap());
    assert!(!ff.is_dual_mono(&real, 40.0, &token).await.unwrap());
}

#[tokio::test]
async fn sample_test_predicts_the_saving() {
    let Some(bin) = ffmpeg_bin() else { return };
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("long.mp3");
    let long = SPEECH.replace("d=150", "d=600");
    make(
        &bin,
        &[
            "-f",
            "lavfi",
            "-i",
            &long,
            "-c:a",
            "libmp3lame",
            "-b:a",
            "64k",
            "-ac",
            "1",
            s(&input),
        ],
    );
    let p = probe_audio(&input).unwrap();
    let t = compress(&p, &AudioSettings::default());
    let sample = dir.path().join("sample.mp3");
    let saving = Ffmpeg::new(bin)
        .sample_saving(
            &input,
            &sample,
            &t,
            p.size,
            p.duration_secs.unwrap(),
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    assert!((40.0..60.0).contains(&saving), "{saving}");
    assert!(!sample.exists(), "sample file is deleted");
}

#[tokio::test]
async fn cancel_stops_ffmpeg_and_removes_the_output() {
    let Some(bin) = ffmpeg_bin() else { return };
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("hour.wav");
    make(
        &bin,
        &[
            "-f",
            "lavfi",
            "-i",
            "sine=f=300:r=44100:d=3600",
            "-ac",
            "2",
            s(&input),
        ],
    );
    let p = probe_audio(&input).unwrap();
    let t = compress(&p, &AudioSettings::default());
    let out = out_path(dir.path(), "hour.wav", &t);
    let token = CancellationToken::new();
    let trigger = token.clone();
    let started = std::time::Instant::now();
    let err = Ffmpeg::new(bin)
        .encode(&input, &out, &t, 3600.0, &token, move |f| {
            if f > 0.0 {
                trigger.cancel();
            }
        })
        .await
        .unwrap_err();
    assert!(err.is_cancelled(), "{err:?}");
    assert!(!out.exists(), "partial output deleted");
    assert!(started.elapsed() < Duration::from_secs(30));
}

#[tokio::test]
async fn failures_are_reported_without_folders() {
    let Some(bin) = ffmpeg_bin() else { return };
    let dir = tempfile::tempdir().unwrap();
    // Valid enough to plan, broken enough to fail: a WAV header with
    // garbage where the audio should be is fine for FFmpeg, so use a file
    // that is not audio at all with a hand-made target.
    let input = dir.path().join("private folder").join("broken.mp3");
    std::fs::create_dir_all(input.parent().unwrap()).unwrap();
    std::fs::write(&input, vec![0x55u8; 4096]).unwrap();
    let t = AudioTarget {
        encoding: r2dock_media::plan::Encoding::Mp3Cbr { kbps: 32 },
        channels: 1,
        sample_rate: Some(22_050),
        extension: None,
        content: r2dock_media::classify::Content::Speech,
        estimated_size: 0,
    };
    let out = dir.path().join("out.mp3");
    let err = Ffmpeg::new(bin)
        .encode(&input, &out, &t, 100.0, &CancellationToken::new(), |_| {})
        .await
        .unwrap_err();
    let msg = err.to_string();
    assert!(matches!(err, EncodeError::Failed(_)), "{err:?}");
    assert!(!msg.contains("private folder"), "{msg}");
    assert!(!out.exists());
}

#[cfg(unix)]
#[tokio::test]
async fn hostile_file_names_are_plain_files() {
    let Some(bin) = ffmpeg_bin() else { return };
    let dir = tempfile::tempdir().unwrap();
    let ff = Ffmpeg::new(bin.clone());
    for name in ["concat:a|b.mp3", "http:x.mp3", "-i.mp3"] {
        let input = dir.path().join(name);
        let tmp = dir.path().join("src.mp3");
        make(
            &bin,
            &[
                "-f",
                "lavfi",
                "-i",
                SPEECH,
                "-c:a",
                "libmp3lame",
                "-b:a",
                "64k",
                "-ac",
                "1",
                s(&tmp),
            ],
        );
        std::fs::rename(&tmp, &input).unwrap();
        let p = probe_audio(&input).unwrap();
        let t = compress(&p, &AudioSettings::default());
        let out = dir.path().join("out.mp3");
        let r = encode(&ff, &input, &out, &t, &p).await;
        assert_eq!(r.codec, AudioCodec::Mp3, "{name}");
        std::fs::remove_file(&out).unwrap();
    }
}

#[tokio::test]
async fn watchdog_stops_a_silent_process() {
    // A process that prints nothing stands in for a stuck FFmpeg.
    #[cfg(unix)]
    let (bin, args) = (PathBuf::from("/bin/sleep"), vec!["10".into()]);
    #[cfg(windows)]
    let (bin, args) = (
        PathBuf::from(r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe"),
        vec![
            "-NoProfile".into(),
            "-Command".into(),
            "Start-Sleep 10".into(),
        ],
    );
    let opts = RunOptions {
        watchdog: Duration::from_millis(500),
        keep_lines: 5,
    };
    let started = std::time::Instant::now();
    let err = run(&bin, &args, &CancellationToken::new(), opts, |_| {})
        .await
        .unwrap_err();
    assert!(matches!(err, RunError::Stalled), "{err:?}");
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[tokio::test]
async fn missing_binary_is_a_spawn_error() {
    let bin = std::env::temp_dir().join("no-such-ffmpeg-binary");
    let err = run(
        &bin,
        &[],
        &CancellationToken::new(),
        RunOptions::default(),
        |_| {},
    )
    .await
    .unwrap_err();
    assert!(matches!(err, RunError::Spawn(_)), "{err:?}");
}

#[tokio::test]
async fn inspects_ffmpeg_and_accepts_it_as_system_tools() {
    use r2dock_media::tools::detect::{inspect, use_system};
    let Some(bin) = ffmpeg_bin() else { return };
    let info = inspect(&bin).await.unwrap();
    assert!(!info.version.is_empty());
    assert!(
        info.audio.mp3 && info.audio.aac && info.audio.flac,
        "{info:?}"
    );
    let (tools, _) = use_system(&bin).await.unwrap();
    assert!(tools.ffprobe.is_file());
    eprintln!("FFmpeg {} ({:?})", info.version, info.license);
}

#[tokio::test]
async fn rejects_files_that_are_not_ffmpeg() {
    use r2dock_media::tools::detect::use_system;
    let dir = tempfile::tempdir().unwrap();
    let fake = dir.path().join("ffmpeg");
    std::fs::write(&fake, b"not a program").unwrap();
    assert!(use_system(&fake).await.is_err());
    assert!(use_system(dir.path()).await.is_err(), "a folder is refused");
    assert!(use_system(&dir.path().join("missing")).await.is_err());
}

mod pipeline {
    use super::*;
    use r2dock_media::classify::ContentOverride;
    use r2dock_media::pipeline::{PrepareJob, Prepared, Unchanged, prepare, upload_key};

    async fn run_prepare(ff: &Ffmpeg, input: &Path, work: &Path) -> Prepared {
        prepare(
            ff,
            PrepareJob {
                input,
                work_dir: work,
                stem: "job-1",
                settings: &AudioSettings::default(),
                content: ContentOverride::Auto,
            },
            &CancellationToken::new(),
            |_| {},
        )
        .await
        .unwrap()
    }

    fn files(dir: &Path) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(dir)
            .map(|r| {
                r.flatten()
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();
        v.sort();
        v
    }

    #[tokio::test]
    async fn compresses_speech_and_leaves_only_the_output() {
        let Some(bin) = ffmpeg_bin() else { return };
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("Talk.mp3");
        let long = SPEECH.replace("d=150", "d=400");
        make(
            &bin,
            &[
                "-f",
                "lavfi",
                "-i",
                &long,
                "-c:a",
                "libmp3lame",
                "-b:a",
                "64k",
                "-ac",
                "1",
                s(&input),
            ],
        );
        let work = dir.path().join("work");
        let Prepared::Compressed(c) = run_prepare(&Ffmpeg::new(bin), &input, &work).await else {
            panic!("expected compression");
        };
        assert_eq!(c.preset, "mp3-32k");
        assert_eq!(c.extension, None);
        assert!(c.size * 100 / c.original_size < 60, "{c:?}");
        assert_eq!(
            files(&work),
            vec!["job-1.mp3".to_string()],
            "sample removed"
        );
        assert_eq!(upload_key("talks/Talk.mp3", &c), "talks/Talk.mp3");
    }

    #[tokio::test]
    async fn leaves_small_and_non_audio_files_alone() {
        let Some(bin) = ffmpeg_bin() else { return };
        let dir = tempfile::tempdir().unwrap();
        let ff = Ffmpeg::new(bin.clone());
        let work = dir.path().join("work");
        let already = dir.path().join("already.mp3");
        let long = SPEECH.replace("d=150", "d=400");
        make(
            &bin,
            &[
                "-f",
                "lavfi",
                "-i",
                &long,
                "-c:a",
                "libmp3lame",
                "-b:a",
                "32k",
                "-ac",
                "1",
                s(&already),
            ],
        );
        let got = run_prepare(&ff, &already, &work).await;
        assert!(matches!(
            got,
            Prepared::Unchanged(Unchanged::Plan {
                reason: SkipReason::AlreadyCompressed { .. }
            })
        ));
        let png = dir.path().join("photo.png");
        make(
            &bin,
            &[
                "-f",
                "lavfi",
                "-i",
                "color=c=blue:s=64x64",
                "-frames:v",
                "1",
                s(&png),
            ],
        );
        assert_eq!(
            run_prepare(&ff, &png, &work).await,
            Prepared::Unchanged(Unchanged::NotAudio)
        );
        let text = dir.path().join("notes.mp3");
        std::fs::write(&text, b"plain text").unwrap();
        assert_eq!(
            run_prepare(&ff, &text, &work).await,
            Prepared::Unchanged(Unchanged::NotAudio)
        );
        assert!(files(&work).is_empty());
    }

    #[tokio::test]
    async fn duplicated_stereo_music_becomes_mono() {
        let Some(bin) = ffmpeg_bin() else { return };
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("song.mp3");
        make(
            &bin,
            &[
                "-f",
                "lavfi",
                "-i",
                "sine=f=440:r=44100:d=60",
                "-ac",
                "2",
                "-c:a",
                "libmp3lame",
                "-b:a",
                "256k",
                s(&input),
            ],
        );
        let work = dir.path().join("work");
        let Prepared::Compressed(c) = run_prepare(&Ffmpeg::new(bin), &input, &work).await else {
            panic!("expected compression");
        };
        let out = probe_audio(&c.output).unwrap();
        assert_eq!(out.channels, Some(1), "identical channels stored once");
    }
}

#[tokio::test]
async fn compare_quality_samples() {
    let Some(bin) = ffmpeg_bin() else { return };
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("talk.mp3");
    let long = SPEECH.replace("d=150", "d=200");
    make(
        &bin,
        &[
            "-f",
            "lavfi",
            "-i",
            &long,
            "-c:a",
            "libmp3lame",
            "-b:a",
            "64k",
            "-ac",
            "1",
            s(&input),
        ],
    );
    let out = dir.path().join("preview");
    std::fs::create_dir_all(&out).unwrap();
    std::fs::write(out.join("old.mp3"), b"stale").unwrap();
    let samples =
        r2dock_media::preview::compare(&Ffmpeg::new(bin), &input, &out, &CancellationToken::new())
            .await
            .unwrap();
    let labels: Vec<&str> = samples.iter().map(|s| s.label.as_str()).collect();
    assert_eq!(
        labels,
        [
            "Original",
            "Speech 32 kbps (MP3)",
            "Speech 48 kbps (MP3)",
            "Smallest (Opus 24 kbps)"
        ]
    );
    for s in &samples {
        assert!(s.path.is_file(), "{:?}", s.path);
        assert!(s.bytes_per_hour > 0);
    }
    // The original's real size: about 29 MB per hour at 64 kbps.
    let original = samples[0].bytes_per_hour as f64 / 1e6;
    assert!((26.0..32.0).contains(&original), "{original}");
    // About 14 MB per hour at 32 kbps.
    let mp3 = samples[1].bytes_per_hour as f64 / 1e6;
    assert!((12.0..17.0).contains(&mp3), "{mp3}");
    assert!(!out.join("old.mp3").exists(), "old samples removed");
}
