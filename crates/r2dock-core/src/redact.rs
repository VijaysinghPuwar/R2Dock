//! Redaction for anything that might end up in logs or error details.

use std::path::Path;

/// Header names and query parameters whose values must never be logged.
const SENSITIVE_MARKERS: [&str; 9] = [
    "authorization",
    "x-amz-signature",
    "x-amz-security-token",
    "x-amz-credential",
    "signature=",
    "credential=",
    "secret",
    "aws_secret_access_key",
    "token=",
];

const REDACTED: &str = "[REDACTED]";

/// R2 endpoint hosts start with the account id (`<account>.r2.cloudflarestorage.com`).
const R2_HOST: &str = "r2.cloudflarestorage.com";
const ACCOUNT: &str = "[ACCOUNT]";

fn is_value_end(c: char) -> bool {
    c.is_whitespace() || matches!(c, '&' | ',' | ';' | '"' | '\'' | ')' | ']' | '}' | '>')
}

/// Replace the value following any sensitive marker with `[REDACTED]`.
///
/// For `Authorization: AWS4-HMAC-SHA256 Credential=..., Signature=...` the
/// whole remainder of the line is removed.
pub fn redact_text(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for line in input.split_inclusive('\n') {
        out.push_str(&redact_line(line));
    }
    redact_account_hosts(&out)
}

/// Replace everything before `r2.cloudflarestorage.com` in a host name
/// (account id, jurisdiction, bucket) with `[ACCOUNT].`.
fn redact_account_hosts(input: &str) -> String {
    let lower = input.to_ascii_lowercase();
    let mut out = String::with_capacity(input.len());
    let mut idx = 0;
    while let Some(p) = lower[idx..].find(R2_HOST) {
        let pos = idx + p;
        // The host starts right after the last character that cannot be part
        // of a host name. That character may be multi-byte, so step past it by
        // its UTF-8 length rather than by one byte.
        let host_start = lower[..pos]
            .char_indices()
            .rev()
            .find(|&(_, c)| !(c.is_ascii_alphanumeric() || c == '.' || c == '-'))
            .map_or(0, |(i, c)| i + c.len_utf8());
        if host_start < pos && host_start >= idx {
            out.push_str(&input[idx..host_start]);
            out.push_str(ACCOUNT);
            out.push('.');
        } else {
            out.push_str(&input[idx..pos]);
        }
        out.push_str(&input[pos..pos + R2_HOST.len()]);
        idx = pos + R2_HOST.len();
    }
    out.push_str(&input[idx..]);
    out
}

fn redact_line(line: &str) -> String {
    let lower = line.to_ascii_lowercase();
    // Authorization headers: drop everything after the marker on this line.
    if let Some(pos) = lower.find("authorization") {
        let rest = &lower[pos + "authorization".len()..];
        let skip = rest.len() - rest.trim_start_matches(['"', ':', '=', ' ']).len();
        if skip > 0 {
            let cut = pos + "authorization".len() + skip;
            let newline = if line.ends_with('\n') { "\n" } else { "" };
            return format!("{}{REDACTED}{newline}", &line[..cut]);
        }
    }

    let mut out = String::with_capacity(line.len());
    let mut idx = 0;
    while idx < line.len() {
        let next = SENSITIVE_MARKERS
            .iter()
            .filter_map(|m| lower[idx..].find(m).map(|p| (idx + p, m.len())))
            .min_by_key(|(p, _)| *p);
        let Some((pos, len)) = next else {
            out.push_str(&line[idx..]);
            break;
        };
        let marker_end = pos + len;
        out.push_str(&line[idx..marker_end]);
        // Skip separators between the marker and its value.
        let after = &line[marker_end..];
        let sep_len = after.len() - after.trim_start_matches([':', '=', ' ', '"']).len();
        out.push_str(&after[..sep_len]);
        let value_start = marker_end + sep_len;
        let value_len = line[value_start..]
            .find(is_value_end)
            .unwrap_or(line.len() - value_start);
        if value_len > 0 {
            out.push_str(REDACTED);
        }
        idx = value_start + value_len;
    }
    out
}

