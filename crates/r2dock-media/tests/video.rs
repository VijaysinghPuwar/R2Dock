//! Video compression with a real FFmpeg and ffprobe. Each test skips when
//! FFmpeg, ffprobe, or the encoder it needs is missing on this machine.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::process::Command;

use r2dock_core::compression::{VideoCodec, VideoSettings, VideoSize};
use r2dock_media::classify::ContentOverride;
use r2dock_media::ffmpeg::Ffmpeg;
use r2dock_media::pipeline::{PrepareJob, Prepared, Unchanged, prepare};
use r2dock_media::plan::{AudioSettings, SkipReason};
use r2dock_media::tools::detect::parse_encoders;
use r2dock_media::video::encoders::detect;
use r2dock_media::video::plan::{H264Encoder, VideoEncoders};
use r2dock_media::video::probe::probe_video;
use tokio_util::sync::CancellationToken;

fn bins() -> Option<(PathBuf, PathBuf)> {
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
    let bin = if bin.is_absolute() {
        bin
    } else {
        let finder = if cfg!(windows) { "where" } else { "which" };
        let out = Command::new(finder).arg(&bin).output().ok()?;
        PathBuf::from(String::from_utf8_lossy(&out.stdout).lines().next()?.trim())
    };
    let probe = bin.with_file_name(if cfg!(windows) {
        "ffprobe.exe"
    } else {
        "ffprobe"
    });
    if !probe.is_file() {
        eprintln!("ffprobe not found; skipping");
        return None;
    }
    Some((bin, probe))
}

