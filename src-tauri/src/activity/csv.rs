//! One activity record as a CSV row (RFC 4180), safe to open in a
//! spreadsheet.
//!
//! - Every record is exactly one line: line breaks inside values become
//!   spaces, so files can be merged and sorted line by line.
//! - The first column is the UTC time in a fixed-width format, so sorting
//!   lines as text sorts them by time.
//! - Cells that a spreadsheet would read as a formula (`=`, `+`, `-`, `@`,
//!   tab, carriage return) get a leading `'` (CSV injection).
//! - Every cell passes through the redaction filter.

use chrono::{DateTime, Local, Utc};
use r2dock_core::redact::redact_text;
use serde_json::{Map, Value};

/// Column names, in order. Documented in the README.
pub const COLUMNS: [&str; 15] = [
    "time",
    "local_time",
    "session",
    "action",
    "outcome",
    "duration_ms",
    "connection",
    "bucket",
    "item",
    "files",
    "bytes",
    "error_code",
    "error_message",
    "request_id",
    "details",
];

/// Byte order mark, so Excel reads the file as UTF-8.
pub const BOM: &str = "\u{feff}";

/// Longest text kept for any single cell.
const MAX_CELL: usize = 500;

/// Fields that may name the main item of an action, most specific first.
const ITEM_FIELDS: [&str; 4] = ["key", "name", "prefix", "query"];

