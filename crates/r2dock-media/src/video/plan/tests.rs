use super::*;
use crate::video::probe::StreamInfo;

fn stream(index: u32, kind: StreamKind, codec: &str) -> StreamInfo {
    StreamInfo {
        index,
        kind,
        codec: codec.into(),
        width: 0,
        height: 0,
        pix_fmt: String::new(),
        color_transfer: String::new(),
        bit_rate: None,
        channels: 0,
        attached_pic: false,
        rotation: 0,
        fps: None,
        tag: String::new(),
    }
}

/// A 31 minute 1080p H.264 web video like the user's test file.
fn web_video() -> VideoProbe {
    let mut v = stream(0, StreamKind::Video, "h264");
    v.width = 1920;
    v.height = 1080;
    v.pix_fmt = "yuv420p".into();
    v.color_transfer = "bt709".into();
    v.bit_rate = Some(3_823_326);
    v.fps = Some(30.0);
    let mut a = stream(1, StreamKind::Audio, "aac");
    a.bit_rate = Some(185_967);
    a.channels = 2;
    VideoProbe {
        size: 936_788_628,
        duration_secs: Some(1865.988),
        streams: vec![v, a],
        bit_rate: Some(4_016_268),
    }
}

fn all_encoders() -> VideoEncoders {
    VideoEncoders {
        h264: Some(H264Encoder::VideoToolbox),
        openh264: true,
        av1: true,
    }
}

fn settings(codec: VideoCodec, size: VideoSize) -> VideoSettings {
    VideoSettings { codec, size }
}

fn run(p: &VideoProbe, s: &VideoSettings) -> Result<VideoTarget, SkipReason> {
    plan_video(p, s, &all_encoders(), 15, "mp4")
}

#[test]
fn av1_shrinks_the_web_video_by_about_two_thirds() {
    let t = run(&web_video(), &VideoSettings::default()).unwrap();
    assert_eq!(t.encoder, VideoEncoder::SvtAv1);
    assert_eq!(t.scale_short_side, None, "already 1080p");
    assert_eq!(
        t.audio,
        vec![(1, TrackAudio::Aac { kbps: 128 })],
        "186 kbps AAC is re-encoded"
    );
    let saved = 100.0 * (1.0 - t.estimated_size as f64 / web_video().size as f64);
    assert!((60.0..75.0).contains(&saved), "{saved}");
    assert_eq!(t.extension, None);
    assert!(!t.ten_bit);
}

#[test]
fn h264_target_is_decided_by_the_sample_test() {
    // The estimate says about 10% for H.264 at high quality, below 15%.
    let err = run(
        &web_video(),
        &settings(VideoCodec::H264, VideoSize::Balanced1080),
    )
    .unwrap_err();
    assert!(matches!(err, SkipReason::SavingTooSmall { .. }), "{err:?}");
    // A large phone recording does shrink with H.264.
    let mut phone = web_video();
    phone.streams[0].bit_rate = Some(16_000_000);
    phone.size = (16_200_000.0 * 1865.988 / 8.0) as u64;
    let t = run(&phone, &settings(VideoCodec::H264, VideoSize::Balanced1080)).unwrap();
    assert_eq!(t.encoder, VideoEncoder::H264(H264Encoder::VideoToolbox));
}

#[test]
fn scaling_uses_the_short_side() {
    assert_eq!(scaled_size(3840, 2160, Some(1080)), (1920, 1080, true));
    assert_eq!(
        scaled_size(2160, 3840, Some(1080)),
        (1080, 1920, true),
        "portrait"
    );
    assert_eq!(scaled_size(1920, 1080, Some(1080)), (1920, 1080, false));
    assert_eq!(scaled_size(1920, 1080, Some(720)), (1280, 720, true));
    assert_eq!(
        scaled_size(1918, 1078, Some(720)),
        (1282, 720, true),
        "even numbers"
    );
    assert_eq!(scaled_size(3840, 2160, None), (3840, 2160, false));
    let mut uhd = web_video();
    uhd.streams[0].width = 3840;
    uhd.streams[0].height = 2160;
    uhd.streams[0].bit_rate = Some(20_000_000);
    let t = run(&uhd, &settings(VideoCodec::Av1, VideoSize::Small720)).unwrap();
    assert_eq!(t.scale_short_side, Some(720));
}