async fn tools() -> Option<(Ffmpeg, VideoEncoders)> {
    let (bin, probe) = bins()?;
    let out = Command::new(&bin)
        .args(["-hide_banner", "-encoders"])
        .output()
        .ok()?;
    let listed = parse_encoders(&String::from_utf8_lossy(&out.stdout));
    let found = detect(&bin, &listed).await;
    eprintln!("video encoders on this machine: {found:?}");
    Some((Ffmpeg::new(bin).with_video(probe, found), found))
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

fn s(p: &Path) -> &str {
    p.to_str().unwrap()
}

/// A busy 720p test video at a high bitrate with stereo audio (MPEG-4 Part
/// 2 is in every FFmpeg build, so this works without x264).
fn make_video(bin: &Path, out: &Path, secs: u32, extra: &[&str]) {
    let src = format!("testsrc2=s=1280x720:r=30:d={secs}");
    let tone = format!("sine=f=440:d={secs}");
    let mut args = vec![
        "-f", "lavfi", "-i", &src, "-f", "lavfi", "-i", &tone, "-c:v", "mpeg4", "-q:v", "2",
        "-c:a", "aac", "-b:a", "256k", "-ac", "2",
    ];
    args.extend_from_slice(extra);
    args.push(s(out));
    make(bin, &args);
}

async fn run(ff: &Ffmpeg, input: &Path, work: &Path, video: VideoSettings) -> Prepared {
    prepare(
        ff,
        PrepareJob {
            input,
            work_dir: work,
            stem: "job",
            settings: &AudioSettings::default(),
            video: &video,
            content: ContentOverride::Auto,
        },
        &CancellationToken::new(),
        |_| {},
    )
    .await
    .unwrap()
}

fn compressed(p: Prepared) -> r2dock_media::pipeline::Compressed {
    match p {
        Prepared::Compressed(c) => c,
        Prepared::Unchanged(u) => panic!("expected compression, got {u:?}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn av1_keeps_audio_tracks_and_subtitles() {
    let Some((ff, enc)) = tools().await else {
        return;
    };
    if !enc.av1 {
        eprintln!("no AV1 encoder; skipping");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let srt = dir.path().join("subs.srt");
    std::fs::write(&srt, "1\n00:00:01,000 --> 00:00:03,000\nHello\n").unwrap();
    let input = dir.path().join("clip.mkv");
    let src = "testsrc2=s=1280x720:r=30:d=20";
    make(
        &ff.bin,
        &[
            "-f",
            "lavfi",
            "-i",
            src,
            "-f",
            "lavfi",
            "-i",
            "sine=f=440:d=20",
            "-f",
            "lavfi",
            "-i",
            "sine=f=660:d=20",
            "-i",
            s(&srt),
            "-map",
            "0",
            "-map",
            "1",
            "-map",
            "2",
            "-map",
            "3",
            "-c:v",
            "mpeg4",
            "-q:v",
            "2",
            "-c:a",
            "aac",
            "-b:a",
            "256k",
            "-c:s",
            "srt",
            s(&input),
        ],
    );
    let c = compressed(run(&ff, &input, dir.path(), VideoSettings::default()).await);
    assert_eq!(c.extension.as_deref(), Some("mp4"), "MKV becomes MP4");
    assert_eq!(c.preset, "av1-crf35");
    assert!(c.size * 2 < c.original_size, "{c:?}");
    let out = probe_video(ff.ffprobe.as_ref().unwrap(), &c.output)
        .await
        .unwrap();
    assert_eq!(out.video().unwrap().codec, "av1");
    assert_eq!(out.audio().count(), 2, "both audio tracks kept");
    assert!(out.audio().all(|a| a.codec == "aac"));
    let subs: Vec<_> = out
        .streams
        .iter()
        .filter(|s| s.kind == r2dock_media::video::probe::StreamKind::Subtitle)
        .collect();
    assert_eq!(subs.len(), 1);
    assert_eq!(subs[0].codec, "mov_text");
}

#[tokio::test(flavor = "multi_thread")]
async fn phone_rotation_and_scaling() {
    let Some((ff, enc)) = tools().await else {
        return;
    };
    if !enc.av1 && enc.h264.is_none() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let flat = dir.path().join("flat.mp4");
    make_video(&ff.bin, &flat, 12, &[]);
    // A portrait phone video: landscape pixels with a 90 degree rotation tag.
    let input = dir.path().join("phone.mp4");
    make(
        &ff.bin,
        &[
            "-display_rotation",
            "90",
            "-i",
            s(&flat),
            "-c",
            "copy",
            s(&input),
        ],
    );
    let src = probe_video(ff.ffprobe.as_ref().unwrap(), &input)
        .await
        .unwrap();
    assert_eq!(
        src.video().unwrap().rotation.abs(),
        90,
        "test input has a rotation"
    );
    let settings = VideoSettings {
        codec: if enc.av1 {
            VideoCodec::Av1
        } else {
            VideoCodec::H264
        },
        size: VideoSize::Small720,
    };
    let mut big = dir.path().join("big.mp4");
    // Upscale to 1080p first so Small 720p has something to reduce.
    make(
        &ff.bin,
        &[
            "-i",
            s(&input),
            "-vf",
            "scale=1920:1080",
            "-c:v",
            "mpeg4",
            "-q:v",
            "2",
            "-c:a",
            "copy",
            s(&big),
        ],
    );
    if probe_video(ff.ffprobe.as_ref().unwrap(), &big)
        .await
        .unwrap()
        .video()
        .unwrap()
        .rotation
        == 0
    {
        big = input.clone();
    }
    let c = compressed(run(&ff, &big, dir.path(), settings).await);
    let out = probe_video(ff.ffprobe.as_ref().unwrap(), &c.output)
        .await
        .unwrap();
    let v = out.video().unwrap();
    assert_eq!(
        v.width.min(v.height),
        720,
        "short side reduced to 720: {v:?}"
    );
    // FFmpeg applies the rotation, so the picture itself is portrait (or the
    // rotation tag is kept); either way it is not sideways.
    let portrait = v.height > v.width;
    assert!(portrait || v.rotation.abs() == 90, "{v:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn hdr_and_off_are_left_alone() {
    let Some((ff, _)) = tools().await else { return };
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("hdr.mp4");
    make_video(
        &ff.bin,
        &input,
        12,
        &[
            "-vf",
            "setparams=color_trc=smpte2084:color_primaries=bt2020:colorspace=bt2020nc",
            "-movflags",
            "+write_colr",
        ],
    );
    assert_eq!(
        run(&ff, &input, dir.path(), VideoSettings::default()).await,
        Prepared::Unchanged(Unchanged::Plan {
            reason: SkipReason::Hdr
        })
    );
    let plain = dir.path().join("plain.mp4");
    make_video(&ff.bin, &plain, 12, &[]);
    let off = VideoSettings {
        size: VideoSize::Off,
        ..VideoSettings::default()
    };
    assert_eq!(
        run(&ff, &plain, dir.path(), off).await,
        Prepared::Unchanged(Unchanged::Plan {
            reason: SkipReason::VideoOff
        })
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn h264_most_compatible() {
    let Some((ff, enc)) = tools().await else {
        return;
    };
    let Some(h264) = enc.h264 else {
        eprintln!("no H.264 encoder; skipping");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("clip.mov");
    make_video(&ff.bin, &input, 15, &[]);
    let settings = VideoSettings {
        codec: VideoCodec::H264,
        size: VideoSize::Balanced1080,
    };
    let c = compressed(run(&ff, &input, dir.path(), settings).await);
    assert_eq!(c.preset, format!("h264-{}", h264.name()));
    assert_eq!(c.extension.as_deref(), Some("mp4"));
    let out = probe_video(ff.ffprobe.as_ref().unwrap(), &c.output)
        .await
        .unwrap();
    assert_eq!(out.video().unwrap().codec, "h264");
}

#[tokio::test(flavor = "multi_thread")]
async fn broken_hardware_encoder_falls_back_to_software() {
    let Some((ff, enc)) = tools().await else {
        return;
    };
    if !enc.openh264 {
        eprintln!("no OpenH264; skipping");
        return;
    }
    // Pretend a hardware encoder works when it does not (an encoder that is
    // missing from the build fails the same way a broken driver does).
    let missing = H264Encoder::PRIORITY
        .into_iter()
        .find(|e| e.is_hardware() && Some(*e) != enc.h264)
        .unwrap();
    let ff = ff.clone().with_video(
        ff.ffprobe.clone().unwrap(),
        VideoEncoders {
            h264: Some(missing),
            openh264: true,
            av1: false,
        },
    );
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("clip.mp4");
    make_video(&ff.bin, &input, 15, &[]);
    let settings = VideoSettings {
        codec: VideoCodec::H264,
        size: VideoSize::Balanced1080,
    };
    let c = compressed(run(&ff, &input, dir.path(), settings).await);
    assert_eq!(c.preset, "h264-libopenh264", "retried in software");
}

/// The user's real video (manual): `R2DOCK_VIDEO_SAMPLE=<file> cargo test
/// -p r2dock-media --release --test video real_video -- --ignored --nocapture`
#[tokio::test(flavor = "multi_thread")]
#[ignore = "manual; needs R2DOCK_VIDEO_SAMPLE"]
async fn real_video() {
    let Some((ff, _)) = tools().await else { return };
    let input = PathBuf::from(std::env::var("R2DOCK_VIDEO_SAMPLE").unwrap());
    let work = std::env::var_os("R2DOCK_MEDIA_OUT")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let started = std::time::Instant::now();
    let p = run(&ff, &input, &work, VideoSettings::default()).await;
    let secs = started.elapsed().as_secs_f64();
    match p {
        Prepared::Compressed(c) => eprintln!(
            "{} MB -> {} MB ({:.1}% smaller) with {} in {:.0} s",
            c.original_size / 1_000_000,
            c.size / 1_000_000,
            100.0 * (1.0 - c.size as f64 / c.original_size as f64),
            c.preset,
            secs
        ),
        Prepared::Unchanged(u) => eprintln!("unchanged: {u:?} after {secs:.0} s"),
    }
}
