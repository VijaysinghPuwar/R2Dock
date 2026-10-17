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
        let host_start = lower[..pos]
            .rfind(|c: char| !(c.is_ascii_alphanumeric() || c == '.' || c == '-'))
            .map_or(0, |i| i + 1);
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
