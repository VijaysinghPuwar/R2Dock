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
