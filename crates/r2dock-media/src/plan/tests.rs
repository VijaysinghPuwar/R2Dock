use super::*;

const HOUR: f64 = 3600.0;

/// A probe of `secs` seconds at `kbps` real bitrate.
fn probe(codec: AudioCodec, rate: u32, channels: u8, kbps: u64, secs: f64) -> AudioProbe {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let size = (kbps as f64 * 1000.0 * secs / 8.0) as u64;
    AudioProbe {
        codec,
        sample_rate: Some(rate),
        channels: Some(channels),
        duration_secs: Some(secs),
        size,
        picture_bytes: 0,
        has_video: false,
        audio_tracks: 1,
        marker: None,
    }
}

fn mp3_book(kbps: u64, rate: u32) -> AudioProbe {
    probe(AudioCodec::Mp3, rate, 1, kbps, 23.0 * HOUR)
}

fn song(codec: AudioCodec, kbps: u64, channels: u8) -> AudioProbe {
    probe(codec, 44_100, channels, kbps, 240.0)
}

fn settings() -> AudioSettings {
    AudioSettings::default()
}

fn with(f: impl FnOnce(&mut AudioSettings)) -> AudioSettings {
    let mut s = settings();
    f(&mut s);
    s
}

fn run(p: &AudioProbe, s: &AudioSettings) -> AudioDecision {
    plan_audio(p, s, AudioInputs::default())
}

fn target(d: AudioDecision) -> AudioTarget {
    match d {
        AudioDecision::Compress(t) => t,
        AudioDecision::Skip(r) => panic!("expected compression, got {r:?}"),
    }
}

fn skip(d: AudioDecision) -> SkipReason {
    match d {
        AudioDecision::Skip(r) => r,
        AudioDecision::Compress(t) => panic!("expected skip, got {t:?}"),
    }
}

fn saving_percent(p: &AudioProbe, t: &AudioTarget) -> f64 {
    100.0 * (1.0 - t.estimated_size as f64 / p.size as f64)
}

#[test]
fn audiobook_64k_speech_32k_halves() {
    let p = mp3_book(64, 22_050);
    let t = target(run(&p, &settings()));
    assert_eq!(t.encoding, Encoding::Mp3Cbr { kbps: 32 });
    assert_eq!((t.channels, t.sample_rate), (1, Some(22_050)));
    assert_eq!(t.extension, None, "MP3 stays MP3");
    assert_eq!(t.content, Content::Speech);
    let saved = saving_percent(&p, &t);
    assert!((49.0..50.0).contains(&saved), "{saved}");
}

#[test]
fn audiobook_48k_24khz() {
    let p = mp3_book(48, 24_000);
    // 32 kbps: about a third smaller, resampled to 22.05 kHz.
    let t = target(run(&p, &settings()));
    assert_eq!(t.sample_rate, Some(22_050));
    let saved = saving_percent(&p, &t);
    assert!((32.0..34.0).contains(&saved), "{saved}");
    // 48 kbps preset: already at the target, left alone.
    let s = with(|s| s.speech = SpeechQuality::Kbps48);
    assert_eq!(
        skip(run(&p, &s)),
        SkipReason::AlreadyCompressed { kbps: 48 }
    );
}

#[test]
fn margin_check() {
    let p = mp3_book(64, 22_050);
    let s48 = with(|s| s.speech = SpeechQuality::Kbps48);
    assert!(matches!(run(&p, &s48), AudioDecision::Compress(_)));
    let strict = with(|s| {
        s.speech = SpeechQuality::Kbps48;
        s.min_saving_percent = 30;
    });
    assert_eq!(
        skip(run(&p, &strict)),
        SkipReason::SavingTooSmall { percent: 24 }
    );
}

