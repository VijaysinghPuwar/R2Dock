//! Activity log: one CSV row per user action and its result, for
//! troubleshooting. Files are `activity-YYYY-MM-DD.csv` (UTC date) in a
//! folder the user can change in Settings. While the day's file is locked
//! (for example open in Excel), rows go to `activity-YYYY-MM-DD-<session>.csv`.
//! Version 0.1.x wrote
//! `activity-YYYY-MM-DD.jsonl`; those files are still trimmed and exported.
//!
//! Privacy rules:
//! - Never records secrets, request arguments, signatures, or clipboard text.
//! - Object keys, bucket names, and local file names are written as a short
//!   hash (`#1a2b3c4d`) unless the user turns on "Include file and folder
//!   names". The same name always gives the same hash, so events can still be
//!   matched up.
//! - Every cell also passes through the redaction filter (see [`csv`]).
//! - Writes happen on a background thread, so commands never wait on disk.

pub mod csv;
pub mod export;

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
use serde_json::{Map, Value};

use self::csv::{Record, RecordError};

/// File name prefix shared by CSV and old JSONL activity files.
pub const FILE_PREFIX: &str = "activity-";

pub struct ActivityLog {
    dir: Arc<RwLock<PathBuf>>,
    enabled: AtomicBool,
    names: AtomicBool,
    session: String,
    /// Sends `(UTC date, CSV line)` to the writer thread.
    tx: Option<Sender<(String, String)>>,
}

