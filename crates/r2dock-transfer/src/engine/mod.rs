//! Queue, scheduler, and job controls (PLAN.md 12.3).
//!
//! Bounded concurrency: a fixed number of jobs run at once, and a global
//! semaphore caps in-flight part requests. Groups (folder transfers) are
//! scheduled fairly so one huge folder does not starve a single file.

use std::collections::{HashMap, HashSet, VecDeque};
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use chrono::Utc;
use parking_lot::{Mutex, RwLock};
use r2dock_core::keys::{base_name, display_name};
use r2dock_core::parts::use_multipart;
use r2dock_core::redact::redact_path;
use r2dock_core::{AppError, ErrorCode};
use r2dock_r2::R2Client;
use tokio::sync::{Notify, Semaphore};
use uuid::Uuid;

use crate::ctx::{JobCtx, JobHooks, Outcome, ResumeState, RunControl, Stop};
use crate::history::{History, HistoryEntry};
use crate::journal::{Journal, JournalEntry};
use crate::model::{
    Direction, GroupInfo, JobAction, JobId, JobSnapshot, JobSpec, JobState, ProgressUpdate,
    TransferEvent,
};
use crate::progress::{Counter, SpeedMeter};
use crate::retry::{JobError, MAX_ATTEMPTS, backoff};

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;
pub type EventSink = Arc<dyn Fn(TransferEvent) + Send + Sync>;

/// Supplies a ready client for a connection (built from the stored secret).
pub trait ClientProvider: Send + Sync + 'static {
    fn client(&self, connection: Uuid) -> BoxFuture<'_, Result<R2Client, AppError>>;
}

#[derive(Debug, Clone)]
pub struct EngineConfig {
    pub concurrency: usize,
    pub parts_per_job: usize,
    pub global_parts: usize,
    pub multipart_threshold: u64,
    pub data_dir: PathBuf,
    pub home: Option<PathBuf>,
    pub progress_interval: Duration,
}

impl EngineConfig {
    pub fn new(data_dir: PathBuf) -> Self {
        Self {
            concurrency: 3,
            parts_per_job: 4,
            global_parts: 8,
            multipart_threshold: 100 * r2dock_core::parts::MIB,
            data_dir,
            home: None,
            progress_interval: Duration::from_millis(250),
        }
    }
}

struct Entry {
    snap: JobSnapshot,
    spec: JobSpec,
    resume: Arc<Mutex<ResumeState>>,
    counter: Arc<Counter>,
    control: Option<Arc<RunControl>>,
    meter: Option<SpeedMeter>,
    started: Option<Instant>,
    last_sent: u64,
}

#[derive(Default)]
struct Jobs {
    order: Vec<JobId>,
    map: HashMap<JobId, Entry>,
    queue: VecDeque<JobId>,
}

struct Inner {
    jobs: Mutex<Jobs>,
    wake: Notify,
    progress_wake: Notify,
    sink: RwLock<Option<EventSink>>,
    delivery: Mutex<()>,
    provider: Arc<dyn ClientProvider>,
    part_slots: Arc<Semaphore>,
    concurrency: AtomicUsize,
    threshold: Mutex<u64>,
    slow_until: Mutex<Option<Instant>>,
    journal: Arc<Journal>,
    history: Arc<History>,
    config: EngineConfig,
}

#[derive(Clone)]
pub struct Engine {
    inner: Arc<Inner>,
}

fn now_rfc3339() -> String {
    Utc::now().to_rfc3339()
}