#[test]
fn cheap_checks_come_first() {
    let mut marked = mp3_book(64, 22_050);
    marked.marker = Some("mp3-32k".into());
    assert_eq!(
        skip(run(&marked, &settings())),
        SkipReason::AlreadyCompressedByR2dock {
            preset: "mp3-32k".into()
        }
    );
    let tiny = probe(AudioCodec::Mp3, 44_100, 2, 320, 20.0);
    assert!(tiny.size < MIN_SIZE);
    assert_eq!(skip(run(&tiny, &settings())), SkipReason::TooSmall);
    let short = probe(AudioCodec::Pcm, 48_000, 2, 1536, 8.0);
    assert!(short.size >= MIN_SIZE);
    assert_eq!(skip(run(&short, &settings())), SkipReason::TooShort);
    let mut unknown = mp3_book(64, 22_050);
    unknown.duration_secs = None;
    assert_eq!(
        skip(run(&unknown, &settings())),
        SkipReason::UnknownDuration
    );
    let mut video = song(AudioCodec::Aac, 128, 2);
    video.has_video = true;
    assert_eq!(skip(run(&video, &settings())), SkipReason::Video);
}

#[test]
fn music_mp3() {
    let t = target(run(&song(AudioCodec::Mp3, 320, 2), &settings()));
    assert_eq!(
        t.encoding,
        Encoding::Mp3Vbr {
            quality: 4,
            approx_kbps: 165
        }
    );
    assert_eq!((t.channels, t.sample_rate), (2, Some(44_100)));
    let smaller = with(|s| s.music = MusicQuality::Smaller);
    let t = target(run(&song(AudioCodec::Mp3, 320, 2), &smaller));
    assert!(matches!(t.encoding, Encoding::Mp3Vbr { quality: 6, .. }));
    assert_eq!(
        skip(run(&song(AudioCodec::Mp3, 128, 2), &settings())),
        SkipReason::AlreadyCompressed { kbps: 128 }
    );
}

#[test]
fn channels_are_kept_unless_downmix_is_allowed() {
    // Stereo speech (22.05 kHz) stays stereo, at twice the mono bitrate.
    let stereo_talk = probe(AudioCodec::Mp3, 22_050, 2, 128, HOUR);
    let t = target(run(&stereo_talk, &settings()));
    assert_eq!(t.content, Content::Speech);
    assert_eq!((t.encoding, t.channels), (Encoding::Mp3Cbr { kbps: 64 }, 2));
    let s48 = with(|s| s.speech = SpeechQuality::Kbps48);
    let t = target(run(&stereo_talk, &s48));
    assert_eq!((t.encoding, t.channels), (Encoding::Mp3Cbr { kbps: 96 }, 2));
    let smallest = with(|s| s.mode = Mode::Smallest);
    let t = target(run(&stereo_talk, &smallest));
    assert_eq!((t.encoding, t.channels), (Encoding::Opus { kbps: 48 }, 2));
    // With permission, speech becomes mono.
    let downmix = with(|s| s.allow_downmix = true);
    let t = target(run(&stereo_talk, &downmix));
    assert_eq!((t.encoding, t.channels), (Encoding::Mp3Cbr { kbps: 32 }, 1));
    // Stereo music is never folded to mono, even with permission.
    let t = target(run(&song(AudioCodec::Mp3, 320, 2), &downmix));
    assert_eq!(t.channels, 2);
    // A low-bitrate stereo file that looks like speech stays stereo, and is
    // left alone because stereo speech is already at its target.
    assert_eq!(
        skip(run(&song(AudioCodec::Mp3, 64, 2), &settings())),
        SkipReason::AlreadyCompressed { kbps: 64 }
    );
}

#[test]
fn multichannel_veto_comes_before_speech() {
    // 22.05 kHz six-channel audio classifies as speech; without permission
    // it is still kept, whatever the content.
    let surround = probe(AudioCodec::Pcm, 22_050, 6, 2117, 300.0);
    for content in [
        ContentOverride::Auto,
        ContentOverride::Speech,
        ContentOverride::Music,
    ] {
        assert_eq!(
            skip(plan_audio(
                &surround,
                &settings(),
                AudioInputs {
                    content,
                    ..AudioInputs::default()
                }
            )),
            SkipReason::Multichannel { channels: 6 },
            "{content:?}"
        );
        let smallest = with(|s| s.mode = Mode::Smallest);
        assert_eq!(
            skip(plan_audio(
                &surround,
                &smallest,
                AudioInputs {
                    content,
                    ..AudioInputs::default()
                }
            )),
            SkipReason::Multichannel { channels: 6 },
            "{content:?} smallest"
        );
    }
    let downmix = with(|s| s.allow_downmix = true);
    assert_eq!(target(run(&surround, &downmix)).channels, 1, "speech");
    let music = AudioInputs {
        content: ContentOverride::Music,
        ..AudioInputs::default()
    };
    assert_eq!(
        target(plan_audio(&surround, &downmix, music)).channels,
        2,
        "music"
    );
}

