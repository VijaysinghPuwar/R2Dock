use super::*;
use crate::classify::Content;
use std::path::PathBuf;

fn abs(name: &str) -> PathBuf {
    std::env::temp_dir().join(name)
}

fn target(encoding: Encoding, channels: u8, rate: Option<u32>, content: Content) -> AudioTarget {
    AudioTarget {
        encoding,
        channels,
        sample_rate: rate,
        extension: None,
        content,
        estimated_size: 0,
    }
}

fn text(args: &[OsString]) -> Vec<String> {
    args.iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect()
}

/// The value after `flag`.
fn after<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    let i = args.iter().position(|a| a == flag)?;
    args.get(i + 1).map(String::as_str)
}

#[test]
fn speech_mp3_snapshot() {
    let t = target(
        Encoding::Mp3Cbr { kbps: 32 },
        1,
        Some(22_050),
        Content::Speech,
    );
    let input = abs("book.mp3");
    let output = abs("job.mp3");
    let args = text(&encode_audio(&input, &output, &t, None).unwrap());
    let want: Vec<String> = [
        "-hide_banner",
        "-nostdin",
        "-y",
        "-loglevel",
        "error",
        "-protocol_whitelist",
        "file",
        "-i",
        &format!("file:{}", input.display()),
        "-map",
        "0:a:0",
        "-map",
        "0:v?",
        "-c:v",
        "copy",
        "-disposition:v",
        "attached_pic",
        "-map_metadata",
        "0",
        "-map_chapters",
        "0",
        "-c:a",
        "libmp3lame",
        "-b:a",
        "32k",
        "-ac",
        "1",
        "-ar",
        "22050",
        "-id3v2_version",
        "3",
        "-write_xing",
        "1",
        "-metadata",
        "R2DOCK_COMPRESSED=mp3-32k",
        "-progress",
        "pipe:1",
        "-nostats",
        "-f",
        "mp3",
        &format!("file:{}", output.display()),
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    assert_eq!(args, want);
}

#[test]
fn every_encoding() {
    let cases = [
        (
            target(
                Encoding::Mp3Vbr {
                    quality: 4,
                    approx_kbps: 165,
                },
                2,
                Some(44_100),
                Content::Music,
            ),
            "x.mp3",
            vec![("-q:a", "4"), ("-c:a", "libmp3lame"), ("-ac", "2")],
        ),
        (
            target(Encoding::Aac { kbps: 64 }, 1, Some(22_050), Content::Speech),
            "x.m4b",
            vec![
                ("-c:a", "aac"),
                ("-b:a", "64k"),
                ("-f", "ipod"),
                ("-movflags", "+faststart"),
            ],
        ),
        (
            target(Encoding::Opus { kbps: 24 }, 1, None, Content::Speech),
            "x.opus",
            vec![
                ("-c:a", "libopus"),
                ("-application", "voip"),
                ("-f", "opus"),
            ],
        ),
        (
            target(Encoding::Opus { kbps: 96 }, 2, None, Content::Music),
            "x.opus",
            vec![("-application", "audio"), ("-b:a", "96k")],
        ),
        (
            target(Encoding::Flac, 6, Some(48_000), Content::Music),
            "x.flac",
            vec![("-c:a", "flac"), ("-ac", "6"), ("-f", "flac")],
        ),
    ];
    for (t, out, expect) in cases {
        let args = text(&encode_audio(&abs("in.wav"), &abs(out), &t, None).unwrap());
        for (flag, value) in expect {
            assert_eq!(after(&args, flag), Some(value), "{out} {flag}");
        }
    }
}

#[test]
fn marker_everywhere_but_mp4() {
    let t = target(Encoding::Aac { kbps: 64 }, 1, None, Content::Speech);
    for (out, marked) in [
        ("o.m4a", false),
        ("o.mp4", false),
        ("o.aac", true),
        ("o.mp3", true),
        ("o.flac", true),
        ("o.opus", true),
    ] {
        let args = text(&encode_audio(&abs("in.m4a"), &abs(out), &t, None).unwrap());
        let has = args.iter().any(|a| a.starts_with("R2DOCK_COMPRESSED="));
        assert_eq!(has, marked, "{out}");
    }
}

#[test]
fn opus_drops_cover_art_and_rate() {
    let t = target(Encoding::Opus { kbps: 24 }, 1, None, Content::Speech);
    let args = text(&encode_audio(&abs("in.mp3"), &abs("out.opus"), &t, None).unwrap());
    assert!(args.contains(&"-vn".to_string()));
    assert!(!args.contains(&"0:v?".to_string()));
    assert!(!args.contains(&"-ar".to_string()));
}

#[test]
fn sample_window_comes_before_input() {
    let t = target(
        Encoding::Mp3Cbr { kbps: 32 },
        1,
        Some(22_050),
        Content::Speech,
    );
    let w = Window {
        start_secs: 41_000.5,
        len_secs: 30.0,
    };
    let args = text(&encode_audio(&abs("in.mp3"), &abs("s.mp3"), &t, Some(w)).unwrap());
    let ss = args.iter().position(|a| a == "-ss").unwrap();
    let i = args.iter().position(|a| a == "-i").unwrap();
    assert!(ss < i);
    assert_eq!(after(&args, "-ss"), Some("41000.500"));
    assert_eq!(after(&args, "-t"), Some("30.000"));
    assert!(args.contains(&"-vn".to_string()), "samples skip cover art");
}

#[test]
fn hostile_names_stay_files() {
    let t = target(Encoding::Mp3Cbr { kbps: 32 }, 1, None, Content::Speech);
    for name in [
        "concat:a|b.mp3",
        "http:x.mp3",
        "-i.mp3",
        "pipe:0.mp3",
        "a b;rm -rf.mp3",
    ] {
        let input = abs(name);
        let args = encode_audio(&input, &abs("o.mp3"), &t, None).unwrap();
        let i = args.iter().position(|a| a == "-i").unwrap();
        let mut want = OsString::from("file:");
        want.push(input.as_os_str());
        assert_eq!(args[i + 1], want, "{name}");
        assert!(
            text(&args)
                .windows(2)
                .any(|w| w[0] == "-protocol_whitelist" && w[1] == "file")
        );
    }
}

#[test]
fn rejects_relative_paths_and_unknown_containers() {
    let t = target(Encoding::Mp3Cbr { kbps: 32 }, 1, None, Content::Speech);
    assert_eq!(
        encode_audio(Path::new("in.mp3"), &abs("o.mp3"), &t, None),
        Err(ArgsError::RelativePath)
    );
    assert_eq!(
        encode_audio(&abs("in.mp3"), Path::new("o.mp3"), &t, None),
        Err(ArgsError::RelativePath)
    );
    assert_eq!(
        encode_audio(&abs("in.mp3"), &abs("o.exe"), &t, None),
        Err(ArgsError::UnknownContainer)
    );
    assert_eq!(Container::from_path(&abs("A.M4B")), Some(Container::Ipod));
}

#[test]
fn channel_difference_reads_the_whole_stream() {
    let args = text(&channel_difference(&abs("in.mp3")).unwrap());
    let graph = after(&args, "-filter_complex").unwrap();
    assert!(graph.starts_with("[0:a:0]asplit=2"), "{graph}");
    assert!(
        graph.contains("pan=mono|c0=c0-c1,volumedetect[d]"),
        "{graph}"
    );
    assert!(
        graph.contains("pan=mono|c0=0.5*c0+0.5*c1,volumedetect[s]"),
        "{graph}"
    );
    assert_eq!(after(&args, "-map"), Some("[d]"));
    assert!(
        !args.contains(&"-ss".to_string()),
        "no window: the whole file"
    );
    assert!(!args.contains(&"-t".to_string()));
    assert_eq!(after(&args, "-progress"), Some("pipe:1"));
    assert_eq!(after(&args, "-f"), Some("null"));
    assert_eq!(args.last().map(String::as_str), Some("-"));
    assert_eq!(
        channel_difference(Path::new("in.mp3")),
        Err(ArgsError::RelativePath)
    );
}
