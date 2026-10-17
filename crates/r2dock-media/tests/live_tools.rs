//! Installs the real pinned FFmpeg package for this platform from the
//! project's GitHub release, then compresses a short file with it.
//! Needs the network, so it is ignored by default; CI runs it on every OS:
//! `cargo test -p r2dock-media --test live_tools -- --ignored`

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use r2dock_media::classify::ContentOverride;
use r2dock_media::ffmpeg::Ffmpeg;
use r2dock_media::pipeline::{PrepareJob, Prepared, prepare};
use r2dock_media::plan::AudioSettings;
use r2dock_media::tools::detect::{License, inspect};
use r2dock_media::tools::install::{HttpFetch, check_installed, install};
use r2dock_media::tools::manifest;
use tokio_util::sync::CancellationToken;

#[tokio::test(flavor = "multi_thread")]
#[ignore = "downloads about 18 to 97 MB from GitHub"]
async fn installs_the_pinned_package_and_compresses() {
    let pkg = manifest::for_this_platform().expect("a package for this platform");
    let root = tempfile::tempdir().unwrap();
    let fetch = HttpFetch::new("R2Dock-tests").unwrap();
    let started = std::time::Instant::now();
    let tools = install(
        root.path(),
        pkg,
        &fetch,
        &CancellationToken::new(),
        |_, _| {},
    )
    .await
    .unwrap();
    eprintln!(
        "installed {} in {:.1} s ({} MB)",
        pkg.platform,
        started.elapsed().as_secs_f64(),
        pkg.total_download() / 1_000_000
    );
    assert_eq!(check_installed(root.path(), Some(pkg)).unwrap(), tools);

    let info = inspect(&tools.ffmpeg).await.unwrap();
    assert_eq!(info.version, manifest::VERSION);
    assert_eq!(info.license, License::LgplV3, "never a GPL build");
    assert!(
        info.audio.mp3 && info.audio.opus && info.audio.aac && info.audio.flac,
        "{info:?}"
    );

    // Video: AV1 everywhere, and at least one H.264 encoder (hardware or
    // OpenH264) must pass a test encode.
    let video = r2dock_media::video::encoders::detect(&tools.ffmpeg, &info.encoders).await;
    eprintln!("video encoders: {video:?}");
    assert!(video.av1, "SVT-AV1 works");
    assert!(video.h264.is_some(), "an H.264 encoder works");

    // Make a test file with the installed FFmpeg itself, then compress it.
    let input = root.path().join("talk.mp3");
    let status = std::process::Command::new(&tools.ffmpeg)
        .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i"])
        .arg("sine=f=220:r=22050:d=120,volume=0.4[a];anoisesrc=r=22050:d=120:a=0.05[n];[a][n]amix=inputs=2")
        .args(["-c:a", "libmp3lame", "-b:a", "96k", "-ac", "1"])
        .arg(&input)
        .status()
        .unwrap();
    assert!(status.success());
    let work = root.path().join("work");
    let result = prepare(
        &Ffmpeg::new(tools.ffmpeg.clone()),
        PrepareJob {
            input: &input,
            work_dir: &work,
            stem: "live",
            settings: &AudioSettings::default(),
            video: &r2dock_core::compression::VideoSettings::default(),
            content: ContentOverride::Auto,
        },
        &CancellationToken::new(),
        |_| {},
    )
    .await
    .unwrap();
    let Prepared::Compressed(c) = result else {
        panic!("{result:?}");
    };
    assert!(c.size * 100 / c.original_size < 45, "{c:?}");
}