#[test]
fn skips() {
    let s = VideoSettings::default();
    assert_eq!(
        run(&web_video(), &settings(VideoCodec::Av1, VideoSize::Off)),
        Err(SkipReason::VideoOff)
    );
    let mut hdr = web_video();
    hdr.streams[0].color_transfer = "smpte2084".into();
    assert_eq!(run(&hdr, &s), Err(SkipReason::Hdr));
    let mut hlg = web_video();
    hlg.streams[0].color_transfer = "arib-std-b67".into();
    assert_eq!(run(&hlg, &s), Err(SkipReason::Hdr));
    let mut av1 = web_video();
    av1.streams[0].codec = "av1".into();
    assert_eq!(
        run(&av1, &s),
        Err(SkipReason::AlreadyEfficient {
            codec: "av1".into()
        })
    );
    let mut hevc = web_video();
    hevc.streams[0].codec = "hevc".into();
    assert!(run(&hevc, &s).is_ok(), "HEVC to AV1 still pays off");
    assert_eq!(
        run(&hevc, &settings(VideoCodec::H264, VideoSize::Balanced1080)),
        Err(SkipReason::AlreadyEfficient {
            codec: "hevc".into()
        })
    );
    let mut pgs = web_video();
    pgs.streams
        .push(stream(2, StreamKind::Subtitle, "hdmv_pgs_subtitle"));
    assert_eq!(run(&pgs, &s), Err(SkipReason::UnsupportedStreams));
    let mut gopro = web_video();
    let mut data = stream(2, StreamKind::Data, "bin_data");
    data.tag = "gpmd".into();
    gopro.streams.push(data);
    assert_eq!(run(&gopro, &s), Err(SkipReason::UnsupportedStreams));
    let mut short = web_video();
    short.duration_secs = Some(8.0);
    assert_eq!(run(&short, &s), Err(SkipReason::TooShort));
    let mut tiny = web_video();
    tiny.size = 500_000;
    assert_eq!(run(&tiny, &s), Err(SkipReason::TooSmall));
    let mut low = web_video();
    low.streams[0].bit_rate = Some(900_000);
    assert_eq!(
        run(&low, &s),
        Err(SkipReason::AlreadyCompressed { kbps: 900 })
    );
}

#[test]
fn keeps_what_matters() {
    let mut p = web_video();
    // iPhone style: metadata and timecode tracks are dropped safely.
    let mut mebx = stream(2, StreamKind::Data, "none");
    mebx.tag = "mebx".into();
    let mut tmcd = stream(3, StreamKind::Data, "none");
    tmcd.tag = "tmcd".into();
    p.streams.push(mebx);
    p.streams.push(tmcd);
    // A second audio track (commentary, mono AAC at 64 kbps) is copied.
    let mut a2 = stream(4, StreamKind::Audio, "aac");
    a2.channels = 1;
    a2.bit_rate = Some(64_000);
    p.streams.push(a2);
    // 5.1 AC-3 becomes AAC at 384 kbps.
    let mut a3 = stream(5, StreamKind::Audio, "ac3");
    a3.channels = 6;
    a3.bit_rate = Some(640_000);
    p.streams.push(a3);
    p.streams.push(stream(6, StreamKind::Subtitle, "subrip"));
    let mut cover = stream(7, StreamKind::Video, "mjpeg");
    cover.attached_pic = true;
    p.streams.push(cover);
    let t = run(&p, &VideoSettings::default()).unwrap();
    assert_eq!(t.video_index, 0);
    assert_eq!(
        t.audio,
        vec![
            (1, TrackAudio::Aac { kbps: 128 }),
            (4, TrackAudio::Copy),
            (5, TrackAudio::Aac { kbps: 384 })
        ]
    );
    assert_eq!(t.subtitles, vec![6]);
}

#[test]
fn ten_bit_and_containers() {
    let mut p = web_video();
    p.streams[0].pix_fmt = "yuv420p10le".into();
    let t = run(&p, &VideoSettings::default()).unwrap();
    assert!(t.ten_bit, "10-bit SDR stays 10-bit in AV1");
    let t = plan_video(
        &web_video(),
        &VideoSettings::default(),
        &all_encoders(),
        15,
        "MOV",
    )
    .unwrap();
    assert_eq!(t.extension.as_deref(), Some("mp4"));
    let t = plan_video(
        &web_video(),
        &VideoSettings::default(),
        &all_encoders(),
        15,
        "m4v",
    )
    .unwrap();
    assert_eq!(t.extension, None);
}

#[test]
fn encoder_choice_and_fallbacks() {
    let no_av1 = VideoEncoders {
        h264: Some(H264Encoder::Nvenc),
        openh264: true,
        av1: false,
    };
    let mut phone = web_video();
    phone.streams[0].bit_rate = Some(16_000_000);
    phone.size = (16_200_000.0 * 1865.988 / 8.0) as u64;
    let t = plan_video(&phone, &VideoSettings::default(), &no_av1, 15, "mp4").unwrap();
    assert_eq!(
        t.encoder,
        VideoEncoder::H264(H264Encoder::Nvenc),
        "AV1 missing: H.264"
    );
    let none = VideoEncoders::default();
    assert_eq!(
        plan_video(&phone, &VideoSettings::default(), &none, 15, "mp4"),
        Err(SkipReason::NoVideoEncoder)
    );
    assert_eq!(VideoEncoder::SvtAv1.preset(), "av1-crf35");
    assert_eq!(
        VideoEncoder::H264(H264Encoder::Qsv).preset(),
        "h264-h264_qsv"
    );
    assert!(!H264Encoder::OpenH264.is_hardware());
}