/// The fixed-width UTC time used in the first column.
pub fn format_time(time: DateTime<Utc>) -> String {
    time.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordError {
    pub code: String,
    pub message: String,
    pub detail: Option<String>,
}

/// One activity record before it is written.
#[derive(Debug, Clone)]
pub struct Record {
    pub time: DateTime<Utc>,
    pub session: String,
    pub action: String,
    pub outcome: String,
    pub duration_ms: Option<u64>,
    pub fields: Map<String, Value>,
    pub error: Option<RecordError>,
}

impl Record {
    /// The CSV line for this record, without a trailing newline.
    pub fn to_line(&self) -> String {
        let mut fields = self.fields.clone();
        let mut take = |name: &str| fields.remove(name).map(|v| value_text(&v));
        let connection = take("connection");
        let bucket = take("bucket");
        let item = ITEM_FIELDS.iter().find_map(|f| take(f));
        let files = take("files");
        let bytes = take("bytes");
        let (error_code, error_message, request_id) = match &self.error {
            Some(e) => (
                Some(e.code.clone()),
                Some(e.message.clone()),
                e.detail.as_deref().and_then(request_id),
            ),
            None => (None, None, None),
        };
        if let Some(detail) = self.error.as_ref().and_then(|e| e.detail.clone()) {
            fields.insert("error_detail".into(), Value::String(detail));
        }
        let details = fields
            .iter()
            .map(|(k, v)| format!("{k}={}", value_text(v)))
            .collect::<Vec<_>>()
            .join("; ");
        let cells = [
            Some(format_time(self.time)),
            Some(
                self.time
                    .with_timezone(&Local)
                    .format("%Y-%m-%d %H:%M:%S")
                    .to_string(),
            ),
            Some(self.session.clone()),
            Some(self.action.clone()),
            Some(self.outcome.clone()),
            self.duration_ms.map(|d| d.to_string()),
            connection,
            bucket,
            item,
            files,
            bytes,
            error_code,
            error_message,
            request_id,
            Some(details),
        ];
        cells
            .iter()
            .map(|c| cell(c.as_deref().unwrap_or("")))
            .collect::<Vec<_>>()
            .join(",")
    }
}

/// The header line, without a trailing newline.
pub fn header_line() -> String {
    COLUMNS.join(",")
}

/// A JSON value as plain cell text (strings without quotes).
fn value_text(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// The R2 request id from an error detail such as
/// `AccessDenied (403), request id abc`.
fn request_id(detail: &str) -> Option<String> {
    let rest = detail.split_once("request id ")?.1;
    let id: String = rest
        .chars()
        .take_while(|c| !c.is_whitespace() && *c != ',' && *c != ';')
        .collect();
    (!id.is_empty()).then_some(id)
}

/// One cell: one line, truncated, redacted, formula-safe, and quoted when
/// needed.
pub fn cell(raw: &str) -> String {
    let one_line: String = raw
        .chars()
        .map(|c| if c == '\n' || c == '\r' { ' ' } else { c })
        .collect();
    let mut text = redact_text(&truncate(&one_line));
    if text.starts_with(['=', '+', '-', '@', '\t', '\r']) {
        text.insert(0, '\'');
    }
    if text.contains([',', '"']) || text.starts_with(' ') || text.ends_with(' ') {
        format!("\"{}\"", text.replace('"', "\"\""))
    } else {
        text
    }
}

fn truncate(s: &str) -> String {
    if s.len() <= MAX_CELL {
        return s.to_string();
    }
    let mut end = MAX_CELL;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &s[..end])
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Minimal RFC 4180 parser for one line, used to check the writer.
    fn parse(line: &str) -> Vec<String> {
        let mut cells = Vec::new();
        let mut cur = String::new();
        let mut quoted = false;
        let mut chars = line.chars().peekable();
        while let Some(c) = chars.next() {
            match (quoted, c) {
                (true, '"') if chars.peek() == Some(&'"') => {
                    cur.push('"');
                    chars.next();
                }
                (true, '"') => quoted = false,
                (false, '"') if cur.is_empty() => quoted = true,
                (false, ',') => cells.push(std::mem::take(&mut cur)),
                _ => cur.push(c),
            }
        }
        cells.push(cur);
        cells
    }

    fn record(fields: Value, error: Option<RecordError>) -> Record {
        Record {
            time: DateTime::parse_from_rfc3339("2026-10-18T09:14:03.120Z")
                .unwrap()
                .with_timezone(&Utc),
            session: "3f2a9c1e".into(),
            action: "upload_start".into(),
            outcome: if error.is_some() { "error" } else { "ok" }.into(),
            duration_ms: Some(412),
            fields: fields.as_object().cloned().unwrap_or_default(),
            error,
        }
    }

    #[test]
    fn maps_fields_to_columns() {
        let line = record(
            json!({"connection": "1a2b3c4d", "bucket": "#9f8e7d6c", "key": "#1a2b3c4d",
                   "files": 195, "bytes": 345000000u64, "policy": "Ask", "cancelled": false}),
            None,
        )
        .to_line();
        let cells = parse(&line);
        assert_eq!(cells.len(), COLUMNS.len());
        assert_eq!(cells[0], "2026-10-18T09:14:03.120Z");
        assert_eq!(cells[2], "3f2a9c1e");
        assert_eq!(cells[3], "upload_start");
        assert_eq!(cells[4], "ok");
        assert_eq!(cells[5], "412");
        assert_eq!(cells[6], "1a2b3c4d");
        assert_eq!(cells[7], "#9f8e7d6c");
        assert_eq!(cells[8], "#1a2b3c4d");
        assert_eq!(cells[9], "195");
        assert_eq!(cells[10], "345000000");
        assert_eq!(cells[11], "");
        assert_eq!(cells[14], "cancelled=false; policy=Ask");
    }

    #[test]
    fn errors_fill_code_message_and_request_id() {
        let line = record(
            json!({}),
            Some(RecordError {
                code: "accessDenied".into(),
                message: "This key cannot do that.".into(),
                detail: Some("AccessDenied (403), request id abc-123".into()),
            }),
        )
        .to_line();
        let cells = parse(&line);
        assert_eq!(cells[4], "error");
        assert_eq!(cells[11], "accessDenied");
        assert_eq!(cells[12], "This key cannot do that.");
        assert_eq!(cells[13], "abc-123");
        assert_eq!(
            cells[14],
            "error_detail=AccessDenied (403), request id abc-123"
        );
    }

    #[test]
    fn item_prefers_key_then_name_then_prefix() {
        let cells = parse(&record(json!({"prefix": "p", "name": "n"}), None).to_line());
        assert_eq!(cells[8], "n");
        assert_eq!(cells[14], "prefix=p");
    }

    #[test]
    fn quoting_and_line_breaks() {
        let table = [
            ("plain", "plain"),
            ("a,b", "\"a,b\""),
            ("say \"hi\"", "\"say \"\"hi\"\"\""),
            ("two\nlines\r\nhere", "two lines  here"),
            (" padded ", "\" padded \""),
            ("Ünïcødé 名前.mp3", "Ünïcødé 名前.mp3"),
            ("", ""),
        ];
        for (raw, want) in table {
            assert_eq!(cell(raw), want, "{raw:?}");
        }
    }

    #[test]
    fn formula_injection_is_neutralized() {
        let table = [
            ("=cmd|' /C calc'!A1", "'=cmd|' /C calc'!A1"),
            ("+1", "'+1"),
            ("-2", "'-2"),
            ("@x", "'@x"),
            ("\tx", "'\tx"),
            (
                "=HYPERLINK(\"http://x\",\"y\")",
                "\"'=HYPERLINK(\"\"http://x\"\",\"\"y\"\")\"",
            ),
            ("a=b", "a=b"),
        ];
        for (raw, want) in table {
            assert_eq!(cell(raw), want, "{raw:?}");
        }
        // A carriage return at the start becomes a space, which is harmless.
        assert_eq!(cell("\r=1"), "\" =1\"");
    }

    #[test]
    fn every_line_is_one_line_with_all_columns() {
        let line = record(
            json!({"key": "evil\n\"name\",=1", "note": "x\ny"}),
            Some(RecordError {
                code: "io".into(),
                message: "Bad\nthing, \"quoted\"".into(),
                detail: None,
            }),
        )
        .to_line();
        assert!(!line.contains('\n') && !line.contains('\r'));
        let cells = parse(&line);
        assert_eq!(cells.len(), COLUMNS.len());
        assert_eq!(cells[8], "evil \"name\",=1");
        assert_eq!(cells[12], "Bad thing, \"quoted\"");
    }

    #[test]
    fn cells_are_redacted_and_truncated() {
        let line = record(json!({"note": "secret=FAKESECRETVALUE123"}), None).to_line();
        assert!(!line.contains("FAKESECRETVALUE123"));
        let account = "0123456789abcdef0123456789abcdef";
        let line = record(
            json!({"endpoint": format!("https://{account}.r2.cloudflarestorage.com")}),
            Some(RecordError {
                code: "network".into(),
                message: format!("Could not reach {account}.r2.cloudflarestorage.com"),
                detail: None,
            }),
        )
        .to_line();
        assert!(!line.contains(account), "{line}");
        let long = "x".repeat(2000);
        assert!(cell(&long).chars().count() <= MAX_CELL + 1);
    }

    #[test]
    fn header_matches_columns() {
        assert_eq!(parse(&header_line()), COLUMNS.to_vec());
        assert!(header_line().starts_with("time,local_time,session,action,outcome"));
    }

    #[test]
    fn time_format_sorts_as_text() {
        let a = format_time(
            DateTime::parse_from_rfc3339("2026-01-02T03:04:05Z")
                .unwrap()
                .into(),
        );
        let b = format_time(
            DateTime::parse_from_rfc3339("2026-01-02T03:04:05.5Z")
                .unwrap()
                .into(),
        );
        assert_eq!(a, "2026-01-02T03:04:05.000Z");
        assert!(a < b);
    }
}