impl ActivityLog {
    /// Start the log and its writer thread.
    pub fn start(dir: PathBuf, enabled: bool, names: bool) -> Arc<Self> {
        let dir = Arc::new(RwLock::new(dir));
        let (tx, rx) = channel::<(String, String)>();
        let writer_dir = dir.clone();
        let session = uuid::Uuid::new_v4().simple().to_string()[..8].to_string();
        let writer_session = session.clone();
        let spawned = std::thread::Builder::new()
            .name("r2dock-activity".into())
            .spawn(move || {
                while let Ok((date, line)) = rx.recv() {
                    let dir = writer_dir.read().clone();
                    write_line(&dir, &date, &line, &writer_session);
                }
            })
            .is_ok();
        Arc::new(Self {
            dir,
            enabled: AtomicBool::new(enabled),
            names: AtomicBool::new(names),
            session,
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
            r2dock_core::keys::display_name(value)
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

    /// Record a compression milestone from the transfer engine.
    pub fn compression(&self, event: &r2dock_transfer::engine::CompressionEvent) {
        use r2dock_transfer::engine::CompressionEvent as E;
        let mut f = Map::new();
        let job = |id: &uuid::Uuid| Value::String(id.simple().to_string()[..8].to_string());
        let (action, outcome, duration, error) = match event {
            E::Started { id, key } => {
                f.insert("job".into(), job(id));
                f.insert("key".into(), self.name(key).into());
                ("compress_started", "info", None, None)
            }
            E::Finished {
                id,
                key,
                original_bytes,
                bytes,
                preset,
                secs,
            } => {
                f.insert("job".into(), job(id));
                f.insert("key".into(), self.name(key).into());
                f.insert("original_bytes".into(), (*original_bytes).into());
                f.insert("bytes".into(), (*bytes).into());
                f.insert("preset".into(), preset.clone().into());
                let ratio = if *original_bytes == 0 {
                    0.0
                } else {
                    *bytes as f64 / *original_bytes as f64
                };
                f.insert("ratio".into(), format!("{ratio:.3}").into());
                (
                    "compress_finished",
                    "ok",
                    Some(Duration::from_secs_f64(secs.max(0.0))),
                    None,
                )
            }
            E::Unchanged { id, key, reason } => {
                f.insert("job".into(), job(id));
                f.insert("key".into(), self.name(key).into());
                f.insert(
                    "reason".into(),
                    serde_json::to_value(reason).unwrap_or(Value::Null),
                );
                ("compress_skipped", "info", None, None)
            }
            E::Failed { id, key, message } => {
                f.insert("job".into(), job(id));
                f.insert("key".into(), self.name(key).into());
                let e = AppError::new(r2dock_core::ErrorCode::Internal, message.clone());
                ("compress_failed", "error", None, Some(e))
            }
        };
        self.emit(action, outcome, duration, f, error.as_ref());
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
        let now = Utc::now();
        let record = Record {
            time: now,
            session: self.session.clone(),
            action: action.to_string(),
            outcome: outcome.to_string(),
            duration_ms: duration.map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX)),
            fields,
            error: error.map(|e| RecordError {
                code: serde_json::to_value(e.code)
                    .ok()
                    .and_then(|v| v.as_str().map(str::to_string))
                    .unwrap_or_default(),
                message: e.message.clone(),
                detail: e.detail.clone(),
            }),
        };
        let date = now.format("%Y-%m-%d").to_string();
        let _ = tx.send((date, record.to_line()));
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
            if export::file_date(&name).is_none() {
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

/// Append a line to the day's file. When that file cannot be opened (on
/// Windows, Excel keeps an open CSV locked), the line goes to
/// `activity-YYYY-MM-DD-<session>.csv` instead, which export and trim treat
/// as part of the same day.
fn write_line(dir: &Path, date: &str, line: &str, session: &str) {
    if std::fs::create_dir_all(dir).is_err() {
        return;
    }
    let daily = dir.join(format!("{FILE_PREFIX}{date}.csv"));
    let fallback = dir.join(format!("{FILE_PREFIX}{date}-{session}.csv"));
    for file in [daily, fallback] {
        if append_line(&file, line).is_ok() {
            return;
        }
    }
}

fn append_line(file: &Path, line: &str) -> std::io::Result<()> {
    let mut f = OpenOptions::new().create(true).append(true).open(file)?;
    // A new (or emptied) file starts with the byte order mark and header.
    if f.metadata()?.len() == 0 {
        writeln!(f, "{}{}", csv::BOM, csv::header_line())?;
    }
    writeln!(f, "{line}")
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
        // Header plus two rows.
        let text = wait_for_lines(&dir.path().join("activity"), 3);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], format!("{}{}", csv::BOM, csv::header_line()));
        assert_eq!(lines.len(), 3, "header is written once: {text}");
        assert!(lines[1].contains(",object_list,ok,"));
        assert!(lines[1].contains(",#"), "hashed bucket column");
        assert!(lines[1].contains("objects=3"));
        assert!(lines[2].contains(",delete_execute,error,"));
        assert!(lines[2].contains(",accessDenied,This key cannot do that.,abc,"));
        assert!(
            !text.contains("media-library"),
            "names must be hashed by default"
        );
        assert!(!text.contains("Audiobook G"));
        let file = std::fs::read_dir(dir.path().join("activity"))
            .unwrap()
            .flatten()
            .next()
            .unwrap()
            .file_name()
            .to_string_lossy()
            .into_owned();
        assert!(
            file.starts_with("activity-") && file.ends_with(".csv"),
            "{file}"
        );
    }

    #[test]
    fn names_when_allowed_and_secrets_never() {
        let dir = tempfile::tempdir().unwrap();
        let log = ActivityLog::start(dir.path().to_path_buf(), true, true);
        log.start_action("object_head")
            .name("key", "covers/cover.webp")
            .field("note", "secret=FAKESECRETVALUE123")
            .name("bucket", "media-library")
            .end::<()>(&Ok(()));
        let text = wait_for_lines(dir.path(), 2);
        assert!(text.contains(",media-library,covers/cover.webp,"));
        assert!(!text.contains("FAKESECRETVALUE123"));
    }

    #[test]
    fn compression_events_become_rows() {
        use r2dock_transfer::engine::CompressionEvent;
        let dir = tempfile::tempdir().unwrap();
        let log = ActivityLog::start(dir.path().to_path_buf(), true, false);
        let id = uuid::Uuid::new_v4();
        log.compression(&CompressionEvent::Finished {
            id,
            key: "Audiobooks/Secret Title.mp3".into(),
            original_bytes: 660_000_000,
            bytes: 330_000_000,
            preset: "mp3-32k".into(),
            secs: 172.0,
        });
        log.compression(&CompressionEvent::Unchanged {
            id,
            key: "photo.png".into(),
            reason: r2dock_media::pipeline::Unchanged::NotAudio,
        });
        let text = wait_for_lines(dir.path(), 3);
        let rows: Vec<&str> = text.lines().skip(1).collect();
        assert!(
            rows[0].contains(",compress_finished,ok,172000,"),
            "{}",
            rows[0]
        );
        assert!(rows[0].contains("preset=mp3-32k"));
        assert!(rows[0].contains("ratio=0.500"));
        assert!(rows[0].contains(",#"), "key is hashed");
        assert!(!text.contains("Secret Title"));
        assert!(rows[1].contains(",compress_skipped,info,"));
        assert!(rows[1].contains("notAudio"));
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
    fn locked_daily_file_falls_back_to_a_session_file() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        // A folder with the daily name cannot be opened for append, like a
        // file Excel holds open.
        std::fs::create_dir(d.join("activity-2026-10-18.csv")).unwrap();
        write_line(d, "2026-10-18", "row-1", "ab12cd34");
        write_line(d, "2026-10-18", "row-2", "ab12cd34");
        let text = std::fs::read_to_string(d.join("activity-2026-10-18-ab12cd34.csv")).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(
            lines,
            [
                format!("{}{}", csv::BOM, csv::header_line()).as_str(),
                "row-1",
                "row-2"
            ]
        );
        assert!(export::file_date("activity-2026-10-18-ab12cd34.csv").is_some());

        // Once the daily file is free again, lines go back to it.
        std::fs::remove_dir(d.join("activity-2026-10-18.csv")).unwrap();
        write_line(d, "2026-10-18", "row-3", "ab12cd34");
        let daily = std::fs::read_to_string(d.join("activity-2026-10-18.csv")).unwrap();
        assert!(daily.ends_with(
            "row-3
"
        ));
    }

    #[test]
    fn same_name_same_hash() {
        let log = ActivityLog::start(std::env::temp_dir(), false, false);
        assert_eq!(log.name("a/b.txt"), log.name("a/b.txt"));
        assert_ne!(log.name("a/b.txt"), log.name("a/c.txt"));
        assert_eq!(log.name("a/b.txt").len(), 9);
    }
}
