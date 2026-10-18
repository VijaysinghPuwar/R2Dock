//! The activity log's background writer.
//!
//! - Rows go through a bounded queue (by count and by bytes). When it is
//!   full the row is dropped and counted; callers never wait on disk, so a
//!   slow or stuck disk cannot hold up a transfer.
//! - Rows are written in batches: each file is opened once per batch and
//!   closed again, so another program (Excel on Windows) can still lock it.
//! - Write failures and dropped rows are counted and kept as a short
//!   problem description that Settings shows. Dropped rows are also noted
//!   in the log itself as an `activity_rows_dropped` row once writing works.
//! - [`Writer::flush`] waits, with a time limit, until every row queued
//!   before it is on disk. Export and app exit use it.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender, SyncSender, TrySendError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use chrono::Utc;
use parking_lot::{Mutex, RwLock};
use serde_json::{Map, Value};

use super::FILE_PREFIX;
use super::csv::{self, Record};

/// Most rows waiting to be written.
pub const MAX_QUEUED_ROWS: usize = 20_000;
/// Most bytes of rows waiting to be written.
pub const MAX_QUEUED_BYTES: usize = 16 * 1024 * 1024;
/// Most rows written per batch.
const BATCH: usize = 1_000;

enum Msg {
    Row { date: String, line: String },
    Flush(Sender<()>),
    Stop,
}

#[derive(Default)]
struct Health {
    /// Rows dropped because the queue was full or the writer had stopped.
    dropped: AtomicU64,
    /// Dropped rows already noted in the log.
    noted: AtomicU64,
    /// Rows that could not be written to disk.
    failed: AtomicU64,
    /// True while the latest write failed.
    failing: AtomicBool,
    /// What went wrong last, in plain words (never a path).
    last_error: Mutex<Option<String>>,
}

pub struct Writer {
    tx: Option<SyncSender<Msg>>,
    health: Arc<Health>,
    queued_bytes: Arc<AtomicUsize>,
    max_bytes: usize,
    handle: Mutex<Option<JoinHandle<()>>>,
}

impl Writer {
    pub fn start(dir: Arc<RwLock<PathBuf>>, session: String) -> Self {
        Self::with_limits(dir, session, MAX_QUEUED_ROWS, MAX_QUEUED_BYTES)
    }

    pub fn with_limits(
        dir: Arc<RwLock<PathBuf>>,
        session: String,
        max_rows: usize,
        max_bytes: usize,
    ) -> Self {
        let (tx, rx) = std::sync::mpsc::sync_channel::<Msg>(max_rows);
        let health = Arc::new(Health::default());
        let queued_bytes = Arc::new(AtomicUsize::new(0));
        let spawned = {
            let health = health.clone();
            let queued = queued_bytes.clone();
            std::thread::Builder::new()
                .name("r2dock-activity".into())
                .spawn(move || run(&rx, &dir, &session, &health, &queued))
        };
        let (tx, handle) = match spawned {
            Ok(h) => (Some(tx), Some(h)),
            Err(_) => {
                *health.last_error.lock() = Some("the writer could not start".into());
                health.failing.store(true, Ordering::Relaxed);
                (None, None)
            }
        };
        Self {
            tx,
            health,
            queued_bytes,
            max_bytes,
            handle: Mutex::new(handle),
        }
    }

