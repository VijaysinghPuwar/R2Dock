//! Activity log: one JSON line per user action and its result, for
//! troubleshooting. Files are `activity-YYYY-MM-DD.jsonl` in a folder the
//! user can change in Settings.
//!
//! Privacy rules:
//! - Never records secrets, request arguments, signatures, or clipboard text.
//! - Object keys, bucket names, and local file names are written as a short
//!   hash (`#1a2b3c4d`) unless the user turns on "Include file and folder
//!   names". The same name always gives the same hash, so events can still be
//!   matched up.
//! - Every line also passes through the redaction filter.
//! - Writes happen on a background thread, so commands never wait on disk.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Sender, channel};
use std::time::{Duration, Instant, SystemTime};

use chrono::Utc;
use parking_lot::RwLock;
use r2dock_core::AppError;
use r2dock_core::redact::redact_text;
use serde_json::{Map, Value, json};

/// Longest text kept for any single field.
const MAX_FIELD: usize = 500;

pub struct ActivityLog {
    dir: Arc<RwLock<PathBuf>>,
    enabled: AtomicBool,
    names: AtomicBool,
    session: String,
    tx: Option<Sender<String>>,
}

impl ActivityLog {
    /// Start the log and its writer thread.
    pub fn start(dir: PathBuf, enabled: bool, names: bool) -> Arc<Self> {
        let dir = Arc::new(RwLock::new(dir));
        let (tx, rx) = channel::<String>();
        let writer_dir = dir.clone();
        let spawned = std::thread::Builder::new()
            .name("r2dock-activity".into())
            .spawn(move || {
                while let Ok(line) = rx.recv() {
                    let dir = writer_dir.read().clone();
                    write_line(&dir, &line);
                }
            })
            .is_ok();
        Arc::new(Self {
            dir,
            enabled: AtomicBool::new(enabled),
            names: AtomicBool::new(names),
            session: uuid::Uuid::new_v4().simple().to_string()[..8].to_string(),
            tx: spawned.then_some(tx),
        })
    }

    pub fn configure(&self, enabled: bool, names: bool, dir: PathBuf) {
        self.enabled.store(enabled, Ordering::Relaxed);
        self.names.store(names, Ordering::Relaxed);
        *self.dir.write() = dir;
    }

    pub fn dir(&self) -> PathBuf {
        self.dir.read().clone()
    }

    /// A name as it should appear in the log: plain when the user allowed
    /// names, otherwise a stable short hash.
    pub fn name(&self, value: &str) -> String {
        if self.names.load(Ordering::Relaxed) {
            truncate(&r2dock_core::keys::display_name(value))
        } else if value.is_empty() {
            String::new()
        } else {
            let digest = r2dock_core::etag::md5_bytes(value.as_bytes());
            format!("#{}", hex::encode(&digest[..4]))
        }
    }

    /// Begin recording an action.
    pub fn start_action(self: &Arc<Self>, action: &str) -> Act {
        Act {
            log: self.clone(),
            action: action.to_string(),
            fields: Map::new(),
            started: Instant::now(),
        }
    }

    /// Record a one-off event.
    pub fn event(&self, action: &str, fields: Map<String, Value>) {
        self.emit(action, "info", None, fields, None);
    }

    fn emit(
        &self,
        action: &str,
        outcome: &str,
        duration: Option<Duration>,
        fields: Map<String, Value>,
        error: Option<&AppError>,
    ) {
        if !self.enabled.load(Ordering::Relaxed) {
            return;
        }
        let Some(tx) = &self.tx else { return };
        let mut line = json!({
            "time": Utc::now().to_rfc3339(),
            "session": self.session,
            "action": action,
            "outcome": outcome,
        });
        if let Some(obj) = line.as_object_mut() {
            if let Some(d) = duration {
                obj.insert(
                    "ms".into(),
                    json!(u64::try_from(d.as_millis()).unwrap_or(u64::MAX)),
                );
            }
            if !fields.is_empty() {
                obj.insert("details".into(), Value::Object(fields));
            }
            if let Some(e) = error {
                obj.insert(
                    "error".into(),
                    json!({
                        "code": e.code,
                        "message": truncate(&e.message),
                        "detail": e.detail.as_deref().map(truncate),
                    }),
                );
            }
        }
        let _ = tx.send(redact_text(&line.to_string()));
    }