impl Engine {
    /// Create the engine and start its background tasks. Jobs found in the
    /// journal come back as `Interrupted`. Must be called inside a Tokio runtime.
    pub fn start(provider: Arc<dyn ClientProvider>, config: EngineConfig) -> Self {
        let transfers = config.data_dir.join("transfers");
        let (journal, recovered) = Journal::load(transfers.join("journal.json"));
        journal.spawn_writer();
        let history = Arc::new(History::new(transfers.join("history.jsonl")));
        let inner = Arc::new(Inner {
            jobs: Mutex::new(Jobs::default()),
            wake: Notify::new(),
            progress_wake: Notify::new(),
            sink: RwLock::new(None),
            delivery: Mutex::new(()),
            provider,
            part_slots: Arc::new(Semaphore::new(config.global_parts.max(1))),
            concurrency: AtomicUsize::new(config.concurrency.clamp(1, 8)),
            threshold: Mutex::new(config.multipart_threshold),
            slow_until: Mutex::new(None),
            journal,
            history,
            config,
        });
        let engine = Self { inner };
        engine.restore(recovered);
        tokio::spawn(scheduler(engine.inner.clone()));
        tokio::spawn(progress_ticker(engine.inner.clone()));
        engine
    }

    fn restore(&self, recovered: Vec<JournalEntry>) {
        let mut jobs = self.inner.jobs.lock();
        for e in recovered {
            let mut snap = self.inner.new_snapshot(e.id, &e.spec, e.group.clone());
            snap.state = JobState::Interrupted;
            snap.created_at = e.created_at.clone();
            let counter = Arc::new(Counter::default());
            match &e.resume {
                ResumeState::Multipart(m) => {
                    let done: u64 = m.done.values().map(|p| p.len).sum();
                    counter.set_committed(done);
                    snap.parts_done = Some(u32::try_from(m.done.len()).unwrap_or(0));
                }
                ResumeState::Download(d) => {
                    let len = std::fs::metadata(&d.temp).map(|m| m.len()).unwrap_or(0);
                    counter.set_committed(len);
                }
                ResumeState::None => {}
            }
            snap.transferred_bytes = counter.total();
            jobs.order.push(e.id);
            jobs.map.insert(
                e.id,
                Entry {
                    snap,
                    spec: e.spec,
                    resume: Arc::new(Mutex::new(e.resume)),
                    counter,
                    control: None,
                    meter: None,
                    started: None,
                    last_sent: 0,
                },
            );
        }
    }

    /// Set (or replace) the event sink and send a full snapshot.
    pub fn subscribe(&self, sink: EventSink) {
        // Hold the delivery lock so no event can slip between the snapshot
        // and the new sink, or arrive before the snapshot.
        let _order = self.inner.delivery.lock();
        let jobs = self.snapshot();
        sink(TransferEvent::Snapshot { jobs });
        *self.inner.sink.write() = Some(sink);
    }

    pub fn set_concurrency(&self, n: usize) {
        self.inner
            .concurrency
            .store(n.clamp(1, 8), Ordering::Relaxed);
        self.inner.wake.notify_one();
    }

    pub fn set_multipart_threshold(&self, bytes: u64) {
        *self.inner.threshold.lock() = bytes;
    }

    pub fn snapshot(&self) -> Vec<JobSnapshot> {
        let jobs = self.inner.jobs.lock();
        jobs.order
            .iter()
            .filter_map(|id| jobs.map.get(id))
            .map(|e| e.snap.clone())
            .collect()
    }

    /// Add jobs to the queue.
    pub fn enqueue(&self, specs: Vec<JobSpec>, group: Option<GroupInfo>) -> Vec<JobId> {
        let mut added = Vec::with_capacity(specs.len());
        let mut ids = Vec::with_capacity(specs.len());
        {
            let mut jobs = self.inner.jobs.lock();
            for spec in specs {
                let id = Uuid::new_v4();
                let snap = self.inner.new_snapshot(id, &spec, group.clone());
                added.push(snap.clone());
                jobs.order.push(id);
                jobs.queue.push_back(id);
                jobs.map.insert(
                    id,
                    Entry {
                        snap,
                        spec,
                        resume: Arc::new(Mutex::new(ResumeState::None)),
                        counter: Arc::new(Counter::default()),
                        control: None,
                        meter: None,
                        started: None,
                        last_sent: 0,
                    },
                );
                ids.push(id);
            }
        }
        // Large batches are sent in chunks to keep IPC messages small.
        for chunk in added.chunks(500) {
            self.inner.emit(TransferEvent::Added {
                jobs: chunk.to_vec(),
            });
        }
        self.inner.wake.notify_one();
        ids
    }

