//! Content preservation with a real FFmpeg and ffprobe: compression never
//! drops a stream or a channel. Every fixture is generated at test time in a
//! temporary folder. Each test skips when FFmpeg or ffprobe is missing.
//!
//! FFmpeg is found through `R2DOCK_TEST_FFMPEG` or on `PATH` (tests only).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::process::Command;

use r2dock_core::compression::VideoSettings;
use r2dock_media::classify::ContentOverride;
use r2dock_media::ffmpeg::Ffmpeg;
use r2dock_media::pipeline::{PrepareJob, Prepared, Unchanged, prepare};
use r2dock_media::plan::{AudioSettings, SkipReason};
use r2dock_media::tools::detect::parse_encoders;
use r2dock_media::video::encoders::detect;
use r2dock_media::video::plan::VideoEncoders;
use r2dock_media::video::probe::{StreamKind, VideoProbe, probe_video};
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
    Some((Ffmpeg::new(bin).with_video(probe, found), found))
}

fn make(bin: &Path, args: &[&str]) {
    let out = Command::new(bin)
        .args(["-hide_banner", "-nostdin", "-loglevel", "error", "-y"])
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

async fn run_with(ff: &Ffmpeg, input: &Path, work: &Path, settings: &AudioSettings) -> Prepared {
    prepare(
        ff,
        PrepareJob {
            input,
            work_dir: work,
            stem: "job",
            settings,
            video: &VideoSettings::default(),
            content: ContentOverride::Auto,
        },
        &CancellationToken::new(),
        |_| {},
    )
    .await
    .unwrap()
}

async fn run(ff: &Ffmpeg, input: &Path, work: &Path) -> Prepared {
    run_with(ff, input, work, &AudioSettings::default()).await
}

async fn inventory(ff: &Ffmpeg, file: &Path) -> VideoProbe {
    probe_video(ff.ffprobe.as_ref().unwrap(), file)
        .await
        .unwrap()
}

fn count(p: &VideoProbe, kind: StreamKind) -> usize {
    p.streams
        .iter()
        .filter(|s| s.kind == kind && !s.attached_pic)
        .count()
}

/// The two mpeg4 video streams of the dual-video fixture.
fn make_dual_video(bin: &Path, out: &Path) {
    make(
        bin,
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=s=1280x720:r=25:d=12",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=s=640x360:r=25:d=12",
            "-map",
            "0:v",
            "-map",
            "1:v",
            "-c:v",
            "mpeg4",
            "-q:v",
            "2",
            s(out),
        ],
    );
}

fn unsupported_streams() -> Prepared {
    Prepared::Unchanged(Unchanged::Plan {
        reason: SkipReason::UnsupportedStreams,
    })
}