    /// Queue a row. Never blocks: a row that does not fit is dropped and
    /// counted.
    pub fn send(&self, date: String, line: String) {
        let len = line.len();
        let fits = self.queued_bytes.fetch_add(len, Ordering::AcqRel) + len <= self.max_bytes;
        let sent = fits
            && self
                .tx
                .as_ref()
                .is_some_and(|tx| tx.try_send(Msg::Row { date, line }).is_ok());
        if !sent {
            self.queued_bytes.fetch_sub(len, Ordering::AcqRel);
            self.health.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Wait until every row queued before this call is written (or failed).
    /// Returns false when that does not happen within `timeout`.
    pub fn flush(&self, timeout: Duration) -> bool {
        let Some(tx) = &self.tx else {
            return false;
        };
        let deadline = Instant::now() + timeout;
        let (ack, done) = std::sync::mpsc::channel();
        let mut msg = Msg::Flush(ack);
        loop {
            match tx.try_send(msg) {
                Ok(()) => break,
                Err(TrySendError::Full(m)) if Instant::now() < deadline => {
                    msg = m;
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(_) => return false,
            }
        }
        done.recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .is_ok()
    }

    /// Write what is queued (within `timeout`) and stop the writer thread.
    /// Rows sent afterwards are counted as dropped.
    pub fn shutdown(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        let flushed = self.flush(timeout);
        if let Some(tx) = &self.tx {
            let _ = tx.try_send(Msg::Stop);
        }
        if let Some(handle) = self.handle.lock().take() {
            while !handle.is_finished() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(5));
            }
            if handle.is_finished() {
                let _ = handle.join();
            }
        }
        flushed
    }

    pub fn dropped(&self) -> u64 {
        self.health.dropped.load(Ordering::Relaxed)
    }

    pub fn failed(&self) -> u64 {
        self.health.failed.load(Ordering::Relaxed)
    }

    /// A plain description of logging problems in this session, if any.
    pub fn problem(&self) -> Option<String> {
        let failed = self.failed();
        let dropped = self.dropped();
        let failing = self.health.failing.load(Ordering::Relaxed);
        let last = self.health.last_error.lock().clone();
        if !failing && failed == 0 && dropped == 0 {
            return None;
        }
        let mut parts = Vec::new();
        if failing {
            parts.push(format!(
                "The activity log cannot be written right now ({}).",
                last.as_deref().unwrap_or("unknown error")
            ));
        }
        if failed > 0 {
            parts.push(format!(
                "{failed} {} could not be saved this session.",
                rows(failed)
            ));
        }
        if dropped > 0 {
            parts.push(format!(
                "{dropped} {} skipped because the log could not keep up.",
                if dropped == 1 { "row was" } else { "rows were" }
            ));
        }
        Some(parts.join(" "))
    }
}

fn rows(n: u64) -> &'static str {
    if n == 1 { "row" } else { "rows" }
}

/// The writer thread: take everything queued, write it, acknowledge flushes.
fn run(
    rx: &Receiver<Msg>,
    dir: &RwLock<PathBuf>,
    session: &str,
    health: &Health,
    queued: &AtomicUsize,
) {
    while let Ok(first) = rx.recv() {
        let mut batch: Vec<(String, String)> = Vec::new();
        let mut acks = Vec::new();
        let mut stop = false;
        let mut next = Some(first);
        while let Some(msg) = next.take() {
            match msg {
                Msg::Row { date, line } => {
                    queued.fetch_sub(line.len(), Ordering::AcqRel);
                    batch.push((date, line));
                }
                Msg::Flush(ack) => acks.push(ack),
                Msg::Stop => stop = true,
            }
            if stop || batch.len() >= BATCH {
                break;
            }
            next = rx.try_recv().ok();
        }
        note_dropped(health, session, &mut batch);
        let dir = dir.read().clone();
        write_batch(&dir, session, &batch, health);
        for ack in acks {
            let _ = ack.send(());
        }
        if stop {
            break;
        }
    }
}

/// Add a row that says how many rows were dropped since the last note.
fn note_dropped(health: &Health, session: &str, batch: &mut Vec<(String, String)>) {
    let dropped = health.dropped.load(Ordering::Relaxed);
    let noted = health.noted.swap(dropped, Ordering::Relaxed);
    if dropped <= noted {
        return;
    }
    let now = Utc::now();
    let mut fields = Map::new();
    fields.insert("dropped".into(), Value::from(dropped - noted));
    let record = Record {
        time: now,
        session: session.to_string(),
        action: "activity_rows_dropped".into(),
        outcome: "error".into(),
        duration_ms: None,
        fields,
        error: None,
    };
    batch.push((now.format("%Y-%m-%d").to_string(), record.to_line()));
}