    /// Apply a user action to one job.
    pub fn control(&self, id: JobId, action: JobAction) -> Result<(), AppError> {
        self.inner.control(id, action)
    }

    /// Apply an action to many jobs; jobs where it does not apply are ignored.
    pub fn control_many(&self, ids: &[JobId], action: JobAction) {
        for id in ids {
            let _ = self.inner.control(*id, action);
        }
    }

    /// Pause every job that supports pausing.
    pub fn pause_all(&self) {
        let ids: Vec<JobId> = {
            let jobs = self.inner.jobs.lock();
            jobs.order
                .iter()
                .filter(|id| {
                    jobs.map.get(id).is_some_and(|e| {
                        e.snap.pausable
                            && (e.snap.state.is_active() || e.snap.state == JobState::Queued)
                    })
                })
                .copied()
                .collect()
        };
        self.control_many(&ids, JobAction::Pause);
    }

    /// Remove completed, skipped, and cancelled jobs from the list.
    pub fn clear_finished(&self) {
        let removed: Vec<JobId> = {
            let mut jobs = self.inner.jobs.lock();
            let ids: Vec<JobId> = jobs
                .order
                .iter()
                .filter(|id| jobs.map.get(id).is_some_and(|e| e.snap.state.is_terminal()))
                .copied()
                .collect();
            for id in &ids {
                jobs.map.remove(id);
            }
            let keep: HashSet<JobId> = jobs.map.keys().copied().collect();
            jobs.order.retain(|id| keep.contains(id));
            ids
        };
        if !removed.is_empty() {
            self.inner.emit(TransferEvent::Removed { ids: removed });
        }
    }

    /// Past transfers, newest first. Blocking file read.
    pub fn history(&self, limit: usize) -> Vec<HistoryEntry> {
        self.inner.history.read(limit)
    }

    pub fn trim_history(&self, days: u32) {
        self.inner.history.trim(days, Utc::now());
    }

    pub fn clear_history(&self) {
        self.inner.history.clear();
    }

    /// Fail queued jobs for a connection (for example after it is removed).
    pub fn fail_connection(&self, connection: Uuid, message: &str) {
        self.inner.fail_connection(
            connection,
            AppError::new(ErrorCode::CredentialMissing, message),
        );
    }

    /// Persist resumable state now (call before the app exits). Blocking.
    pub fn flush(&self) {
        self.inner.journal.flush();
    }

    /// Final file of a completed download.
    pub fn download_path(&self, id: JobId) -> Option<PathBuf> {
        let jobs = self.inner.jobs.lock();
        let e = jobs.map.get(&id)?;
        match (&e.spec, e.snap.state) {
            (JobSpec::Download(d), JobState::Completed) => Some(d.dest_root.join(&d.rel_path)),
            _ => None,
        }
    }

    /// Number of jobs that are queued or active.
    pub fn active_count(&self) -> usize {
        let jobs = self.inner.jobs.lock();
        jobs.map
            .values()
            .filter(|e| e.snap.state.is_active() || e.snap.state == JobState::Queued)
            .count()
    }
}

impl Inner {
    fn emit(&self, event: TransferEvent) {
        // Events are delivered in order. The sink must not block on the UI
        // thread (the app layer only forwards to a channel).
        let _order = self.delivery.lock();
        let sink = self.sink.read().clone();
        if let Some(sink) = sink {
            sink(event);
        }
    }

    fn local_display(&self, spec: &JobSpec) -> String {
        let home = self.config.home.as_deref();
        match spec {
            JobSpec::Upload(u) => redact_path(&u.local, home),
            JobSpec::Download(d) => redact_path(&d.dest_root.join(&d.rel_path), home),
        }
    }