#[test]
fn proven_identical_channels_become_mono_without_permission() {
    let talk = probe(AudioCodec::Mp3, 22_050, 2, 128, HOUR);
    let proven = AudioInputs {
        identical_channels: true,
        ..AudioInputs::default()
    };
    let t = target(plan_audio(&talk, &settings(), proven));
    assert_eq!((t.encoding, t.channels), (Encoding::Mp3Cbr { kbps: 32 }, 1));
    // Never for more than two channels: identical left and right say
    // nothing about the others.
    let surround = probe(AudioCodec::Pcm, 22_050, 6, 2117, 300.0);
    assert_eq!(
        skip(plan_audio(&surround, &settings(), proven)),
        SkipReason::Multichannel { channels: 6 }
    );
}

#[test]
fn second_audio_track_keeps_the_original() {
    let mut p = mp3_book(64, 22_050);
    p.audio_tracks = 2;
    assert_eq!(skip(run(&p, &settings())), SkipReason::UnsupportedStreams);
}

#[test]
fn lossless_sources() {
    let wav = song(AudioCodec::Pcm, 1411, 2);
    let t = target(run(&wav, &settings()));
    assert!(matches!(t.encoding, Encoding::Mp3Vbr { .. }));
    assert_eq!(t.extension.as_deref(), Some("mp3"));
    let flac = with(|s| s.lossless = LosslessAction::Flac);
    let t = target(run(&wav, &flac));
    assert_eq!(t.encoding, Encoding::Flac);
    assert_eq!(t.extension.as_deref(), Some("flac"));
    assert_eq!(t.sample_rate, Some(44_100));
    // FLAC to FLAC saves nothing worth the work.
    let already = song(AudioCodec::Flac, 900, 2);
    assert_eq!(
        skip(run(&already, &flac)),
        SkipReason::SavingTooSmall { percent: 0 }
    );
    let keep = with(|s| s.lossless = LosslessAction::Keep);
    assert_eq!(skip(run(&wav, &keep)), SkipReason::LosslessKept);
    // Lossless speech still follows the speech ladder.
    let mono_wav = song(AudioCodec::Pcm, 705, 1);
    let t = target(run(&mono_wav, &settings()));
    assert_eq!(t.encoding, Encoding::Mp3Cbr { kbps: 32 });
    let smallest = with(|s| s.mode = Mode::Smallest);
    let t = target(run(&wav, &smallest));
    assert_eq!(t.encoding, Encoding::Opus { kbps: 96 });
    assert_eq!(t.extension.as_deref(), Some("opus"));
}

#[test]
fn multichannel() {
    let surround = probe(AudioCodec::Pcm, 48_000, 6, 4608, 300.0);
    assert_eq!(
        skip(run(&surround, &settings())),
        SkipReason::Multichannel { channels: 6 }
    );
    let flac = with(|s| s.lossless = LosslessAction::Flac);
    assert_eq!(target(run(&surround, &flac)).channels, 6);
    let downmix = with(|s| s.allow_downmix = true);
    assert_eq!(target(run(&surround, &downmix)).channels, 2);
}

#[test]
fn aac_keeps_its_format() {
    let t = target(run(&song(AudioCodec::Aac, 256, 2), &settings()));
    assert_eq!(t.encoding, Encoding::Aac { kbps: 128 });
    assert_eq!((t.extension, t.channels), (None, 2));
    let speech = probe(AudioCodec::Aac, 22_050, 1, 96, 3.0 * HOUR);
    assert_eq!(
        target(run(&speech, &settings())).encoding,
        Encoding::Aac { kbps: 64 }
    );
}