#[tokio::test(flavor = "multi_thread")]
async fn second_audio_stream_keeps_the_original() {
    let Some((ff, _)) = tools().await else { return };
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("two-languages.m4a");
    make(
        &ff.bin,
        &[
            "-f",
            "lavfi",
            "-i",
            "anoisesrc=r=44100:d=60:a=0.1",
            "-f",
            "lavfi",
            "-i",
            "anoisesrc=r=44100:d=60:a=0.1",
            "-map",
            "0:a",
            "-map",
            "1:a",
            "-c:a",
            "aac",
            "-b:a",
            "192k",
            "-ac",
            "1",
            s(&input),
        ],
    );
    assert_eq!(count(&inventory(&ff, &input).await, StreamKind::Audio), 2);
    assert_eq!(run(&ff, &input, dir.path()).await, unsupported_streams());

    // The same check without ffprobe (the in-process probe sees both tracks).
    let audio_only = Ffmpeg::new(ff.bin.clone());
    assert_eq!(
        run(&audio_only, &input, dir.path()).await,
        unsupported_streams()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn second_video_stream_keeps_the_original() {
    let Some((ff, enc)) = tools().await else {
        return;
    };
    if !enc.av1 && enc.h264.is_none() {
        eprintln!("no video encoder; skipping");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("two-cameras.mkv");
    make_dual_video(&ff.bin, &input);
    assert_eq!(count(&inventory(&ff, &input).await, StreamKind::Video), 2);
    assert_eq!(run(&ff, &input, dir.path()).await, unsupported_streams());
}

#[tokio::test(flavor = "multi_thread")]
async fn unconvertible_subtitles_keep_the_original() {
    let Some((ff, enc)) = tools().await else {
        return;
    };
    if !enc.av1 && enc.h264.is_none() {
        eprintln!("no video encoder; skipping");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let video = dir.path().join("video.mkv");
    make_dual_video(&ff.bin, &video);
    let srt = dir.path().join("captions.srt");
    std::fs::write(&srt, "1\n00:00:01,000 --> 00:00:04,000\nHello\n").unwrap();
    let input = dir.path().join("captioned.mp4");
    make(
        &ff.bin,
        &[
            "-i",
            s(&video),
            "-i",
            s(&srt),
            "-map",
            "0:v:0",
            "-map",
            "1:s:0",
            "-c:v",
            "copy",
            "-c:s",
            "ttml",
            s(&input),
        ],
    );
    let src = inventory(&ff, &input).await;
    assert_eq!(count(&src, StreamKind::Video), 1);
    assert_eq!(count(&src, StreamKind::Subtitle), 1);
    assert_eq!(run(&ff, &input, dir.path()).await, unsupported_streams());
}

/// Mean level in dB of `secs` seconds starting at `start`, one channel.
fn mean_volume(bin: &Path, file: &Path, start: f64, secs: f64, channel: u8) -> f64 {
    let filter = format!("atrim=start={start}:duration={secs},pan=mono|c0=c{channel},volumedetect");
    let out = Command::new(bin)
        .args(["-hide_banner", "-nostdin", "-i", s(file), "-af", &filter])
        .args(["-f", "null", "-"])
        .output()
        .unwrap();
    let log = String::from_utf8_lossy(&out.stderr);
    log.lines()
        .find_map(|l| l.split("mean_volume:").nth(1))
        .map(|v| v.trim().trim_end_matches("dB").trim().parse().unwrap())
        .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn stereo_that_only_looks_mono_in_the_middle_stays_stereo() {
    let Some((ff, _)) = tools().await else { return };
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("edges.wav");
    // Left and right match in the middle 40 seconds and are opposite at the
    // start and end, so a mono mix would erase the first and last 10 s.
    make(
        &ff.bin,
        &[
            "-f",
            "lavfi",
            "-i",
            "aevalsrc=0.3*sin(2*PI*440*t)|if(lt(t\\,10)+gte(t\\,50)\\,-0.3*sin(2*PI*440*t)\\,0.3*sin(2*PI*440*t)):s=44100:d=60",
            "-c:a",
            "pcm_s16le",
            s(&input),
        ],
    );
    match run(&ff, &input, dir.path()).await {
        Prepared::Compressed(c) => {
            let out = inventory(&ff, &c.output).await;
            let audio: Vec<_> = out.audio().collect();
            assert_eq!(audio.len(), 1);
            assert_eq!(audio[0].channels, 2, "channels kept");
            for channel in [0, 1] {
                let intro = mean_volume(&ff.bin, &c.output, 0.0, 5.0, channel);
                assert!(intro > -30.0, "intro audible on c{channel}: {intro} dB");
            }
            // The mono mix of the output keeps the intro too.
            let mix = dir.path().join("mix.wav");
            make(&ff.bin, &["-i", s(&c.output), "-ac", "1", s(&mix)]);
            let source_mix = dir.path().join("source-mix.wav");
            make(&ff.bin, &["-i", s(&input), "-ac", "1", s(&source_mix)]);
            let got = mean_volume(&ff.bin, &mix, 20.0, 5.0, 0);
            let want = mean_volume(&ff.bin, &source_mix, 20.0, 5.0, 0);
            assert!((got - want).abs() < 3.0, "middle level {got} vs {want}");
        }
        Prepared::Unchanged(u) => panic!("a WAV should still compress: {u:?}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn identical_stereo_music_is_not_folded_to_mono() {
    let Some((ff, _)) = tools().await else { return };
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("song.wav");
    make(
        &ff.bin,
        &[
            "-f",
            "lavfi",
            "-i",
            "sine=f=330:r=44100:d=40",
            "-ac",
            "2",
            "-c:a",
            "pcm_s16le",
            s(&input),
        ],
    );
    let Prepared::Compressed(c) = run(&ff, &input, dir.path()).await else {
        panic!("a WAV should compress");
    };
    let out = inventory(&ff, &c.output).await;
    assert_eq!(out.audio().next().unwrap().channels, 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn multichannel_speech_is_not_downmixed_without_permission() {
    let Some((ff, _)) = tools().await else { return };
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("surround.wav");
    make(
        &ff.bin,
        &[
            "-f",
            "lavfi",
            "-i",
            "sine=f=220:r=22050:d=30",
            "-af",
            "aformat=channel_layouts=5.1",
            "-c:a",
            "pcm_s16le",
            s(&input),
        ],
    );
    assert_eq!(
        run(&ff, &input, dir.path()).await,
        Prepared::Unchanged(Unchanged::Plan {
            reason: SkipReason::Multichannel { channels: 6 }
        })
    );
    let allowed = AudioSettings {
        allow_downmix: true,
        ..AudioSettings::default()
    };
    let Prepared::Compressed(c) = run_with(&ff, &input, dir.path(), &allowed).await else {
        panic!("downmix allowed: expected compression");
    };
    let out = inventory(&ff, &c.output).await;
    assert!(out.audio().next().unwrap().channels <= 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn supported_streams_are_all_kept() {
    let Some((ff, enc)) = tools().await else {
        return;
    };
    if !enc.av1 && enc.h264.is_none() {
        eprintln!("no video encoder; skipping");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let srt = dir.path().join("subs.srt");
    std::fs::write(&srt, "1\n00:00:01,000 --> 00:00:03,000\nHello\n").unwrap();
    let input = dir.path().join("clip.mkv");
    make(
        &ff.bin,
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=s=1280x720:r=25:d=12",
            "-f",
            "lavfi",
            "-i",
            "sine=f=440:d=12",
            "-f",
            "lavfi",
            "-i",
            "sine=f=660:d=12",
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
            "-metadata:s:a:0",
            "language=eng",
            "-metadata:s:a:1",
            "language=spa",
            s(&input),
        ],
    );
    let src = inventory(&ff, &input).await;
    let Prepared::Compressed(c) = run(&ff, &input, dir.path()).await else {
        panic!("expected compression");
    };
    let out = inventory(&ff, &c.output).await;
    for kind in [StreamKind::Video, StreamKind::Audio, StreamKind::Subtitle] {
        assert_eq!(
            count(&out, kind.clone()),
            count(&src, kind.clone()),
            "{kind:?}"
        );
    }
    let languages: Vec<_> = out.audio().map(|a| a.language.as_str()).collect();
    assert_eq!(languages, ["eng", "spa"]);
}

/// An ffprobe that reports the compressed copy (`job.*` in the work folder)
/// with something missing: `edit` is a sed expression applied to its JSON.
#[cfg(unix)]
fn lying_ffprobe(dir: &Path, real: &Path, edit: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let script = dir.join(format!("ffprobe-lies-{}", edit.len()));
    let body = format!(
        "#!/bin/sh\nfor last; do :; done\ncase \"$last\" in\n  */job.*) \"{}\" \"$@\" | sed '{edit}' ;;\n  *) exec \"{}\" \"$@\" ;;\nesac\n",
        real.display(),
        real.display()
    );
    std::fs::write(&script, body).unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    script
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn a_copy_that_lost_something_uploads_the_original() {
    let Some((ff, enc)) = tools().await else {
        return;
    };
    let real = ff.ffprobe.clone().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let not_kept = Prepared::Unchanged(Unchanged::Plan {
        reason: SkipReason::TracksNotKept,
    });

    // Audio: the copy reports one channel instead of two.
    let wav = dir.path().join("song.wav");
    make(
        &ff.bin,
        &[
            "-f",
            "lavfi",
            "-i",
            "sine=f=330:r=44100:d=40",
            "-ac",
            "2",
            s(&wav),
        ],
    );
    let lying = lying_ffprobe(dir.path(), &real, r#"s/"channels": 2/"channels": 1/"#);
    let ff_lying = ff.clone().with_video(lying, enc);
    let work = dir.path().join("audio-work");
    assert_eq!(run(&ff_lying, &wav, &work).await, not_kept);
    assert!(!work.join("job.mp3").exists(), "the copy is deleted");

    // Video: the copy reports its subtitle as a data track.
    if !enc.av1 && enc.h264.is_none() {
        eprintln!("no video encoder; skipping the video half");
        return;
    }
    let srt = dir.path().join("subs.srt");
    std::fs::write(&srt, "1\n00:00:01,000 --> 00:00:03,000\nHello\n").unwrap();
    let mkv = dir.path().join("clip.mkv");
    make(
        &ff.bin,
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=s=1280x720:r=25:d=12",
            "-f",
            "lavfi",
            "-i",
            "sine=f=440:d=12",
            "-i",
            s(&srt),
            "-map",
            "0",
            "-map",
            "1",
            "-map",
            "2",
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
            s(&mkv),
        ],
    );
    let lying = lying_ffprobe(
        dir.path(),
        &real,
        r#"s/"codec_type": "subtitle"/"codec_type": "data"/"#,
    );
    let ff_lying = ff.clone().with_video(lying, enc);
    let work = dir.path().join("video-work");
    assert_eq!(run(&ff_lying, &mkv, &work).await, not_kept);
    assert!(!work.join("job.mp4").exists(), "the copy is deleted");
}

#[tokio::test(flavor = "multi_thread")]
async fn audiobook_with_chapters_and_cover_still_compresses() {
    let Some((ff, _)) = tools().await else { return };
    let dir = tempfile::tempdir().unwrap();
    let meta = dir.path().join("meta.txt");
    std::fs::write(
        &meta,
        ";FFMETADATA1\ntitle=Book\n[CHAPTER]\nTIMEBASE=1/1000\nSTART=0\nEND=60000\ntitle=One\n[CHAPTER]\nTIMEBASE=1/1000\nSTART=60000\nEND=120000\ntitle=Two\n",
    )
    .unwrap();
    let art = dir.path().join("art.png");
    make(
        &ff.bin,
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
    let input = dir.path().join("book.m4b");
    make(
        &ff.bin,
        &[
            "-f",
            "lavfi",
            "-i",
            "anoisesrc=r=22050:d=120:a=0.1",
            "-i",
            s(&meta),
            "-i",
            s(&art),
            "-map",
            "0:a",
            "-map",
            "2:v",
            "-map_metadata",
            "1",
            "-map_chapters",
            "1",
            "-c:a",
            "aac",
            "-b:a",
            "128k",
            "-ac",
            "1",
            "-c:v",
            "png",
            "-disposition:v",
            "attached_pic",
            s(&input),
        ],
    );
    let src = inventory(&ff, &input).await;
    assert!(
        src.streams.iter().any(|s| s.kind == StreamKind::Data),
        "the fixture has a chapter track: {src:?}"
    );
    let Prepared::Compressed(c) = run(&ff, &input, dir.path()).await else {
        panic!("an audiobook with chapters and cover art should compress");
    };
    let out = inventory(&ff, &c.output).await;
    assert_eq!(count(&out, StreamKind::Audio), 1);
    let chapters = Command::new(ff.ffprobe.as_ref().unwrap())
        .args([
            "-v",
            "error",
            "-show_chapters",
            "-of",
            "csv=p=0",
            s(&c.output),
        ])
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&chapters.stdout).lines().count(), 2);
}

/// Speech-like audio at 22.05 kHz (counts as speech), as a lavfi graph.
fn speech_graph(secs: u32) -> String {
    format!(
        "sine=f=220:r=22050:d={secs},volume=0.4[a];anoisesrc=r=22050:d={secs}:a=0.05[n];[a][n]amix=inputs=2"
    )
}

async fn output_channels(ff: &Ffmpeg, p: Prepared) -> u32 {
    let Prepared::Compressed(c) = p else {
        panic!("expected compression, got {p:?}");
    };
    inventory(ff, &c.output)
        .await
        .audio()
        .next()
        .unwrap()
        .channels
}

#[tokio::test(flavor = "multi_thread")]
async fn identical_stereo_speech_becomes_mono() {
    let Some((ff, _)) = tools().await else { return };
    let dir = tempfile::tempdir().unwrap();
    let graph = speech_graph(120);
    // The same signal in both channels, as WAV and as joint-stereo MP3.
    let wav = dir.path().join("talk.wav");
    make(
        &ff.bin,
        &[
            "-f",
            "lavfi",
            "-i",
            &graph,
            "-ac",
            "2",
            "-c:a",
            "pcm_s16le",
            s(&wav),
        ],
    );
    let mp3 = dir.path().join("talk.mp3");
    make(
        &ff.bin,
        &[
            "-f",
            "lavfi",
            "-i",
            &graph,
            "-ac",
            "2",
            "-c:a",
            "libmp3lame",
            "-b:a",
            "128k",
            s(&mp3),
        ],
    );
    let token = CancellationToken::new();
    for input in [&wav, &mp3] {
        assert!(
            ff.channels_identical(input, &token).await.unwrap(),
            "{input:?}"
        );
        let work = dir.path().join(format!(
            "work-{}",
            input.extension().unwrap().to_str().unwrap()
        ));
        assert_eq!(
            output_channels(&ff, run(&ff, input, &work).await).await,
            1,
            "{input:?}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn speech_that_only_matches_in_the_middle_stays_stereo() {
    let Some((ff, _)) = tools().await else { return };
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("edges.wav");
    // 22.05 kHz, so it is speech and gets the channel check. Left and right
    // match except in the first and last 10 seconds, where they are opposite.
    make(
        &ff.bin,
        &[
            "-f",
            "lavfi",
            "-i",
            "aevalsrc=0.3*sin(2*PI*440*t)|if(lt(t\\,10)+gte(t\\,50)\\,-0.3*sin(2*PI*440*t)\\,0.3*sin(2*PI*440*t)):s=22050:d=60",
            "-c:a",
            "pcm_s16le",
            s(&input),
        ],
    );
    let token = CancellationToken::new();
    assert!(!ff.channels_identical(&input, &token).await.unwrap());
    let Prepared::Compressed(c) = run(&ff, &input, dir.path()).await else {
        panic!("expected compression");
    };
    assert_eq!(
        inventory(&ff, &c.output)
            .await
            .audio()
            .next()
            .unwrap()
            .channels,
        2
    );
    for channel in [0, 1] {
        let intro = mean_volume(&ff.bin, &c.output, 0.0, 5.0, channel);
        assert!(intro > -30.0, "intro audible on c{channel}: {intro} dB");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn lossless_audio_with_a_language_tag_still_compresses() {
    let Some((ff, _)) = tools().await else { return };
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("tagged.m4a");
    // ALAC with language=eng becomes MP3, which has no per-track language.
    make(
        &ff.bin,
        &[
            "-f",
            "lavfi",
            "-i",
            "anoisesrc=r=44100:d=120:a=0.3",
            "-ac",
            "2",
            "-c:a",
            "alac",
            "-metadata:s:a:0",
            "language=eng",
            s(&input),
        ],
    );
    let Prepared::Compressed(c) = run(&ff, &input, dir.path()).await else {
        panic!("expected compression");
    };
    assert_eq!(inventory(&ff, &c.output).await.audio().count(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn quiet_opposite_channels_are_not_identical() {
    let Some((ff, _)) = tools().await else { return };
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("quiet.mp3");
    // Very quiet audio with right = -left: the difference is below the
    // absolute limit, but a mono mix would cancel it to silence.
    make(
        &ff.bin,
        &[
            "-f",
            "lavfi",
            "-i",
            "aevalsrc=0.0002*sin(2*PI*300*t)|-0.0002*sin(2*PI*300*t):s=22050:d=20",
            "-c:a",
            "libmp3lame",
            "-b:a",
            "64k",
            s(&input),
        ],
    );
    let token = CancellationToken::new();
    assert!(!ff.channels_identical(&input, &token).await.unwrap());
}

#[tokio::test(flavor = "multi_thread")]
async fn channel_check_can_be_cancelled() {
    let Some((ff, _)) = tools().await else { return };
    let dir = tempfile::tempdir().unwrap();
    // 40 minutes of stereo speech: long enough that the check is still
    // running when it is cancelled.
    let input = dir.path().join("long.mp3");
    make(
        &ff.bin,
        &[
            "-f",
            "lavfi",
            "-i",
            &speech_graph(2400),
            "-ac",
            "2",
            "-c:a",
            "libmp3lame",
            "-b:a",
            "96k",
            s(&input),
        ],
    );
    let token = CancellationToken::new();
    let cancel = token.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        cancel.cancel();
    });
    let started = std::time::Instant::now();
    let err = ff.channels_identical(&input, &token).await.unwrap_err();
    assert!(err.is_cancelled(), "{err:?}");
    assert!(started.elapsed() < std::time::Duration::from_secs(5));

    // Cancelling the whole preparation during the check stops it too.
    let token = CancellationToken::new();
    let cancel = token.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        cancel.cancel();
    });
    let result = prepare(
        &ff,
        PrepareJob {
            input: &input,
            work_dir: &dir.path().join("work"),
            stem: "job",
            settings: &AudioSettings::default(),
            video: &VideoSettings::default(),
            content: ContentOverride::Auto,
        },
        &token,
        |_| {},
    )
    .await;
    assert!(result.is_err_and(|e| e.is_cancelled()));
}

/// The cost of the whole-stream check on an hour of stereo speech (manual):
/// `R2DOCK_TEST_FFMPEG=... cargo test -p r2dock-media --release --test
/// preserve channel_check_cost -- --ignored --nocapture`
#[tokio::test(flavor = "multi_thread")]
#[ignore = "manual measurement; encodes an hour of audio first"]
async fn channel_check_cost() {
    let Some((ff, _)) = tools().await else { return };
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("hour.mp3");
    make(
        &ff.bin,
        &[
            "-f",
            "lavfi",
            "-i",
            &speech_graph(3600),
            "-ac",
            "2",
            "-c:a",
            "libmp3lame",
            "-b:a",
            "64k",
            s(&input),
        ],
    );
    let started = std::time::Instant::now();
    let same = ff
        .channels_identical(&input, &CancellationToken::new())
        .await
        .unwrap();
    eprintln!(
        "60 min stereo MP3 (64 kbps, 22.05 kHz): identical={same} in {:.2} s",
        started.elapsed().as_secs_f64()
    );
    assert!(same);
}