    /// Delete activity files older than `days`. Blocking.
    pub fn trim(&self, days: u32) {
        let dir = self.dir();
        let Ok(entries) = std::fs::read_dir(&dir) else {
            return;
        };
        let cutoff = SystemTime::now() - Duration::from_secs(u64::from(days) * 86_400);
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !(name.starts_with("activity-") && name.ends_with(".jsonl")) {
                continue;
            }
            let old = entry
                .metadata()
                .and_then(|m| m.modified())
                .is_ok_and(|t| t < cutoff);
            if old {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
}

fn truncate(s: &str) -> String {
    if s.len() <= MAX_FIELD {
        return s.to_string();
    }
    let mut end = MAX_FIELD;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &s[..end])
}

fn write_line(dir: &Path, line: &str) {
    if std::fs::create_dir_all(dir).is_err() {
        return;
    }
    let file = dir.join(format!("activity-{}.jsonl", Utc::now().format("%Y-%m-%d")));
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(file) {
        let _ = writeln!(f, "{line}");
    }
}

/// One action being recorded. Call [`Act::end`] with the result.
pub struct Act {
    log: Arc<ActivityLog>,
    action: String,
    fields: Map<String, Value>,
    started: Instant,
}

impl Act {
    /// Add a plain detail (counts, flags, ids). Never pass secrets or names.
    pub fn field(mut self, key: &str, value: impl Into<Value>) -> Self {
        self.fields.insert(key.into(), value.into());
        self
    }

    /// Add a bucket name, object key, or file name (hashed unless allowed).
    pub fn name(mut self, key: &str, value: &str) -> Self {
        let v = self.log.name(value);
        self.fields.insert(key.into(), Value::String(v));
        self
    }

    /// Add a connection id (short form; not personal).
    pub fn connection(self, id: uuid::Uuid) -> Self {
        let short = id.simple().to_string()[..8].to_string();
        self.field("connection", short)
    }

    pub fn set(&mut self, key: &str, value: impl Into<Value>) {
        self.fields.insert(key.into(), value.into());
    }

    /// Finish with a command result.
    pub fn end<T>(self, result: &Result<T, AppError>) {
        match result {
            Ok(_) => self.log.emit(
                &self.action,
                "ok",
                Some(self.started.elapsed()),
                self.fields,
                None,
            ),
            Err(e) => self.log.emit(
                &self.action,
                "error",
                Some(self.started.elapsed()),
                self.fields,
                Some(e),
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_all(dir: &Path) -> String {
        let mut out = String::new();
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            out.push_str(&std::fs::read_to_string(e.path()).unwrap());
        }
        out
    }

    fn wait_for_lines(dir: &Path, n: usize) -> String {
        for _ in 0..200 {
            if dir.exists() {
                let s = read_all(dir);
                if s.lines().count() >= n {
                    return s;
                }
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        read_all(dir)
    }

    #[test]
    fn records_actions_and_errors_without_names_by_default() {
        let dir = tempfile::tempdir().unwrap();
        let log = ActivityLog::start(dir.path().join("activity"), true, false);
        log.start_action("object_list")
            .name("bucket", "media-library")
            .name("prefix", "Audiobook G/")
            .field("objects", 3)
            .end::<()>(&Ok(()));
        let err = AppError::new(
            r2dock_core::ErrorCode::AccessDenied,
            "This key cannot do that.",
        )
        .with_detail("AccessDenied (403), request id abc");
        log.start_action("delete_execute").end::<()>(&Err(err));
        let text = wait_for_lines(&dir.path().join("activity"), 2);
        assert!(text.contains("\"action\":\"object_list\""));
        assert!(text.contains("\"outcome\":\"error\""));
        assert!(text.contains("request id abc"));
        assert!(
            !text.contains("media-library"),
            "names must be hashed by default"
        );
        assert!(!text.contains("Audiobook G"));
        assert!(text.contains("\"bucket\":\"#"));
    }

    #[test]
    fn names_when_allowed_and_secrets_never() {
        let dir = tempfile::tempdir().unwrap();
        let log = ActivityLog::start(dir.path().to_path_buf(), true, true);
        log.start_action("object_head")
            .name("key", "covers/cover.webp")
            .field("note", "secret=FAKESECRETVALUE123")
            .end::<()>(&Ok(()));
        let text = wait_for_lines(dir.path(), 1);
        assert!(text.contains("covers/cover.webp"));
        assert!(!text.contains("FAKESECRETVALUE123"));
    }

    #[test]
    fn disabled_log_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let log = ActivityLog::start(dir.path().join("a"), false, false);
        log.event("app_start", Map::new());
        std::thread::sleep(Duration::from_millis(100));
        assert!(!dir.path().join("a").exists());
    }

    #[test]
    fn same_name_same_hash() {
        let log = ActivityLog::start(std::env::temp_dir(), false, false);
        assert_eq!(log.name("a/b.txt"), log.name("a/b.txt"));
        assert_ne!(log.name("a/b.txt"), log.name("a/c.txt"));
        assert_eq!(log.name("a/b.txt").len(), 9);
    }
}