/// Show a local path with the home directory replaced by `~`.
pub fn redact_path(path: &Path, home: Option<&Path>) -> String {
    if let Some(home) = home
        && let Ok(rest) = path.strip_prefix(home)
    {
        if rest.as_os_str().is_empty() {
            return "~".to_string();
        }
        return format!("~{}{}", std::path::MAIN_SEPARATOR, rest.display());
    }
    path.display().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_authorization_header() {
        let line = "authorization: AWS4-HMAC-SHA256 Credential=AKID/2026/auto/s3/aws4_request, Signature=abc123";
        let out = redact_text(line);
        assert!(!out.contains("AKID"));
        assert!(!out.contains("abc123"));
        assert!(out.contains(REDACTED));
    }

    #[test]
    fn redacts_presigned_query() {
        let url = "https://x.r2.cloudflarestorage.com/b/k?X-Amz-Credential=AK%2F1&X-Amz-Signature=deadbeef&x=1";
        let out = redact_text(url);
        assert!(!out.contains("deadbeef"));
        assert!(!out.contains("AK%2F1"));
        assert!(out.ends_with("&x=1"));
    }

    #[test]
    fn redacts_secret_fields() {
        let out = redact_text("secret_access_key=0123456789abcdef other=keep");
        assert!(!out.contains("0123456789abcdef"));
        assert!(out.contains("other=keep"));
        let json = redact_text("{\"secret\":\"topsecretvalue\"}");
        assert!(!json.contains("topsecretvalue"));
    }

    #[test]
    fn redacts_account_id_in_endpoints() {
        let account = "0123456789abcdef0123456789abcdef";
        for text in [
            format!("https://{account}.r2.cloudflarestorage.com/media-library"),
            format!("dns error for {account}.eu.r2.cloudflarestorage.com:443"),
            format!("{}.R2.CLOUDFLARESTORAGE.COM", account.to_uppercase()),
        ] {
            let out = redact_text(&text);
            assert!(!out.to_lowercase().contains(account), "{out}");
            assert!(out.contains("[ACCOUNT]."), "{out}");
        }
        assert_eq!(
            redact_text("see r2.cloudflarestorage.com docs"),
            "see r2.cloudflarestorage.com docs"
        );
    }

    #[test]
    fn unicode_next_to_hosts_and_markers_does_not_panic() {
        let account = "0123456789abcdef0123456789abcdef";
        let cases: &[(&str, &str)] = &[
            (
                "\u{e9}account.r2.cloudflarestorage.com",
                "\u{e9}[ACCOUNT].r2.cloudflarestorage.com",
            ),
            (
                "\u{4e2d}.r2.cloudflarestorage.com",
                "\u{4e2d}[ACCOUNT].r2.cloudflarestorage.com",
            ),
            (
                "\u{4e2d}r2.cloudflarestorage.com",
                "\u{4e2d}r2.cloudflarestorage.com",
            ),
            (
                "\u{1f600}x.r2.cloudflarestorage.com/\u{1f600}",
                "\u{1f600}[ACCOUNT].r2.cloudflarestorage.com/\u{1f600}",
            ),
            ("r2.cloudflarestorage.com", "r2.cloudflarestorage.com"),
            ("", ""),
            ("\u{e9}secret=\u{e9}\u{e9} ok", "\u{e9}secret=[REDACTED] ok"),
            ("token=\u{4e2d}&x=\u{4e2d}", "token=[REDACTED]&x=\u{4e2d}"),
        ];
        for (input, expected) in cases {
            assert_eq!(redact_text(input), *expected, "input {input:?}");
        }
        let multi = format!(
            "\u{e9}{account}.r2.cloudflarestorage.com and \u{4e2d}{account}.eu.r2.cloudflarestorage.com"
        );
        let out = redact_text(&multi);
        assert!(!out.contains(account), "{out}");
        assert_eq!(out.matches("[ACCOUNT].").count(), 2, "{out}");
        let ctl = format!("\u{7}{account}.r2.cloudflarestorage.com\u{0}");
        assert!(!redact_text(&ctl).contains(account));
    }

    proptest::proptest! {
        #[test]
        fn redaction_is_total_over_utf8(
            prefix in "\\PC{0,12}",
            host in "[a-z0-9.\u{e9}\u{4e2d}-]{0,12}",
            suffix in "\\PC{0,12}",
            marker in proptest::sample::select(vec!["", "secret=", "token=", "Authorization: ", "r2.cloudflarestorage.com"]),
        ) {
            let text = format!("{prefix}{host}.r2.cloudflarestorage.com{marker}{suffix}");
            let out = redact_text(&text);
            proptest::prop_assert!(out.contains("r2.cloudflarestorage.com"));
        }

        #[test]
        fn known_account_is_always_masked(prefix in "\\PC{0,8}", account in "[a-f0-9]{32}") {
            let out = redact_text(&format!("{prefix} {account}.r2.cloudflarestorage.com"));
            proptest::prop_assert!(!out.contains(&account), "{}", out);
        }
    }

    #[test]
    fn leaves_ordinary_text() {
        assert_eq!(redact_text("Uploaded 3 files"), "Uploaded 3 files");
    }

    #[test]
    fn path_home_replacement() {
        let home = Path::new("/Users/alice");
        assert_eq!(
            redact_path(Path::new("/Users/alice/Downloads/a.txt"), Some(home)),
            format!("~{}Downloads/a.txt", std::path::MAIN_SEPARATOR)
        );
        assert_eq!(redact_path(Path::new("/tmp/x"), Some(home)), "/tmp/x");
    }
}
