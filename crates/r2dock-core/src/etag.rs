//! ETag helpers.
//!
//! An ETag is a full-file MD5 only for single-part uploads without SSE-C.
//! Multipart ETags are the MD5 of the concatenated part MD5s plus `-<parts>`.

use md5::{Digest, Md5};

/// Strip quotes and weak markers, lowercase.
pub fn normalize(etag: &str) -> String {
    etag.trim()
        .trim_start_matches("W/")
        .trim_matches('"')
        .to_ascii_lowercase()
}

/// True when the ETag looks like a plain MD5 of the whole object.
/// Callers must also know the object was not encrypted with SSE-C.
pub fn is_single_part_md5(etag: &str) -> bool {
    let e = normalize(etag);
    e.len() == 32 && e.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Number of parts encoded in a multipart ETag, if any.
pub fn multipart_part_count(etag: &str) -> Option<u32> {
    let e = normalize(etag);
    let (hash, count) = e.split_once('-')?;
    if hash.len() != 32 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    count.parse().ok()
}

/// Expected ETag for a multipart object from its part MD5 digests.
pub fn multipart_etag(part_md5s: &[[u8; 16]]) -> String {
    let mut hasher = Md5::new();
    for d in part_md5s {
        hasher.update(d);
    }
    format!("{}-{}", hex::encode(hasher.finalize()), part_md5s.len())
}

/// MD5 of a byte slice as a 16-byte array.
pub fn md5_bytes(data: &[u8]) -> [u8; 16] {
    Md5::digest(data).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes() {
        assert_eq!(normalize("\"ABCDEF\""), "abcdef");
        assert_eq!(normalize("W/\"abc\""), "abc");
    }

    #[test]
    fn single_part_detection() {
        assert!(is_single_part_md5("\"9b2cf1e04a7d63e8c5f1a2b3d4e5f601\""));
        assert!(!is_single_part_md5(
            "\"9b2cf1e04a7d63e8c5f1a2b3d4e5f601-3\""
        ));
        assert!(!is_single_part_md5("nothex"));
        assert_eq!(
            multipart_part_count("\"9b2cf1e04a7d63e8c5f1a2b3d4e5f601-3\""),
            Some(3)
        );
        assert_eq!(
            multipart_part_count("9b2cf1e04a7d63e8c5f1a2b3d4e5f601"),
            None
        );
    }

    #[test]
    fn multipart_etag_matches_s3_rule() {
        let a = md5_bytes(b"part one");
        let b = md5_bytes(b"part two");
        let mut concat = Vec::new();
        concat.extend_from_slice(&a);
        concat.extend_from_slice(&b);
        let expected = format!("{}-2", hex::encode(md5_bytes(&concat)));
        assert_eq!(multipart_etag(&[a, b]), expected);
    }

    #[test]
    fn md5_known_value() {
        assert_eq!(
            hex::encode(md5_bytes(b"")),
            "d41d8cd98f00b204e9800998ecf8427e"
        );
    }
}