    fn new_snapshot(&self, id: JobId, spec: &JobSpec, group: Option<GroupInfo>) -> JobSnapshot {
        let threshold = *self.threshold.lock();
        let pausable = match spec {
            JobSpec::Upload(u) => use_multipart(u.size, threshold),
            JobSpec::Download(_) => true,
        };
        JobSnapshot {
            id,
            group,
            direction: spec.direction(),
            connection_id: spec.connection(),
            bucket: spec.bucket().to_string(),
            key: spec.key().to_string(),
            name: display_name(base_name(spec.key())),
            local_display: self.local_display(spec),
            total_bytes: spec.size(),
            transferred_bytes: 0,
            state: JobState::Queued,
            speed_bps: None,
            eta_secs: None,
            attempt: 1,
            max_attempts: MAX_ATTEMPTS,
            parts_done: None,
            parts_total: None,
            pausable,
            error: None,
            created_at: now_rfc3339(),
            finished_at: None,
        }
    }

    /// Change state and notify. Returns the updated snapshot.
    fn set_state(&self, id: JobId, state: JobState, error: Option<AppError>) {
        let snap = {
            let mut jobs = self.jobs.lock();
            let Some(e) = jobs.map.get_mut(&id) else {
                return;
            };
            e.snap.state = state;
            e.snap.error = error;
            e.snap.transferred_bytes = e.counter.total();
            if state.is_active() {
                if e.started.is_none() {
                    e.started = Some(Instant::now());
                }
            } else {
                e.snap.speed_bps = None;
                e.snap.eta_secs = None;
                e.meter = None;
            }
            if matches!(
                state,
                JobState::Completed | JobState::Failed | JobState::Cancelled | JobState::Skipped
            ) {
                e.snap.finished_at = Some(now_rfc3339());
            } else {
                e.snap.finished_at = None;
            }
            if state == JobState::Queued && !jobs.queue.contains(&id) {
                jobs.queue.push_back(id);
            }
            jobs.map.get(&id).map(|e| e.snap.clone())
        };
        if let Some(job) = snap {
            if state.is_active() {
                self.progress_wake.notify_one();
            }
            self.emit(TransferEvent::Updated { job: Box::new(job) });
        }
    }

    fn effective_concurrency(&self) -> usize {
        let base = self.concurrency.load(Ordering::Relaxed);
        let slow = self.slow_until.lock().is_some_and(|t| Instant::now() < t);
        if slow { (base / 2).max(1) } else { base }
    }

    /// Start queued jobs while slots are free.
    fn schedule(self: &Arc<Self>) {
        let limit = self.effective_concurrency();
        let mut to_start = Vec::new();
        {
            let mut guard = self.jobs.lock();
            let Jobs { map, queue, .. } = &mut *guard;
            queue.retain(|id| {
                map.get(id)
                    .is_some_and(|e| e.snap.state == JobState::Queued)
            });
            let mut active = 0usize;
            let mut per_group: HashMap<Option<Uuid>, usize> = HashMap::new();
            let mut busy: HashSet<(Uuid, String, String)> = HashSet::new();
            for e in map.values().filter(|e| e.snap.state.is_active()) {
                active += 1;
                *per_group
                    .entry(e.snap.group.as_ref().map(|g| g.id))
                    .or_default() += 1;
                if e.snap.direction == Direction::Upload {
                    busy.insert((
                        e.snap.connection_id,
                        e.snap.bucket.clone(),
                        e.spec.key().to_string(),
                    ));
                }
            }
            while active < limit {
                // Look at the front of the queue; prefer the group with the fewest running jobs.
                let pick = queue
                    .iter()
                    .take(256)
                    .enumerate()
                    .filter(|(_, id)| {
                        map.get(id).is_some_and(|e| {
                            e.snap.direction == Direction::Download
                                || !busy.contains(&(
                                    e.snap.connection_id,
                                    e.snap.bucket.clone(),
                                    e.spec.key().to_string(),
                                ))
                        })
                    })
                    .min_by_key(|(i, id)| {
                        let g = map
                            .get(id)
                            .and_then(|e| e.snap.group.as_ref().map(|g| g.id));
                        (per_group.get(&g).copied().unwrap_or(0), *i)
                    })
                    .map(|(i, id)| (i, *id));
                let Some((index, id)) = pick else { break };
                queue.remove(index);
                let Some(e) = map.get_mut(&id) else { continue };
                e.snap.state = JobState::Preparing;
                e.snap.error = None;
                let control = Arc::new(RunControl::new());
                e.control = Some(control.clone());
                if e.snap.direction == Direction::Upload {
                    busy.insert((
                        e.snap.connection_id,
                        e.snap.bucket.clone(),
                        e.spec.key().to_string(),
                    ));
                }
                *per_group
                    .entry(e.snap.group.as_ref().map(|g| g.id))
                    .or_default() += 1;
                active += 1;
                to_start.push((id, control, e.snap.clone()));
            }
        }
        for (id, control, snap) in to_start {
            self.emit(TransferEvent::Updated {
                job: Box::new(snap),
            });
            self.progress_wake.notify_one();
            tokio::spawn(run_job(self.clone(), id, control));
        }
    }

