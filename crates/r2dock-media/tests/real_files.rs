//! Manual checks against real media files (read only):
//! `R2DOCK_MEDIA_SAMPLES="a.mp3:b.mp3" cargo test -p r2dock-media --release --test real_files -- --ignored --nocapture`

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;
use std::time::Instant;

use r2dock_media::plan::{AudioDecision, AudioInputs, AudioSettings, SpeechQuality, plan_audio};
use r2dock_media::probe::probe_audio;

fn samples() -> Vec<String> {
    std::env::var("R2DOCK_MEDIA_SAMPLES")
        .unwrap_or_default()
        .split(':')
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

#[test]
#[ignore = "manual; needs R2DOCK_MEDIA_SAMPLES"]
fn probe_real_files() {
    for path in samples() {
        let started = Instant::now();
        let p = probe_audio(Path::new(&path)).unwrap();
        eprintln!(
            "{path}: {:?} in {:.3} s, real bitrate {:?}",
            p,
            started.elapsed().as_secs_f64(),
            p.real_bitrate()
        );
        for speech in [SpeechQuality::Kbps32, SpeechQuality::Kbps48] {
            let s = AudioSettings {
                speech,
                ..AudioSettings::default()
            };
            match plan_audio(&p, &s, AudioInputs::default()) {
                AudioDecision::Compress(t) => eprintln!(
                    "  {speech:?}: {} -> {:.0} MB ({:.0}% smaller), {:?}",
                    t.encoding.preset(),
                    t.estimated_size as f64 / 1e6,
                    100.0 * (1.0 - t.estimated_size as f64 / p.size as f64),
                    t.sample_rate
                ),
                AudioDecision::Skip(r) => eprintln!("  {speech:?}: skip {r:?}"),
            }
        }
    }
}

/// Real files through the whole pipeline: a compressed copy holds the same
/// video, audio, and subtitle tracks as the original and never fewer audio
/// channels unless downmix is allowed. Skips when `R2DOCK_MEDIA_SAMPLES` or
/// FFmpeg is missing. Read only: copies go to a temporary folder.
/// `R2DOCK_MEDIA_SAMPLES="a.m4b:b.mkv" R2DOCK_TEST_FFMPEG=... cargo test -p
/// r2dock-media --release --test real_files real_files_keep_their_tracks`
#[tokio::test(flavor = "multi_thread")]
async fn real_files_keep_their_tracks() {
    use r2dock_media::classify::ContentOverride;
    use r2dock_media::ffmpeg::Ffmpeg;
    use r2dock_media::pipeline::{PrepareJob, Prepared, prepare};
    use r2dock_media::video::plan::VideoEncoders;
    use r2dock_media::video::probe::{StreamKind, VideoProbe, probe_video};
    use tokio_util::sync::CancellationToken;

    let paths = samples();
    let Some(bin) = std::env::var_os("R2DOCK_TEST_FFMPEG").map(std::path::PathBuf::from) else {
        eprintln!("R2DOCK_TEST_FFMPEG not set; skipping");
        return;
    };
    if paths.is_empty() {
        eprintln!("R2DOCK_MEDIA_SAMPLES not set; skipping");
        return;
    }
    let ffprobe = bin.with_file_name(if cfg!(windows) {
        "ffprobe.exe"
    } else {
        "ffprobe"
    });
    let out = std::process::Command::new(&bin)
        .args(["-hide_banner", "-encoders"])
        .output()
        .unwrap();
    let listed = r2dock_media::tools::detect::parse_encoders(&String::from_utf8_lossy(&out.stdout));
    let encoders: VideoEncoders = r2dock_media::video::encoders::detect(&bin, &listed).await;
    let ff = Ffmpeg::new(bin).with_video(ffprobe.clone(), encoders);
    let count = |p: &VideoProbe, kind: StreamKind| {
        p.streams
            .iter()
            .filter(|s| s.kind == kind && !s.attached_pic)
            .count()
    };
    for (n, path) in paths.iter().enumerate() {
        let input = Path::new(path);
        let work = tempfile::tempdir().unwrap();
        let source = probe_video(&ffprobe, input).await.unwrap();
        let prepared = prepare(
            &ff,
            PrepareJob {
                input,
                work_dir: work.path(),
                stem: "job",
                settings: &AudioSettings::default(),
                video: &r2dock_core::compression::VideoSettings::default(),
                content: ContentOverride::Auto,
            },
            &CancellationToken::new(),
            |_| {},
        )
        .await
        .unwrap();
        // File names stay out of the log; the sample number is enough.
        let Prepared::Compressed(c) = prepared else {
            eprintln!("sample {n}: unchanged: {prepared:?}");
            continue;
        };
        let output = probe_video(&ffprobe, &c.output).await.unwrap();
        for kind in [StreamKind::Video, StreamKind::Audio, StreamKind::Subtitle] {
            assert_eq!(
                count(&output, kind.clone()),
                count(&source, kind.clone()),
                "sample {n}: {kind:?}"
            );
        }
        for (a, b) in output.audio().zip(source.audio()) {
            assert_eq!(a.channels, b.channels, "sample {n}: channels");
        }
        eprintln!(
            "sample {n}: {} -> {} bytes, tracks kept",
            c.original_size, c.size
        );
    }
}

/// Full pipeline on real files: plan, sample test, full encode, verify.
/// `R2DOCK_MEDIA_OUT` is the folder for the compressed copies.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "manual; needs R2DOCK_MEDIA_SAMPLES, R2DOCK_MEDIA_OUT, R2DOCK_TEST_FFMPEG"]
async fn compress_real_files() {
    use r2dock_media::ffmpeg::Ffmpeg;
    use tokio_util::sync::CancellationToken;

    let out_dir = std::path::PathBuf::from(std::env::var("R2DOCK_MEDIA_OUT").unwrap());
    let ff = Ffmpeg::new(std::env::var("R2DOCK_TEST_FFMPEG").unwrap().into());
    let jobs = samples().into_iter().map(|path| {
        let ff = ff.clone();
        let out_dir = out_dir.clone();
        tokio::spawn(async move {
            let input = std::path::PathBuf::from(&path);
            let p = probe_audio(&input).unwrap();
            let s = AudioSettings::default();
            let AudioDecision::Compress(t) = plan_audio(&p, &s, AudioInputs::default()) else {
                panic!("{path}: expected compression");
            };
            let name = input.file_name().unwrap().to_string_lossy().into_owned();
            let duration = p.duration_secs.unwrap();
            let token = CancellationToken::new();
            let started = Instant::now();
            let sample = out_dir.join(format!("sample-{name}"));
            let predicted = ff
                .sample_saving(&input, &sample, &t, p.size, duration, &token)
                .await
                .unwrap();
            let sample_secs = started.elapsed().as_secs_f64();
            let out = out_dir.join(&name);
            let started = Instant::now();
            let r = ff
                .encode(&input, &out, &t, duration, &token, |_| {})
                .await
                .unwrap();
            let saved = 100.0 * (1.0 - r.size as f64 / p.size as f64);
            format!(
                "{name}: {:.0} MB -> {:.0} MB, saved {saved:.1}% (sample predicted {predicted:.1}% in {sample_secs:.1} s), encode {:.0} s, duration {:.0} s vs {:.0} s, marker {:?}",
                p.size as f64 / 1e6,
                r.size as f64 / 1e6,
                started.elapsed().as_secs_f64(),
                r.duration_secs.unwrap(),
                duration,
                r.marker
            )
        })
    });
    for j in jobs {
        eprintln!("{}", j.await.unwrap());
    }
}
