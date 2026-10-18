//! On-disk journal of transfers so they come back after a restart
//! (PLAN.md 13).
//!
//! The engine keeps two of these: `journal.json` holds resumable state
//! (multipart uploads, partial downloads, compressed copies) and changes
//! often but stays small; the queue (`queue.json` plus `queue.log`) holds
//! every accepted job that has not finished and can hold a very large
//! number of jobs.
//!
//! Updates are batched and written by a background task, so the async
//! runtime never blocks on disk writes. A plain journal rewrites its whole
//! file atomically each time. A logged journal (the queue) appends each
//! change to a log beside a snapshot, and rewrites the snapshot only when
//! the log has grown about as large as it, so a queue of 100,000 jobs that
//! drains one job at a time writes a few bytes per job instead of the
//! whole queue every second. Each log line carries the snapshot's
//! generation; lines from before the last snapshot are ignored, and a line
//! cut short by a crash ends the log.

use std::collections::BTreeMap;
use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use parking_lot::Mutex;
use r2dock_core::storage::{Loaded, read_json, write_bytes_atomic};
use serde::{Deserialize, Serialize};
use tokio::sync::Notify;

use crate::ctx::ResumeState;
use crate::model::{GroupInfo, JobId, JobSpec};

pub const JOURNAL_VERSION: u32 = 1;