    fn control(self: &Arc<Self>, id: JobId, action: JobAction) -> Result<(), AppError> {
        enum Then {
            Nothing,
            Emit(Box<JobSnapshot>),
            Cancelled,
            Removed(ResumeState, Uuid, String),
        }
        let not_applicable =
            || AppError::invalid_input("That action is not available for this transfer.");
        // Check and change state under one lock so the scheduler cannot start
        // a job between the check and the change.
        let then = {
            let mut guard = self.jobs.lock();
            let Jobs { map, queue, order } = &mut *guard;
            let e = map
                .get_mut(&id)
                .ok_or_else(|| AppError::not_found("Transfer not found."))?;
            let state = e.snap.state;
            let requeue = |e: &mut Entry, queue: &mut VecDeque<JobId>| {
                e.snap.state = JobState::Queued;
                e.snap.error = None;
                e.snap.finished_at = None;
                if !queue.contains(&id) {
                    queue.push_back(id);
                }
                Then::Emit(Box::new(e.snap.clone()))
            };
            match action {
                JobAction::Pause => {
                    if !e.snap.pausable {
                        return Err(not_applicable());
                    }
                    if state.is_active() {
                        if let Some(c) = &e.control {
                            let interrupt = e.snap.direction == Direction::Download
                                || state == JobState::Retrying;
                            c.request_pause(interrupt);
                        }
                        Then::Nothing
                    } else if state == JobState::Queued {
                        e.snap.state = JobState::Paused;
                        queue.retain(|x| *x != id);
                        Then::Emit(Box::new(e.snap.clone()))
                    } else {
                        return Err(not_applicable());
                    }
                }
                JobAction::Resume => {
                    if !matches!(state, JobState::Paused | JobState::Interrupted) {
                        return Err(not_applicable());
                    }
                    requeue(e, queue)
                }
                JobAction::Retry => {
                    if !matches!(
                        state,
                        JobState::Failed | JobState::Interrupted | JobState::Cancelled
                    ) {
                        return Err(not_applicable());
                    }
                    e.snap.attempt = 1;
                    requeue(e, queue)
                }
                JobAction::Resolve(policy) => {
                    if state != JobState::NeedsDecision {
                        return Err(not_applicable());
                    }
                    e.spec.set_policy(policy);
                    requeue(e, queue)
                }
                JobAction::Cancel => {
                    if state.is_terminal() {
                        return Err(not_applicable());
                    }
                    if state.is_active() {
                        if let Some(c) = &e.control {
                            c.request_cancel();
                        }
                        Then::Nothing
                    } else {
                        e.snap.state = JobState::Cancelled;
                        queue.retain(|x| *x != id);
                        Then::Cancelled
                    }
                }
                JobAction::Dismiss => {
                    if state.is_active() {
                        return Err(not_applicable());
                    }
                    let resume = if state == JobState::Completed {
                        ResumeState::None
                    } else {
                        std::mem::take(&mut *e.resume.lock())
                    };
                    let (conn, bucket) = (e.snap.connection_id, e.snap.bucket.clone());
                    map.remove(&id);
                    order.retain(|x| *x != id);
                    queue.retain(|x| *x != id);
                    Then::Removed(resume, conn, bucket)
                }
            }
        };
        match then {
            Then::Nothing => {}
            Then::Emit(job) => {
                self.emit(TransferEvent::Updated { job });
                self.wake.notify_one();
            }
            Then::Cancelled => {
                self.discard_partial(id);
                self.finish(id, JobState::Cancelled, None);
            }
            Then::Removed(resume, conn, bucket) => {
                self.journal.remove(id);
                self.discard_resume(resume, conn, bucket);
                self.emit(TransferEvent::Removed { ids: vec![id] });
            }
        }
        Ok(())
    }

