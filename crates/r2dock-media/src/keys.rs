//! The object key a compressed file is uploaded to.

/// `key` with its file extension replaced by `ext`, or unchanged when `ext`
/// is `None` or already the extension (in any letter case). A name with no
/// extension, or one that is only a leading dot (`.hidden`), gains `.ext`.
pub fn output_key(key: &str, ext: Option<&str>) -> String {
    let Some(ext) = ext else {
        return key.to_string();
    };
    let name_start = key.rfind('/').map_or(0, |i| i + 1);
    let name = &key[name_start..];
    match name.rfind('.') {
        Some(dot) if dot > 0 => {
            if name[dot + 1..].eq_ignore_ascii_case(ext) {
                key.to_string()
            } else {
                format!("{}.{ext}", &key[..name_start + dot])
            }
        }
        _ => format!("{key}.{ext}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table() {
        let table = [
            ("a/b/track.wav", Some("mp3"), "a/b/track.mp3"),
            ("a/b/track.WAV", Some("mp3"), "a/b/track.mp3"),
            ("a/b/track.MP3", Some("mp3"), "a/b/track.MP3"),
            ("book.m4b", None, "book.m4b"),
            ("song.flac", Some("opus"), "song.opus"),
            ("no-extension", Some("mp3"), "no-extension.mp3"),
            (
                "dir.v2/no-extension",
                Some("mp3"),
                "dir.v2/no-extension.mp3",
            ),
            (".hidden", Some("mp3"), ".hidden.mp3"),
            ("a/archive.tar.wav", Some("flac"), "a/archive.tar.flac"),
            ("名前/歌.Wav", Some("mp3"), "名前/歌.mp3"),
        ];
        for (key, ext, want) in table {
            assert_eq!(output_key(key, ext), want, "{key}");
        }
    }
}
