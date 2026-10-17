//! The `r2dock-media` URI scheme for image and audio previews (PLAN.md 9.5).
//!
//! The webview gets an opaque, expiring token, never a presigned URL or a
//! credential. Rust fetches from R2 with Range support and returns only
//! allowlisted content types; everything else is served as an octet stream.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use http::{HeaderValue, Response, StatusCode, header};
use parking_lot::Mutex;
use uuid::Uuid;

/// Tokens expire after this long.
const MEDIA_TTL: Duration = Duration::from_secs(60 * 60);
const MAX_TOKENS: usize = 2000;
/// Largest response body for one ranged request.
pub const RANGE_CHUNK: u64 = 2 * 1024 * 1024;
/// Largest object served whole when the webview does not ask for a range.
pub const MAX_WHOLE: u64 = 64 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct MediaTarget {
    pub connection: Uuid,
    pub bucket: String,
    pub key: String,
}

#[derive(Default)]
pub struct MediaRegistry {
    map: Mutex<HashMap<String, (Instant, MediaTarget)>>,
}

impl MediaRegistry {
    pub fn issue(&self, target: MediaTarget) -> String {
        let token = Uuid::new_v4().simple().to_string();
        let mut map = self.map.lock();
        let now = Instant::now();
        map.retain(|_, (t, _)| now.duration_since(*t) < MEDIA_TTL);
        if map.len() >= MAX_TOKENS
            && let Some(k) = map
                .iter()
                .min_by_key(|(_, (t, _))| *t)
                .map(|(k, _)| k.clone())
        {
            map.remove(&k);
        }
        map.insert(token.clone(), (now, target));
        token
    }

    pub fn resolve(&self, token: &str) -> Option<MediaTarget> {
        let map = self.map.lock();
        let (t, target) = map.get(token)?;
        (t.elapsed() < MEDIA_TTL).then(|| target.clone())
    }
}

/// Safe content type by file extension. Unknown types are never rendered.
pub fn content_type_for(key: &str) -> &'static str {
    let ext = key.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "avif" => "image/avif",
        "bmp" => "image/bmp",
        "ico" => "image/x-icon",
        "svg" => "image/svg+xml",
        "mp3" => "audio/mpeg",
        "m4a" | "mp4a" => "audio/mp4",
        "aac" => "audio/aac",
        "ogg" | "oga" | "opus" => "audio/ogg",
        "wav" => "audio/wav",
        "flac" => "audio/flac",
        _ => "application/octet-stream",
    }
}

/// Token from a request path such as `/abc123` (case kept).
pub fn token_from_path(path: &str) -> Option<&str> {
    let t = path.trim_start_matches('/');
    let t = t.split(['?', '#', '/']).next().unwrap_or("");
    (t.len() == 32 && t.bytes().all(|b| b.is_ascii_hexdigit())).then_some(t)
}

/// Parse a single `bytes=a-b` / `bytes=a-` / `bytes=-n` range, clamped to one chunk.
/// Returns `(start, end_inclusive)` or `None` when absent or unsupported.
pub fn parse_range(value: Option<&str>, size: Option<u64>) -> Option<(u64, u64)> {
    let spec = value?.trim().strip_prefix("bytes=")?;
    if spec.contains(',') {
        return None;
    }
    let (a, b) = spec.split_once('-')?;
    let (start, end) = if a.is_empty() {
        let n: u64 = b.parse().ok()?;
        let size = size?;
        (size.saturating_sub(n), size.saturating_sub(1))
    } else {
        let start: u64 = a.parse().ok()?;
        let end = if b.is_empty() {
            u64::MAX
        } else {
            b.parse().ok()?
        };
        (start, end)
    };
    if end < start {
        return None;
    }
    let end = end.min(start.saturating_add(RANGE_CHUNK - 1));
    let end = match size {
        Some(s) if s > 0 => end.min(s - 1),
        _ => end,
    };
    Some((start, end))
}

pub fn error_response(status: StatusCode) -> Response<Vec<u8>> {
    let mut r = Response::new(Vec::new());
    *r.status_mut() = status;
    r
}

/// Build a response with safe headers.
pub fn media_response(
    status: StatusCode,
    content_type: &str,
    body: Vec<u8>,
    content_range: Option<String>,
) -> Response<Vec<u8>> {
    let len = body.len();
    let mut r = Response::new(body);
    *r.status_mut() = status;
    let h = r.headers_mut();
    if let Ok(v) = HeaderValue::from_str(content_type) {
        h.insert(header::CONTENT_TYPE, v);
    }
    h.insert(header::CONTENT_LENGTH, HeaderValue::from(len));
    h.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    h.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    h.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static("default-src 'none'; style-src 'unsafe-inline'; sandbox"),
    );
    if let Some(cr) = content_range.and_then(|c| HeaderValue::from_str(&c).ok()) {
        h.insert(header::CONTENT_RANGE, cr);
    }
    r
}