    /// Abort a saved multipart upload or delete a partial download, in the background.
    fn discard_partial(self: &Arc<Self>, id: JobId) {
        let (resume, connection, bucket) = {
            let jobs = self.jobs.lock();
            let Some(e) = jobs.map.get(&id) else { return };
            let r = std::mem::take(&mut *e.resume.lock());
            (r, e.snap.connection_id, e.snap.bucket.clone())
        };
        self.journal.remove(id);
        self.discard_resume(resume, connection, bucket);
    }

    fn discard_resume(self: &Arc<Self>, resume: ResumeState, connection: Uuid, bucket: String) {
        match resume {
            ResumeState::None => {}
            ResumeState::Download(d) => {
                let _ = std::fs::remove_file(&d.temp);
            }
            ResumeState::Multipart(m) => {
                let inner = self.clone();
                tokio::spawn(async move {
                    if let Ok(client) = inner.provider.client(connection).await {
                        let _ = client.abort_multipart(&bucket, &m.key, &m.upload_id).await;
                    }
                });
            }
        }
    }

    fn fail_connection(&self, connection: Uuid, error: AppError) {
        let ids: Vec<JobId> = {
            let jobs = self.jobs.lock();
            jobs.map
                .values()
                .filter(|e| e.snap.connection_id == connection && e.snap.state == JobState::Queued)
                .map(|e| e.snap.id)
                .collect()
        };
        for id in ids {
            self.set_state(id, JobState::Failed, Some(error.clone()));
        }
    }

    /// Record a final state, history, and journal cleanup.
    fn finish(&self, id: JobId, state: JobState, error: Option<AppError>) {
        self.set_state(id, state, error.clone());
        let entry = {
            let jobs = self.jobs.lock();
            let Some(e) = jobs.map.get(&id) else {
                return;
            };
            let duration = e.started.map_or(0.0, |s| s.elapsed().as_secs_f64());
            HistoryEntry {
                id,
                direction: e.snap.direction,
                connection_id: e.snap.connection_id,
                bucket: e.snap.bucket.clone(),
                key: e.snap.key.clone(),
                name: e.snap.name.clone(),
                local_display: e.snap.local_display.clone(),
                bytes: e.snap.total_bytes,
                state,
                message: error.map(|e| e.message),
                finished_at: now_rfc3339(),
                duration_secs: duration,
            }
        };
        if matches!(
            state,
            JobState::Completed | JobState::Cancelled | JobState::Skipped
        ) {
            self.journal.remove(id);
        }
        let history = self.history.clone();
        tokio::task::spawn_blocking(move || history.append(&entry));
    }
}

/// Hooks given to running jobs.
struct Hooks(Arc<Inner>);

impl JobHooks for Hooks {
    fn state(&self, id: JobId, state: JobState) {
        self.0.set_state(id, state, None);
    }

    fn parts(&self, id: JobId, done: u32, total: u32) {
        let mut jobs = self.0.jobs.lock();
        if let Some(e) = jobs.map.get_mut(&id) {
            e.snap.parts_done = Some(done);
            e.snap.parts_total = Some(total);
        }
    }