/// Write a batch, one file open per day, and update the health counters.
fn write_batch(dir: &Path, session: &str, batch: &[(String, String)], health: &Health) {
    let mut start = 0;
    while start < batch.len() {
        let date = &batch[start].0;
        let end = batch[start..]
            .iter()
            .position(|(d, _)| d != date)
            .map_or(batch.len(), |p| start + p);
        let lines: Vec<&str> = batch[start..end].iter().map(|(_, l)| l.as_str()).collect();
        match write_lines(dir, date, &lines, session) {
            Ok(()) => health.failing.store(false, Ordering::Relaxed),
            Err(e) => {
                health
                    .failed
                    .fetch_add(lines.len() as u64, Ordering::Relaxed);
                let text = describe(&e);
                if !health.failing.swap(true, Ordering::Relaxed) {
                    // Once per failure spell, never with a path.
                    tracing::warn!(error = %text, "activity log rows could not be written");
                }
                *health.last_error.lock() = Some(text);
            }
        }
        start = end;
    }
}

fn describe(e: &std::io::Error) -> String {
    use std::io::ErrorKind as K;
    match e.kind() {
        K::PermissionDenied => "permission denied".into(),
        K::NotFound => "the folder does not exist".into(),
        K::StorageFull => "the disk is full".into(),
        K::NotADirectory => "the folder path is not a folder".into(),
        K::ReadOnlyFilesystem => "the disk is read-only".into(),
        _ => "the file could not be written".into(),
    }
}

/// Append lines to the day's file. When that file cannot be opened (on
/// Windows, Excel keeps an open CSV locked), they go to
/// `activity-YYYY-MM-DD-<session>.csv` instead, which export and trim treat
/// as part of the same day.
pub(super) fn write_lines(
    dir: &Path,
    date: &str,
    lines: &[&str],
    session: &str,
) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let daily = dir.join(format!("{FILE_PREFIX}{date}.csv"));
    let fallback = dir.join(format!("{FILE_PREFIX}{date}-{session}.csv"));
    append_lines(&daily, lines).or_else(|_| append_lines(&fallback, lines))
}