/// Serve one request for the media scheme.
pub async fn serve(app: &tauri::AppHandle, request: http::Request<Vec<u8>>) -> Response<Vec<u8>> {
    use r2dock_r2::R2Error;
    use tauri::Manager;

    let Some(state) = app.try_state::<crate::state::AppState>() else {
        return error_response(StatusCode::SERVICE_UNAVAILABLE);
    };
    let Some(token) = token_from_path(request.uri().path()) else {
        return error_response(StatusCode::NOT_FOUND);
    };
    let Some(target) = state.media.resolve(token) else {
        return error_response(StatusCode::NOT_FOUND);
    };
    let Ok(client) = state.connections.client(target.connection).await else {
        return error_response(StatusCode::BAD_GATEWAY);
    };
    let size = match client.head_object(&target.bucket, &target.key).await {
        Ok(d) => d.size,
        Err(R2Error::NoSuchKey { .. }) => return error_response(StatusCode::NOT_FOUND),
        Err(_) => return error_response(StatusCode::BAD_GATEWAY),
    };
    let content_type = content_type_for(&target.key);
    let range_header = request
        .headers()
        .get(header::RANGE)
        .and_then(|v| v.to_str().ok());
    let range = parse_range(range_header, Some(size)).or_else(|| {
        // Large objects are always served in chunks.
        (size > MAX_WHOLE).then_some((0, RANGE_CHUNK.min(size) - 1))
    });
    match range {
        Some((start, _)) if size == 0 || start >= size => {
            let mut r = error_response(StatusCode::RANGE_NOT_SATISFIABLE);
            if let Ok(v) = HeaderValue::from_str(&format!("bytes */{size}")) {
                r.headers_mut().insert(header::CONTENT_RANGE, v);
            }
            r
        }
        Some((start, end)) => {
            let out = client
                .get_object(
                    &target.bucket,
                    &target.key,
                    Some(format!("bytes={start}-{end}")),
                )
                .await;
            let Ok(out) = out else {
                return error_response(StatusCode::BAD_GATEWAY);
            };
            let Ok(body) = out.body.collect().await else {
                return error_response(StatusCode::BAD_GATEWAY);
            };
            media_response(
                StatusCode::PARTIAL_CONTENT,
                content_type,
                body.into_bytes().to_vec(),
                Some(format!("bytes {start}-{end}/{size}")),
            )
        }
        None => {
            let Ok(out) = client.get_object(&target.bucket, &target.key, None).await else {
                return error_response(StatusCode::BAD_GATEWAY);
            };
            let Ok(body) = out.body.collect().await else {
                return error_response(StatusCode::BAD_GATEWAY);
            };
            media_response(
                StatusCode::OK,
                content_type,
                body.into_bytes().to_vec(),
                None,
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_types_are_allowlisted() {
        assert_eq!(content_type_for("a/b.MP3"), "audio/mpeg");
        assert_eq!(content_type_for("x.webp"), "image/webp");
        assert_eq!(content_type_for("page.html"), "application/octet-stream");
        assert_eq!(content_type_for("script.js"), "application/octet-stream");
        assert_eq!(content_type_for("noext"), "application/octet-stream");
    }

    #[test]
    fn tokens_parse_strictly() {
        let t = "0123456789abcdef0123456789abcdef";
        assert_eq!(token_from_path(&format!("/{t}")), Some(t));
        assert_eq!(token_from_path(&format!("/{t}?x=1")), Some(t));
        assert_eq!(token_from_path("/../etc/passwd"), None);
        assert_eq!(token_from_path("/short"), None);
    }

    #[test]
    fn ranges() {
        assert_eq!(parse_range(None, Some(10)), None);
        assert_eq!(parse_range(Some("bytes=0-"), Some(10)), Some((0, 9)));
        assert_eq!(parse_range(Some("bytes=2-5"), Some(10)), Some((2, 5)));
        assert_eq!(parse_range(Some("bytes=-3"), Some(10)), Some((7, 9)));
        assert_eq!(
            parse_range(Some("bytes=0-"), None),
            Some((0, RANGE_CHUNK - 1))
        );
        assert_eq!(
            parse_range(Some("bytes=0-"), Some(100 * RANGE_CHUNK)),
            Some((0, RANGE_CHUNK - 1))
        );
        assert_eq!(parse_range(Some("bytes=5-2"), Some(10)), None);
        assert_eq!(parse_range(Some("bytes=0-1,4-5"), Some(10)), None);
        assert_eq!(parse_range(Some("items=0-1"), Some(10)), None);
    }

    #[test]
    fn registry_tokens() {
        let r = MediaRegistry::default();
        let t = r.issue(MediaTarget {
            connection: Uuid::nil(),
            bucket: "b".into(),
            key: "k".into(),
        });
        assert_eq!(r.resolve(&t).unwrap().key, "k");
        assert!(r.resolve("0123456789abcdef0123456789abcdef").is_none());
    }

    #[test]
    fn responses_have_safe_headers() {
        let r = media_response(
            StatusCode::PARTIAL_CONTENT,
            "audio/mpeg",
            vec![1, 2],
            Some("bytes 0-1/10".into()),
        );
        assert_eq!(r.headers()["x-content-type-options"], "nosniff");
        assert_eq!(r.headers()[header::CONTENT_RANGE], "bytes 0-1/10");
        assert_eq!(r.headers()[header::CONTENT_LENGTH], "2");
    }
}