    fn renamed(&self, id: JobId, key: Option<String>, rel_path: Option<PathBuf>) {
        let snap = {
            let mut jobs = self.0.jobs.lock();
            let Some(e) = jobs.map.get_mut(&id) else {
                return;
            };
            match (&mut e.spec, key, rel_path) {
                (JobSpec::Upload(u), Some(k), _) => u.key = k,
                (JobSpec::Download(d), _, Some(p)) => d.rel_path = p,
                _ => {}
            }
            e.snap.key = e.spec.key().to_string();
            e.snap.name = match &e.spec {
                JobSpec::Upload(u) => display_name(base_name(&u.key)),
                JobSpec::Download(d) => display_name(
                    &d.rel_path
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                ),
            };
            e.snap.local_display = self.0.local_display(&e.spec);
            e.snap.clone()
        };
        self.0.emit(TransferEvent::Updated {
            job: Box::new(snap),
        });
    }

    fn total(&self, id: JobId, total: u64) {
        let mut jobs = self.0.jobs.lock();
        if let Some(e) = jobs.map.get_mut(&id) {
            e.snap.total_bytes = total;
            match &mut e.spec {
                JobSpec::Upload(u) => u.size = total,
                JobSpec::Download(d) => d.size = total,
            }
        }
    }

    fn checkpoint(&self, id: JobId) {
        let entry = {
            let jobs = self.0.jobs.lock();
            let Some(e) = jobs.map.get(&id) else { return };
            let resume = e.resume.lock().clone();
            if resume == ResumeState::None {
                None
            } else {
                Some(JournalEntry {
                    id,
                    spec: e.spec.clone(),
                    group: e.snap.group.clone(),
                    resume,
                    created_at: e.snap.created_at.clone(),
                })
            }
        };
        match entry {
            Some(entry) => self.0.journal.upsert(entry),
            None => self.0.journal.remove(id),
        }
    }
}

async fn scheduler(inner: Arc<Inner>) {
    loop {
        inner.schedule();
        // Re-check after a slowdown window ends even without other events.
        let wait = inner
            .slow_until
            .lock()
            .map(|t| t.saturating_duration_since(Instant::now()))
            .filter(|d| !d.is_zero());
        match wait {
            Some(d) => {
                tokio::select! {
                    () = inner.wake.notified() => {}
                    () = tokio::time::sleep(d) => {}
                }
            }
            None => inner.wake.notified().await,
        }
    }
}

/// Emits progress at most every interval while jobs are active. Sleeps otherwise.
async fn progress_ticker(inner: Arc<Inner>) {
    loop {
        inner.progress_wake.notified().await;
        loop {
            tokio::time::sleep(inner.config.progress_interval).await;
            let now = Instant::now();
            let (updates, any_active) = {
                let mut jobs = inner.jobs.lock();
                let mut updates = Vec::new();
                let mut any_active = false;
                for e in jobs.map.values_mut() {
                    if !e.snap.state.is_active() {
                        continue;
                    }
                    any_active = true;
                    let bytes = e.counter.total();
                    let total = e.snap.total_bytes;
                    let meter = e.meter.get_or_insert_with(|| SpeedMeter::new(now, bytes));
                    let (speed, eta) = meter.sample(now, bytes, total);
                    e.snap.transferred_bytes = bytes;
                    e.snap.speed_bps = speed;
                    e.snap.eta_secs = eta;
                    if bytes != e.last_sent || speed.is_some() {
                        e.last_sent = bytes;
                        updates.push(ProgressUpdate {
                            id: e.snap.id,
                            transferred_bytes: bytes,
                            speed_bps: speed,
                            eta_secs: eta,
                            parts_done: e.snap.parts_done,
                        });
                    }
                }
                (updates, any_active)
            };
            if !updates.is_empty() {
                inner.emit(TransferEvent::Progress { updates });
            }
            if !any_active {
                break;
            }
        }
    }
}

