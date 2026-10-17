//! Transfer history as append-only JSON lines, trimmed by age and count.

use std::fs::OpenOptions;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;

use chrono::{DateTime, Duration, Utc};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use crate::model::{Direction, JobState};

pub const MAX_ENTRIES: usize = 1000;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct HistoryEntry {
    pub id: Uuid,
    pub direction: Direction,
    pub connection_id: Uuid,
    pub bucket: String,
    pub key: String,
    pub name: String,
    pub local_display: String,
    #[ts(type = "number")]
    pub bytes: u64,
    pub state: JobState,
    pub message: Option<String>,
    pub finished_at: String,
    pub duration_secs: f64,
}

pub struct History {
    path: PathBuf,
    lock: Mutex<()>,
}

impl History {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            lock: Mutex::new(()),
        }
    }

    /// Append one entry. Blocking.
    pub fn append(&self, entry: &HistoryEntry) {
        let _guard = self.lock.lock();
        let Ok(line) = serde_json::to_string(entry) else {
            return;
        };
        if let Some(dir) = self.path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let result = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .and_then(|mut f| writeln!(f, "{line}"));
        if let Err(e) = result {
            tracing::warn!("could not save transfer history: {:?}", e.kind());
        }
    }

    /// Read entries, newest first. Malformed lines are skipped. Blocking.
    pub fn read(&self, limit: usize) -> Vec<HistoryEntry> {
        let _guard = self.lock.lock();
        let Ok(file) = std::fs::File::open(&self.path) else {
            return Vec::new();
        };
        let mut entries: Vec<HistoryEntry> = BufReader::new(file)
            .lines()
            .map_while(Result::ok)
            .filter_map(|l| serde_json::from_str(&l).ok())
            .collect();
        entries.reverse();
        entries.truncate(limit);
        entries
    }

    /// Drop entries older than `days` and keep at most `MAX_ENTRIES`. Blocking.
    pub fn trim(&self, days: u32, now: DateTime<Utc>) {
        let mut entries = self.read(usize::MAX);
        let cutoff = now - Duration::days(i64::from(days));
        entries.retain(|e| {
            DateTime::parse_from_rfc3339(&e.finished_at)
                .is_ok_and(|t| t.with_timezone(&Utc) >= cutoff)
        });
        entries.truncate(MAX_ENTRIES);
        entries.reverse();
        let _guard = self.lock.lock();
        let mut out = String::new();
        for e in &entries {
            if let Ok(l) = serde_json::to_string(e) {
                out.push_str(&l);
                out.push('\n');
            }
        }
        let Some(dir) = self.path.parent() else {
            return;
        };
        let write = tempfile::NamedTempFile::new_in(dir).and_then(|mut t| {
            t.write_all(out.as_bytes())?;
            t.persist(&self.path).map_err(|e| e.error)?;
            Ok(())
        });
        if let Err(e) = write {
            tracing::warn!("could not trim transfer history: {:?}", e.kind());
        }
    }

    /// Remove all history. Blocking.
    pub fn clear(&self) {
        let _guard = self.lock.lock();
        let _ = std::fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, at: DateTime<Utc>) -> HistoryEntry {
        HistoryEntry {
            id: Uuid::new_v4(),
            direction: Direction::Upload,
            connection_id: Uuid::nil(),
            bucket: "bkt".into(),
            key: name.into(),
            name: name.into(),
            local_display: "~/x".into(),
            bytes: 1,
            state: JobState::Completed,
            message: None,
            finished_at: at.to_rfc3339(),
            duration_secs: 1.0,
        }
    }

    #[test]
    fn append_read_trim() {
        let dir = tempfile::tempdir().unwrap();
        let h = History::new(dir.path().join("history.jsonl"));
        let now = Utc::now();
        h.append(&entry("old", now - Duration::days(40)));
        h.append(&entry("new", now));
        std::fs::OpenOptions::new()
            .append(true)
            .open(dir.path().join("history.jsonl"))
            .unwrap()
            .write_all(b"not json\n")
            .unwrap();
        let all = h.read(10);
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].name, "new");
        h.trim(30, now);
        let all = h.read(10);
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].name, "new");
        h.clear();
        assert!(h.read(10).is_empty());
    }
}