/// The log is folded into the snapshot once it is this big and at least as
/// big as the snapshot.
const LOG_COMPACT_MIN: u64 = 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JournalEntry {
    pub id: JobId,
    pub spec: JobSpec,
    pub group: Option<GroupInfo>,
    pub resume: ResumeState,
    pub created_at: String,
    /// Order the job was added in (0 in older journals).
    #[serde(default)]
    pub seq: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct JournalFile {
    version: u32,
    /// Log lines with another generation are older than this snapshot.
    generation: u64,
    entries: Vec<JournalEntry>,
}

impl Default for JournalFile {
    fn default() -> Self {
        Self {
            version: JOURNAL_VERSION,
            generation: 0,
            entries: Vec::new(),
        }
    }
}

/// One change in the log: an entry added or replaced, or (without an
/// entry) removed.
#[derive(Debug, Serialize, Deserialize)]
struct LogLine {
    generation: u64,
    id: JobId,
    #[serde(default)]
    entry: Option<JournalEntry>,
}

#[derive(Default)]
struct State {
    entries: BTreeMap<JobId, JournalEntry>,
    /// Changes not yet in the log (logged journals only).
    pending: Vec<(JobId, Option<JournalEntry>)>,
    generation: u64,
    log_bytes: u64,
    snapshot_bytes: u64,
    /// A snapshot failed to save; the next save writes one again.
    needs_snapshot: bool,
}

pub struct Journal {
    path: PathBuf,
    /// The append log beside the snapshot, for a logged journal.
    log: Option<PathBuf>,
    state: Mutex<State>,
    dirty: Notify,
    /// One write at a time, so an older snapshot never lands after a newer one.
    writing: Mutex<()>,
    /// Bytes written so far (snapshot and log), for measurements.
    written: AtomicU64,
}

impl Journal {
    /// Load a plain journal (a corrupt file is backed up and ignored).
    pub fn load(path: PathBuf) -> (Arc<Self>, Vec<JournalEntry>) {
        Self::open(path, None)
    }

    /// Load a logged journal: the snapshot at `path` plus the changes in
    /// `log`. Anything in the log is folded into a new snapshot at once,
    /// so later appends never follow a line cut short by a crash. Blocking.
    pub fn load_logged(path: PathBuf, log: PathBuf) -> (Arc<Self>, Vec<JournalEntry>) {
        Self::open(path, Some(log))
    }

    fn open(path: PathBuf, log: Option<PathBuf>) -> (Arc<Self>, Vec<JournalEntry>) {
        let file = match read_json::<JournalFile>(&path) {
            Ok(Loaded::Found(f)) => f,
            _ => JournalFile::default(),
        };
        let snapshot_bytes = std::fs::metadata(&path).map_or(0, |m| m.len());
        let mut state = State {
            generation: file.generation,
            snapshot_bytes,
            ..State::default()
        };
        state.entries = file.entries.into_iter().map(|e| (e.id, e)).collect();
        let replayed = log
            .as_ref()
            .and_then(|l| std::fs::metadata(l).ok())
            .is_some_and(|m| m.len() > 0);
        if let Some(log) = &log
            && let Ok(f) = std::fs::File::open(log)
        {
            for line in std::io::BufReader::new(f).lines() {
                let Ok(line) = line else { break };
                if line.trim().is_empty() {
                    continue;
                }
                // A line cut short by a crash ends the log.
                let Ok(change) = serde_json::from_str::<LogLine>(&line) else {
                    break;
                };
                if change.generation != state.generation {
                    continue;
                }
                match change.entry {
                    Some(e) => {
                        state.entries.insert(change.id, e);
                    }
                    None => {
                        state.entries.remove(&change.id);
                    }
                }
            }
        }
        let loaded: Vec<JournalEntry> = state.entries.values().cloned().collect();
        let journal = Arc::new(Self {
            path,
            log,
            state: Mutex::new(state),
            dirty: Notify::new(),
            writing: Mutex::new(()),
            written: AtomicU64::new(0),
        });
        if replayed {
            journal.compact();
        }
        (journal, loaded)
    }

    /// Start the background writer, which saves at most once per `batch`.
    /// Must be called inside a Tokio runtime.
    pub fn spawn_writer(self: &Arc<Self>, batch: Duration) {
        let this = Arc::clone(self);
        tokio::spawn(async move {
            loop {
                this.dirty.notified().await;
                // Batch bursts of updates (one per finished part).
                tokio::time::sleep(batch).await;
                let me = Arc::clone(&this);
                let _ = tokio::task::spawn_blocking(move || me.flush()).await;
            }
        });
    }

    pub fn upsert(&self, entry: JournalEntry) {
        self.upsert_many([entry]);
    }

    /// Add or replace many entries with one save.
    pub fn upsert_many(&self, entries: impl IntoIterator<Item = JournalEntry>) {
        let logged = self.log.is_some();
        let mut state = self.state.lock();
        let mut any = false;
        for e in entries {
            if logged {
                state.pending.push((e.id, Some(e.clone())));
            }
            state.entries.insert(e.id, e);
            any = true;
        }
        drop(state);
        if any {
            self.dirty.notify_one();
        }
    }

    pub fn remove(&self, id: JobId) {
        let mut state = self.state.lock();
        if state.entries.remove(&id).is_some() {
            if self.log.is_some() {
                state.pending.push((id, None));
            }
            drop(state);
            self.dirty.notify_one();
        }
    }

    /// Whether a job has a saved entry (tests only).
    #[cfg(test)]
    pub(crate) fn contains(&self, id: JobId) -> bool {
        self.state.lock().entries.contains_key(&id)
    }

    /// Bytes written to disk so far (snapshots and log).
    pub fn bytes_written(&self) -> u64 {
        self.written.load(Ordering::Relaxed)
    }

    /// Write the journal now. Blocking.
    pub fn flush(&self) {
        let Some(log) = &self.log else {
            self.compact();
            return;
        };
        let _writing = self.writing.lock();
        let (lines, generation) = {
            let mut state = self.state.lock();
            if state.needs_snapshot || state.log_bytes >= LOG_COMPACT_MIN.max(state.snapshot_bytes)
            {
                drop(state);
                drop(_writing);
                self.compact();
                return;
            }
            (std::mem::take(&mut state.pending), state.generation)
        };
        if lines.is_empty() {
            return;
        }
        let mut bytes = Vec::new();
        for (id, entry) in lines {
            let line = LogLine {
                generation,
                id,
                entry,
            };
            if serde_json::to_writer(&mut bytes, &line).is_ok() {
                bytes.push(b'\n');
            }
        }
        let appended = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(log)
            .and_then(|mut f| {
                f.write_all(&bytes)?;
                f.sync_data()
            });
        match appended {
            Ok(()) => {
                let n = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
                self.state.lock().log_bytes += n;
                self.written.fetch_add(n, Ordering::Relaxed);
            }
            Err(e) => {
                // Write everything as a new snapshot instead.
                tracing::warn!("could not append to the transfer queue log: {e}");
                self.compact();
            }
        }
    }

    /// Write a full snapshot and start an empty log. Blocking.
    fn compact(&self) {
        let _writing = self.writing.lock();
        let file = {
            let mut state = self.state.lock();
            state.pending.clear();
            JournalFile {
                version: JOURNAL_VERSION,
                generation: state.generation + u64::from(self.log.is_some()),
                entries: state.entries.values().cloned().collect(),
            }
        };
        let Ok(bytes) = serde_json::to_vec(&file) else {
            tracing::warn!("could not encode the transfer journal");
            return;
        };
        // On Windows an antivirus or indexer can briefly hold the file and
        // make the replace fail; try once more before giving up.
        let mut saved = write_bytes_atomic(&self.path, &bytes);
        if saved.is_err() {
            std::thread::sleep(Duration::from_millis(50));
            saved = write_bytes_atomic(&self.path, &bytes);
        }
        if let Err(e) = saved {
            tracing::warn!("could not save the transfer journal: {e}");
            // Keep the log; its lines still match the old snapshot. The
            // dropped changes are in the entries: save a snapshot next time.
            self.state.lock().needs_snapshot = true;
            self.dirty.notify_one();
            return;
        }
        let n = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        self.written.fetch_add(n, Ordering::Relaxed);
        {
            let mut state = self.state.lock();
            state.generation = file.generation;
            state.snapshot_bytes = n;
            state.log_bytes = 0;
            state.needs_snapshot = false;
        }
        // The new snapshot holds everything; old log lines carry the old
        // generation and would be skipped anyway.
        if let Some(log) = &self.log {
            let _ = std::fs::OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(log);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ctx::DownloadState;
    use crate::model::DownloadSpec;
    use r2dock_core::policy::ConflictPolicy;

    fn entry() -> JournalEntry {
        JournalEntry {
            id: uuid::Uuid::new_v4(),
            spec: JobSpec::Download(DownloadSpec {
                connection: uuid::Uuid::nil(),
                bucket: "bkt".into(),
                key: "k".into(),
                size: 10,
                dest_root: PathBuf::from("/tmp"),
                rel_path: PathBuf::from("k"),
                policy: ConflictPolicy::Ask,
            }),
            group: None,
            resume: ResumeState::Download(DownloadState {
                etag: Some("\"abc\"".into()),
                temp: PathBuf::from("/tmp/.k.r2dock-part"),
                final_path: PathBuf::from("/tmp/k"),
                finalized: None,
            }),
            created_at: "2026-10-17T00:00:00Z".into(),
            seq: 0,
        }
    }

    fn logged(dir: &std::path::Path) -> (Arc<Journal>, Vec<JournalEntry>) {
        Journal::load_logged(dir.join("queue.json"), dir.join("queue.log"))
    }

    #[test]
    fn logged_journal_appends_changes_and_reloads_them() {
        let dir = tempfile::tempdir().unwrap();
        let (j, loaded) = logged(dir.path());
        assert!(loaded.is_empty());
        let entries: Vec<JournalEntry> = (0..100).map(|_| entry()).collect();
        j.upsert_many(entries.clone());
        j.flush();
        let after_add = j.bytes_written();
        for e in &entries[..50] {
            j.remove(e.id);
        }
        j.flush();
        // Removing 50 entries appends 50 short lines, not the whole queue.
        let per_remove = (j.bytes_written() - after_add) / 50;
        assert!(per_remove < 100, "{per_remove} bytes per removal");
        assert!(!dir.path().join("queue.json").exists(), "no snapshot yet");
        let (j2, mut loaded) = logged(dir.path());
        loaded.sort_by_key(|e| e.id);
        let mut want: Vec<JournalEntry> = entries[50..].to_vec();
        want.sort_by_key(|e| e.id);
        assert_eq!(loaded, want);
        // Loading folded the log into a snapshot.
        assert!(dir.path().join("queue.json").exists());
        assert_eq!(
            std::fs::metadata(dir.path().join("queue.log"))
                .unwrap()
                .len(),
            0
        );
        drop(j2);
        let (_, again) = logged(dir.path());
        assert_eq!(again.len(), 50);
    }

    #[test]
    fn logged_journal_compacts_and_skips_older_lines() {
        let dir = tempfile::tempdir().unwrap();
        let (j, _) = logged(dir.path());
        let keep = entry();
        j.upsert(keep.clone());
        // Enough churn to pass the compaction size several times.
        for _ in 0..40 {
            let batch: Vec<JournalEntry> = (0..100).map(|_| entry()).collect();
            j.upsert_many(batch.clone());
            j.flush();
            for e in &batch {
                j.remove(e.id);
            }
            j.flush();
        }
        let log_len = std::fs::metadata(dir.path().join("queue.log"))
            .unwrap()
            .len();
        assert!(
            log_len <= 2 * LOG_COMPACT_MIN,
            "the log stays bounded: {log_len}"
        );
        // A stale line from an older generation (a crash between writing a
        // snapshot and emptying the log) is ignored.
        let stale = LogLine {
            generation: 0,
            id: keep.id,
            entry: None,
        };
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(dir.path().join("queue.log"))
            .unwrap();
        writeln!(f, "{}", serde_json::to_string(&stale).unwrap()).unwrap();
        // A line cut short by a crash ends the log.
        write!(f, "{{\"generation\":").unwrap();
        drop(f);
        drop(j);
        let (_, loaded) = logged(dir.path());
        assert_eq!(loaded, vec![keep]);
    }

    #[test]
    fn old_pretty_queue_files_still_load() {
        let dir = tempfile::tempdir().unwrap();
        let e = entry();
        let old = serde_json::json!({ "version": 1, "entries": [e] });
        std::fs::write(
            dir.path().join("queue.json"),
            serde_json::to_vec_pretty(&old).unwrap(),
        )
        .unwrap();
        let (_, loaded) = logged(dir.path());
        assert_eq!(loaded, vec![e]);
    }

    #[test]
    fn persists_and_reloads() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("journal.json");
        let (j, loaded) = Journal::load(path.clone());
        assert!(loaded.is_empty());
        let e = entry();
        j.upsert(e.clone());
        j.flush();
        let (_, loaded) = Journal::load(path.clone());
        assert_eq!(loaded, vec![e.clone()]);
        j.remove(e.id);
        j.flush();
        let (_, loaded) = Journal::load(path);
        assert!(loaded.is_empty());
    }

    /// Regression: `u128` fields cannot be read back inside internally
    /// tagged enums, which made every journal with a multipart upload fail
    /// to load.
    #[test]
    fn multipart_and_compressed_entries_reload() {
        use crate::ctx::MultipartState;
        use crate::model::{CompressedCopy, CompressionResult, UploadSpec};
        let fingerprint = crate::source::Fingerprint {
            len: 1,
            mtime_ns: 1_792_272_545_142_633_478,
        };
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("journal.json");
        let (j, _) = Journal::load(path.clone());
        let e = JournalEntry {
            id: uuid::Uuid::new_v4(),
            spec: JobSpec::Upload(UploadSpec {
                connection: uuid::Uuid::nil(),
                bucket: "bkt".into(),
                key: "k.mp3".into(),
                local: PathBuf::from("/tmp/copy.mp3"),
                size: 10,
                policy: ConflictPolicy::Ask,
                identity: None,
                compression: None,
                compressed: Some(CompressionResult::Compressed(CompressedCopy {
                    temp: PathBuf::from("/tmp/copy.mp3"),
                    original_local: PathBuf::from("/tmp/k.mp3"),
                    original_key: "k.mp3".into(),
                    original_size: 20,
                    original: fingerprint,
                    preset: "mp3-32k".into(),
                    output: Some(crate::model::CopyDigest {
                        file: crate::source::WrittenFile {
                            len: 10,
                            mtime_ns: 1_792_272_545_142_633_478,
                            identity: Some(crate::source::FileIdentity {
                                dev: u64::MAX,
                                ino: 7,
                            }),
                        },
                        sha256: "ab".repeat(32),
                    }),
                    original_identity: Some(crate::source::FileIdentity { dev: 3, ino: 9 }),
                })),
            }),
            group: None,
            resume: ResumeState::Multipart(MultipartState {
                upload_id: "u".into(),
                key: "k.mp3".into(),
                part_size: 8,
                fingerprint,
                done: [(
                    1,
                    crate::ctx::PartDone {
                        etag: "\"e1\"".into(),
                        md5: "00".into(),
                        len: 8,
                    },
                )]
                .into(),
            }),
            created_at: "2026-10-17T00:00:00Z".into(),
            seq: 0,
        };
        j.upsert(e.clone());
        j.flush();
        let (_, loaded) = Journal::load(path);
        assert_eq!(loaded, vec![e]);
    }

    #[test]
    fn corrupt_journal_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("journal.json");
        std::fs::write(&path, b"garbage").unwrap();
        let (_, loaded) = Journal::load(path);
        assert!(loaded.is_empty());
    }
}