async fn run_job(inner: Arc<Inner>, id: JobId, control: Arc<RunControl>) {
    loop {
        let (spec, resume, counter, connection) = {
            let jobs = inner.jobs.lock();
            let Some(e) = jobs.map.get(&id) else { return };
            (
                e.spec.clone(),
                e.resume.clone(),
                e.counter.clone(),
                e.snap.connection_id,
            )
        };
        let client = tokio::select! {
            () = control.cancel.cancelled() => Err(AppError::new(ErrorCode::Cancelled, "Cancelled.")),
            c = inner.provider.client(connection) => c,
        };
        let result = match client {
            Err(e) if e.code == ErrorCode::Cancelled => Err(JobError::cancelled()),
            Err(e) => Err(JobError::fatal(e)),
            Ok(client) => {
                let ctx = JobCtx {
                    id,
                    client,
                    control: control.clone(),
                    counter,
                    part_slots: inner.part_slots.clone(),
                    parts_per_job: inner.config.parts_per_job.max(1),
                    multipart_threshold: *inner.threshold.lock(),
                    resume,
                    hooks: Arc::new(Hooks(inner.clone())),
                };
                let mut spec = spec;
                let r = match &mut spec {
                    JobSpec::Upload(u) => crate::upload::run(&ctx, u).await,
                    JobSpec::Download(d) => crate::download::run(&ctx, d).await,
                };
                if let Some(e) = inner.jobs.lock().map.get_mut(&id) {
                    e.spec = spec;
                    e.snap.key = e.spec.key().to_string();
                }
                r
            }
        };

        match result {
            Ok(Outcome::Completed) => {
                inner.finish(id, JobState::Completed, None);
            }
            Ok(Outcome::Skipped) => inner.finish(id, JobState::Skipped, None),
            Ok(Outcome::NeedsDecision) => {
                inner.set_state(
                    id,
                    JobState::NeedsDecision,
                    Some(AppError::new(
                        ErrorCode::AlreadyExists,
                        "A file with this name already exists.",
                    )),
                );
            }
            Ok(Outcome::Paused) => inner.set_state(id, JobState::Paused, None),
            Err(e) if e.cancelled => match control.stop() {
                Stop::Pause => inner.set_state(id, JobState::Paused, None),
                _ => {
                    inner.discard_partial(id);
                    inner.finish(id, JobState::Cancelled, None);
                }
            },
            Err(e) => {
                if matches!(e.app.code, ErrorCode::RateLimited | ErrorCode::ServerError) {
                    *inner.slow_until.lock() = Some(Instant::now() + Duration::from_secs(60));
                }
                if e.auth {
                    inner.finish(id, JobState::Failed, Some(e.app.clone()));
                    inner.fail_connection(connection, e.app);
                    inner.emit(TransferEvent::AuthFailed {
                        connection_id: connection,
                    });
                } else {
                    let attempt = inner
                        .jobs
                        .lock()
                        .map
                        .get(&id)
                        .map_or(MAX_ATTEMPTS, |e| e.snap.attempt);
                    if e.retryable && attempt < MAX_ATTEMPTS && control.stop() == Stop::None {
                        if let Some(entry) = inner.jobs.lock().map.get_mut(&id) {
                            entry.snap.attempt = attempt + 1;
                        }
                        inner.set_state(id, JobState::Retrying, Some(e.app.clone()));
                        let wait = backoff(attempt + 1);
                        tokio::select! {
                            () = control.cancel.cancelled() => {}
                            () = tokio::time::sleep(wait) => {}
                        }
                        match control.stop() {
                            Stop::None => continue,
                            Stop::Pause => inner.set_state(id, JobState::Paused, None),
                            Stop::Cancel => {
                                inner.discard_partial(id);
                                inner.finish(id, JobState::Cancelled, None);
                            }
                        }
                    } else {
                        inner.finish(id, JobState::Failed, Some(e.app));
                    }
                }
            }
        }
        break;
    }
    // Clear the control only if a newer run has not replaced it.
    if let Some(e) = inner.jobs.lock().map.get_mut(&id)
        && e.control.as_ref().is_some_and(|c| Arc::ptr_eq(c, &control))
    {
        e.control = None;
    }
    inner.wake.notify_one();
}