#[test]
fn smallest_mode() {
    let s = with(|s| s.mode = Mode::Smallest);
    let t = target(run(&mp3_book(64, 22_050), &s));
    assert_eq!(t.encoding, Encoding::Opus { kbps: 24 });
    assert_eq!((t.channels, t.sample_rate), (1, None));
    assert_eq!(t.extension.as_deref(), Some("opus"));
    let saved = saving_percent(&mp3_book(64, 22_050), &t);
    assert!((62.0..63.0).contains(&saved), "{saved}");
    let mono_music = probe(AudioCodec::Mp3, 44_100, 1, 192, 240.0);
    let t = target(plan_audio(
        &mono_music,
        &s,
        AudioInputs {
            content: ContentOverride::Music,
            ..AudioInputs::default()
        },
    ));
    assert_eq!(t.encoding, Encoding::Opus { kbps: 64 });
    // An Opus file at the target stays as it is.
    let opus = probe(AudioCodec::Opus, 48_000, 1, 24, 2.0 * HOUR);
    assert_eq!(
        skip(run(&opus, &settings())),
        SkipReason::AlreadyCompressed { kbps: 24 }
    );
}

#[test]
fn unsupported_formats() {
    let vorbis = song(AudioCodec::Vorbis, 256, 2);
    assert_eq!(
        skip(run(&vorbis, &settings())),
        SkipReason::UnsupportedFormat
    );
    let other = song(AudioCodec::Other, 256, 2);
    let smallest = with(|s| s.mode = Mode::Smallest);
    assert_eq!(skip(run(&other, &smallest)), SkipReason::UnsupportedFormat);
}

#[test]
fn never_raises_sample_rate() {
    for (source, want) in [
        (8_000, 8_000),
        (16_000, 16_000),
        (22_050, 22_050),
        (24_000, 22_050),
        (44_100, 22_050),
    ] {
        let p = probe(AudioCodec::Mp3, source, 1, 128, HOUR);
        assert_eq!(
            target(run(&p, &settings())).sample_rate,
            Some(want),
            "{source}"
        );
    }
    // Music at 32 kHz stays at 32 kHz.
    let p = probe(AudioCodec::Mp3, 32_000, 2, 256, 240.0);
    let t = target(plan_audio(
        &p,
        &settings(),
        AudioInputs {
            content: ContentOverride::Music,
            ..AudioInputs::default()
        },
    ));
    assert_eq!(t.sample_rate, Some(32_000));
}

#[test]
fn cover_art_is_not_counted_as_audio() {
    // 2 MB of art on a 100 s file at 80 kbps.
    let mut p = probe(AudioCodec::Mp3, 22_050, 1, 80, 100.0);
    p.picture_bytes = 2_000_000;
    p.size += p.picture_bytes;
    assert_eq!(p.real_bitrate(), Some(80_000));
    let t = target(run(&p, &settings()));
    assert!(t.estimated_size > p.picture_bytes, "art is kept");
}

#[test]
fn presets_are_stable() {
    assert_eq!(Encoding::Mp3Cbr { kbps: 32 }.preset(), "mp3-32k");
    assert_eq!(
        Encoding::Mp3Vbr {
            quality: 4,
            approx_kbps: 165
        }
        .preset(),
        "mp3-vbr4"
    );
    assert_eq!(Encoding::Aac { kbps: 64 }.preset(), "aac-64k");
    assert_eq!(Encoding::Opus { kbps: 24 }.preset(), "opus-24k");
    assert_eq!(Encoding::Flac.preset(), "flac");
}

#[test]
fn header_overhead_still_counts_as_at_target() {
    // 32.01 kbps measured on a 32 kbps file (headers and tags).
    let mut p = mp3_book(32, 22_050);
    p.size += p.size / 3000;
    assert_eq!(
        skip(run(&p, &settings())),
        SkipReason::AlreadyCompressed { kbps: 32 }
    );
    // 34 kbps is a real difference, and the margin decides.
    let p = mp3_book(34, 22_050);
    assert!(matches!(
        skip(run(&p, &settings())),
        SkipReason::SavingTooSmall { .. }
    ));
}