fn append_lines(file: &Path, lines: &[&str]) -> std::io::Result<()> {
    let f = OpenOptions::new().create(true).append(true).open(file)?;
    let empty = f.metadata()?.len() == 0;
    let mut w = std::io::BufWriter::new(f);
    // A new (or emptied) file starts with the byte order mark and header.
    if empty {
        writeln!(w, "{}{}", csv::BOM, csv::header_line())?;
    }
    for line in lines {
        writeln!(w, "{line}")?;
    }
    w.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows_in(dir: &Path) -> Vec<String> {
        let mut out = Vec::new();
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            let text = std::fs::read_to_string(e.path()).unwrap();
            out.extend(
                text.lines()
                    .filter(|l| !l.ends_with(&csv::header_line()))
                    .map(str::to_string),
            );
        }
        out
    }

    /// Stall the writer: hold the folder setting and let the writer pick up
    /// one primer row, so it is blocked with an empty queue.
    fn stalled<'a>(
        w: &Writer,
        dir: &'a RwLock<PathBuf>,
    ) -> parking_lot::RwLockWriteGuard<'a, PathBuf> {
        let guard = dir.write();
        w.send("2026-10-18".into(), "primer".into());
        let deadline = Instant::now() + Duration::from_secs(5);
        while w.queued_bytes.load(Ordering::Acquire) > 0 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        // The writer has taken the primer row and is now waiting on the guard.
        std::thread::sleep(Duration::from_millis(20));
        guard
    }

    #[test]
    fn a_stuck_writer_drops_rows_without_blocking_and_reports_them() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = Arc::new(RwLock::new(tmp.path().to_path_buf()));
        let w = Writer::with_limits(dir.clone(), "s1".into(), 10, 1 << 20);
        // Holding the folder setting stalls the writer, like a stuck disk.
        let stall = stalled(&w, &dir);
        let started = Instant::now();
        for i in 0..50 {
            w.send("2026-10-18".into(), format!("row-{i}"));
        }
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "send never waits"
        );
        assert!(!w.flush(Duration::from_millis(100)), "flush is bounded");
        drop(stall);
        assert!(w.flush(Duration::from_secs(5)));
        let rows = rows_in(tmp.path());
        let written = rows.iter().filter(|r| r.starts_with("row-")).count() as u64;
        // Exactly the queue's capacity was kept.
        assert_eq!(written, 10);
        assert_eq!(w.dropped(), 40);
        let note = rows
            .iter()
            .find(|r| r.contains(",activity_rows_dropped,error,"))
            .expect("the log notes dropped rows");
        assert!(note.contains(&format!("dropped={}", w.dropped())), "{note}");
        let problem = w.problem().unwrap();
        assert!(problem.contains("could not keep up"), "{problem}");
    }

    #[test]
    fn the_byte_budget_also_bounds_the_queue() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = Arc::new(RwLock::new(tmp.path().to_path_buf()));
        let w = Writer::with_limits(dir.clone(), "s1".into(), 1_000, 100);
        let stall = stalled(&w, &dir);
        for _ in 0..10 {
            w.send("2026-10-18".into(), "x".repeat(40));
        }
        drop(stall);
        assert!(w.flush(Duration::from_secs(5)));
        // Two rows of 40 bytes fit in 100 bytes.
        assert_eq!(w.dropped(), 8);
    }

    #[test]
    fn write_failures_are_counted_and_described() {
        let tmp = tempfile::tempdir().unwrap();
        // A folder path whose parent is a regular file cannot be created.
        let blocker = tmp.path().join("file");
        std::fs::write(&blocker, b"x").unwrap();
        let dir = Arc::new(RwLock::new(blocker.join("activity")));
        let w = Writer::start(dir.clone(), "s1".into());
        w.send("2026-10-18".into(), "a".into());
        w.send("2026-10-18".into(), "b".into());
        assert!(w.flush(Duration::from_secs(5)));
        assert_eq!(w.failed(), 2);
        let problem = w.problem().unwrap();
        assert!(problem.contains("cannot be written right now"), "{problem}");
        assert!(problem.contains("2 rows could not be saved"), "{problem}");
        assert!(!problem.contains(&tmp.path().to_string_lossy().to_string()));

        // Writing works again once the folder is usable.
        *dir.write() = tmp.path().join("ok");
        w.send("2026-10-18".into(), "c".into());
        assert!(w.flush(Duration::from_secs(5)));
        assert_eq!(rows_in(&tmp.path().join("ok")), vec!["c".to_string()]);
        let problem = w.problem().unwrap();
        assert!(!problem.contains("right now"), "{problem}");
        assert!(problem.contains("2 rows could not be saved"));
    }

    #[test]
    fn shutdown_drains_then_stops() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = Arc::new(RwLock::new(tmp.path().to_path_buf()));
        let w = Writer::start(dir, "s1".into());
        for i in 0..500 {
            w.send("2026-10-18".into(), format!("row-{i}"));
        }
        assert!(w.shutdown(Duration::from_secs(5)));
        assert_eq!(rows_in(tmp.path()).len(), 500);
        assert!(w.handle.lock().is_none(), "thread joined");
        // Rows after exit are counted, not lost silently, and never panic.
        w.send("2026-10-18".into(), "late".into());
        assert_eq!(w.dropped(), 1);
        assert!(!w.flush(Duration::from_millis(50)));
        assert!(w.problem().is_some());
    }

    #[test]
    fn rows_spanning_two_days_go_to_two_files_in_order() {
        let tmp = tempfile::tempdir().unwrap();
        let health = Health::default();
        let batch = vec![
            ("2026-10-17".to_string(), "a".to_string()),
            ("2026-10-17".to_string(), "b".to_string()),
            ("2026-10-18".to_string(), "c".to_string()),
        ];
        write_batch(tmp.path(), "s1", &batch, &health);
        let day = |d: &str| {
            std::fs::read_to_string(tmp.path().join(format!("activity-{d}.csv"))).unwrap()
        };
        assert!(day("2026-10-17").ends_with("a\nb\n"));
        assert!(day("2026-10-18").ends_with("c\n"));
        assert_eq!(health.failed.load(Ordering::Relaxed), 0);
    }
}
