//! Connection lifecycle (PLAN.md 9.3): test, save, edit, replace secret,
//! remove, and building clients from stored secrets.
//!
//! Secrets go to the OS credential store, or to session memory when the
//! store is unavailable. They are never written to the config file and never
//! returned to the frontend.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use chrono::Utc;
use parking_lot::{Mutex, RwLock};
use r2dock_core::connections::{
    ConnectionConfig, ConnectionsFile, SecretLocation, account_id_looks_unusual, shorten_id,
    validate_access_key_id, validate_account_id, validate_api_token, validate_bucket_name,
    validate_name, validate_public_base_url, validate_secret,
};
use r2dock_core::storage::{read_json_or_default, write_json_atomic};
use r2dock_core::{AppError, ErrorCode};
use r2dock_r2::ops::TestOutcome;
use r2dock_r2::{ClientParams, R2Client, R2Error};
use r2dock_secrets::{MemoryStore, SecretError, SecretStore, new_reference};
use secrecy::SecretString;
use uuid::Uuid;

use crate::dto::{ConnectionInput, ConnectionStatus, ConnectionSummary, SecretStorage, TestResult};

pub struct ConnectionService {
    path: PathBuf,
    file: RwLock<ConnectionsFile>,
    /// The OS credential store, when it works on this system.
    keychain: Option<Arc<dyn SecretStore>>,
    session: Arc<MemoryStore>,
    clients: Mutex<HashMap<Uuid, R2Client>>,
    /// Bumped whenever a connection's credentials or settings change, so a
    /// client built from older values is never cached.
    generation: Mutex<HashMap<Uuid, u64>>,
    /// Serializes snapshot-and-write of the config file.
    persist_lock: tokio::sync::Mutex<()>,
    status: Mutex<HashMap<Uuid, ConnectionStatus>>,
    missing: Mutex<std::collections::HashSet<Uuid>>,
    /// Test-only: point every client at a local S3 server.
    endpoint_override: Option<String>,
    /// Credential store references that must be deleted but could not be
    /// recorded in the config file yet (the save that would record them
    /// failed). They are added to the next saved config.
    unsaved_cleanup: Mutex<Vec<String>>,
    #[cfg(test)]
    fail_writes: std::sync::atomic::AtomicBool,
}

/// Session memory key of a connection's optional API token.
fn api_session_key(id: Uuid) -> String {
    format!("{id}:api-token")
}

/// The small file next to the config that holds credential store references
/// still to be deleted while the config itself cannot be saved. References
/// only, never secret values.
fn cleanup_path(config: &std::path::Path) -> PathBuf {
    config.with_file_name("connections-cleanup.json")
}

/// What a save does with the optional API token.
enum ApiTokenChange {
    Keep,
    Replace(SecretString),
    Remove,
}

fn blocking_err() -> AppError {
    AppError::internal("A background task stopped unexpectedly.")
}

fn secret_error(err: SecretError) -> AppError {
    match err {
        SecretError::NotFound => {
            AppError::new(ErrorCode::CredentialMissing, "Keys not found in keychain.")
        }
        SecretError::Unavailable | SecretError::Denied => AppError::new(
            ErrorCode::CredentialStoreUnavailable,
            "The system keychain is not available, so the key could not be saved or read.",
        ),
        SecretError::Other => AppError::new(
            ErrorCode::CredentialStoreUnavailable,
            "The system keychain returned an error.",
        ),
    }
}

impl ConnectionService {
    pub fn load(
        path: PathBuf,
        keychain: Option<Arc<dyn SecretStore>>,
        endpoint_override: Option<String>,
    ) -> Self {
        let file: ConnectionsFile = read_json_or_default(&path).unwrap_or_default();
        // References left by a run that could not save its config. They are
        // recorded by the next save and deleted by the next cleanup.
        let leftover: Vec<String> = read_json_or_default(&cleanup_path(&path)).unwrap_or_default();
        Self {
            path,
            file: RwLock::new(file),
            keychain,
            session: Arc::new(MemoryStore::new()),
            clients: Mutex::new(HashMap::new()),
            generation: Mutex::new(HashMap::new()),
            persist_lock: tokio::sync::Mutex::new(()),
            status: Mutex::new(HashMap::new()),
            missing: Mutex::new(std::collections::HashSet::new()),
            endpoint_override,
            unsaved_cleanup: Mutex::new(leftover),
            #[cfg(test)]
            fail_writes: std::sync::atomic::AtomicBool::new(false),
        }
    }

    /// Write the unsaved cleanup references to their own small file, so they
    /// survive a quit before the next successful config save. An empty list
    /// removes the file. Best effort: the in-memory list is kept either way.
    async fn persist_unsaved_cleanup(&self) {
        let pending = self.unsaved_cleanup.lock().clone();
        let path = cleanup_path(&self.path);
        let written = tokio::task::spawn_blocking(move || {
            if pending.is_empty() {
                match std::fs::remove_file(&path) {
                    Ok(()) => Ok(()),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                    Err(_) => Err(()),
                }
            } else {
                write_json_atomic(&path, &pending).map_err(|_| ())
            }
        })
        .await;
        if !matches!(written, Ok(Ok(()))) {
            tracing::warn!("could not record keys waiting to be deleted from the credential store");
        }
    }

    /// Drop the cached client and make any client being built now stale.
    fn invalidate(&self, id: Uuid) {
        self.clients.lock().remove(&id);
        *self.generation.lock().entry(id).or_default() += 1;
    }

    /// Changes whenever the connection's credentials or settings change.
    pub fn generation_of(&self, id: Uuid) -> u64 {
        self.generation.lock().get(&id).copied().unwrap_or(0)
    }

    pub fn keychain_available(&self) -> bool {
        self.keychain.is_some()
    }

    /// Apply `change` to a copy of the config, save the copy, and only then
    /// make it the live config. A failed save leaves memory and disk as they
    /// were. Commits run one at a time, so none is lost or reordered.
    async fn commit<R>(
        &self,
        change: impl FnOnce(&mut ConnectionsFile) -> Result<R, AppError>,
    ) -> Result<R, AppError> {
        self.commit_then(change, || ()).await
    }

    /// Like `commit`, and runs `publish` while the new config becomes live,
    /// before any reader can see it. Session secrets and client invalidation
    /// go here, so no reader ever pairs new settings with an old secret or
    /// old settings with a new secret.
    async fn commit_then<R>(
        &self,
        change: impl FnOnce(&mut ConnectionsFile) -> Result<R, AppError>,
        publish: impl FnOnce(),
    ) -> Result<R, AppError> {
        let _guard = self.persist_lock.lock().await;
        let mut candidate = self.file.read().clone();
        let out = change(&mut candidate)?;
        let unsaved: Vec<String> = self.unsaved_cleanup.lock().drain(..).collect();
        for r in &unsaved {
            candidate.add_cleanup(r.clone());
        }
        if let Err(e) = self.write(&candidate).await {
            self.unsaved_cleanup.lock().extend(unsaved);
            return Err(e);
        }
        {
            let mut live = self.file.write();
            *live = candidate;
            publish();
        }
        if !unsaved.is_empty() {
            // Now recorded in the config: the side file is no longer needed.
            self.persist_unsaved_cleanup().await;
        }
        Ok(out)
    }

    async fn write(&self, snapshot: &ConnectionsFile) -> Result<(), AppError> {
        let failed = || AppError::new(ErrorCode::Io, "Could not save connection settings.");
        #[cfg(test)]
        if self.fail_writes.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(failed());
        }
        let snapshot = snapshot.clone();
        let path = self.path.clone();
        tokio::task::spawn_blocking(move || write_json_atomic(&path, &snapshot))
            .await
            .map_err(|_| blocking_err())?
            .map_err(|_| failed())
    }

    /// How many unused secrets are still waiting to be deleted from the
    /// credential store.
    pub fn pending_secret_cleanup(&self) -> usize {
        self.file.read().secret_cleanup.len() + self.unsaved_cleanup.lock().len()
    }

    /// Delete secrets that are no longer used by any connection. Called at
    /// startup and after every connection change; safe to repeat. Returns how
    /// many are still waiting (for example while the credential store is
    /// unavailable).
    pub async fn cleanup_secrets(&self) -> usize {
        let pending = {
            let file = self.file.read();
            let mut pending = file.secret_cleanup.clone();
            pending.extend(self.unsaved_cleanup.lock().iter().cloned());
            pending.retain(|r| !file.uses_reference(r));
            pending
        };
        if let Some(store) = self.keychain.clone()
            && !pending.is_empty()
        {
            let deleted = tokio::task::spawn_blocking(move || {
                pending
                    .into_iter()
                    .filter(|r| store.delete(r).is_ok())
                    .collect::<Vec<_>>()
            })
            .await
            .unwrap_or_default();
            if !deleted.is_empty() {
                self.unsaved_cleanup.lock().retain(|r| !deleted.contains(r));
                self.persist_unsaved_cleanup().await;
                // If this save fails the references stay listed; deleting a
                // missing entry again later is harmless.
                let _ = self
                    .commit(|f| {
                        f.secret_cleanup.retain(|r| !deleted.contains(r));
                        Ok(())
                    })
                    .await;
            }
        }
        let left = self.pending_secret_cleanup();
        if left > 0 {
            tracing::warn!(
                pending = left,
                "some removed keys could not be deleted from the credential store yet"
            );
        }
        left
    }

    fn summary(&self, cfg: &ConnectionConfig, active: Option<Uuid>) -> ConnectionSummary {
        let missing = self.missing.lock().contains(&cfg.id);
        let secret_storage = match &cfg.secret {
            _ if missing => SecretStorage::Missing,
            SecretLocation::Keychain { .. } => SecretStorage::Keychain,
            SecretLocation::Session => {
                if self.session.get(&cfg.id.to_string()).is_ok() {
                    SecretStorage::Session
                } else {
                    SecretStorage::Missing
                }
            }
        };
        let status = if secret_storage == SecretStorage::Missing {
            ConnectionStatus::NeedsAttention
        } else {
            self.status
                .lock()
                .get(&cfg.id)
                .copied()
                .unwrap_or(ConnectionStatus::Unknown)
        };
        ConnectionSummary {
            id: cfg.id,
            name: cfg.name.clone(),
            account_id: cfg.account_id.clone(),
            account_id_short: shorten_id(&cfg.account_id),
            access_key_hint: shorten_id(&cfg.access_key_id),
            jurisdiction: cfg.jurisdiction,
            secret_storage,
            status,
            bucket_scoped: cfg.bucket_scoped,
            manual_buckets: cfg.manual_buckets.clone(),
            favorite_buckets: cfg.favorite_buckets.clone(),
            public_base_urls: cfg.public_base_urls.clone(),
            has_api_token: match &cfg.api_token {
                None => false,
                Some(SecretLocation::Keychain { .. }) => true,
                Some(SecretLocation::Session) => self.session.get(&api_session_key(cfg.id)).is_ok(),
            },
            last_tested_at: cfg.last_tested_at.map(|t| t.to_rfc3339()),
            active: active == Some(cfg.id),
        }
    }

    pub fn list(&self) -> Vec<ConnectionSummary> {
        let file = self.file.read();
        file.connections
            .iter()
            .map(|c| self.summary(c, file.active))
            .collect()
    }

    pub fn get(&self, id: Uuid) -> Result<ConnectionConfig, AppError> {
        self.file
            .read()
            .connections
            .iter()
            .find(|c| c.id == id)
            .cloned()
            .ok_or_else(|| AppError::not_found("Connection not found."))
    }

    fn summary_of(&self, id: Uuid) -> Result<ConnectionSummary, AppError> {
        let file = self.file.read();
        let cfg = file
            .connections
            .iter()
            .find(|c| c.id == id)
            .ok_or_else(|| AppError::not_found("Connection not found."))?;
        Ok(self.summary(cfg, file.active))
    }

    /// Record the outcome of a request so the sidebar shows the right status.
    pub fn note_result<T>(&self, id: Uuid, result: &Result<T, R2Error>) {
        let status = match result {
            Ok(_) => ConnectionStatus::Online,
            Err(e) if e.is_auth_failure() => ConnectionStatus::NeedsAttention,
            Err(R2Error::Network | R2Error::Timeout) => ConnectionStatus::Offline,
            Err(_) => return,
        };
        self.status.lock().insert(id, status);
        if status == ConnectionStatus::NeedsAttention {
            self.invalidate(id);
        }
    }

    /// Let finished transfers update the sidebar status too: a completed job
    /// proves the connection works, a network failure means it is offline.
    pub fn note_transfer(&self, job: &r2dock_transfer::JobSnapshot) {
        use r2dock_transfer::JobState;
        let status = match (job.state, job.error.as_ref().map(|e| e.code)) {
            (JobState::Completed, _) => ConnectionStatus::Online,
            (JobState::Failed, Some(ErrorCode::Network | ErrorCode::Timeout)) => {
                ConnectionStatus::Offline
            }
            _ => return,
        };
        let mut map = self.status.lock();
        // Never hide a credentials problem behind a network result.
        if map.get(&job.connection_id) != Some(&ConnectionStatus::NeedsAttention) {
            map.insert(job.connection_id, status);
        }
    }

    pub fn mark_needs_attention(&self, id: Uuid) {
        self.status
            .lock()
            .insert(id, ConnectionStatus::NeedsAttention);
        self.invalidate(id);
    }

    fn read_secret(&self, cfg: &ConnectionConfig) -> Result<SecretString, AppError> {
        let result = match &cfg.secret {
            SecretLocation::Keychain { reference } => match &self.keychain {
                Some(store) => store.get(reference),
                None => Err(SecretError::Unavailable),
            },
            SecretLocation::Session => self.session.get(&cfg.id.to_string()),
        };
        match result {
            Ok(s) => {
                self.missing.lock().remove(&cfg.id);
                Ok(s)
            }
            Err(SecretError::NotFound) => {
                self.missing.lock().insert(cfg.id);
                let message = match cfg.secret {
                    SecretLocation::Keychain { .. } => format!(
                        "Keys not found in keychain. The saved secret for \u{201c}{}\u{201d} was removed outside R2Dock.",
                        cfg.name
                    ),
                    SecretLocation::Session => format!(
                        "Enter the secret for \u{201c}{}\u{201d} again. Keys kept for one session are cleared when R2Dock quits.",
                        cfg.name
                    ),
                };
                Err(AppError::new(ErrorCode::CredentialMissing, message))
            }
            Err(e) => Err(secret_error(e)),
        }
    }

    /// The saved Cloudflare API token, if any. Blocking: call from
    /// `spawn_blocking`.
    fn read_api_token(&self, cfg: &ConnectionConfig) -> Result<Option<SecretString>, SecretError> {
        match &cfg.api_token {
            None => Ok(None),
            Some(SecretLocation::Keychain { reference }) => match &self.keychain {
                Some(store) => store.get(reference).map(Some),
                None => Err(SecretError::Unavailable),
            },
            Some(SecretLocation::Session) => self.session.get(&api_session_key(cfg.id)).map(Some),
        }
    }

    /// The saved Cloudflare API token of a connection, read off the async
    /// runtime. `Ok(None)` when none was saved.
    pub async fn api_token(
        self: &Arc<Self>,
        id: Uuid,
    ) -> Result<Option<SecretString>, SecretError> {
        let cfg = self.get(id).map_err(|_| SecretError::NotFound)?;
        let this = self.clone();
        tokio::task::spawn_blocking(move || this.read_api_token(&cfg))
            .await
            .map_err(|_| SecretError::Other)?
    }

    /// Write a secret to the credential store under a new reference. The
    /// reference is not used by any config yet.
    async fn store_new_reference(
        &self,
        store: Arc<dyn SecretStore>,
        secret: &SecretString,
    ) -> Result<String, AppError> {
        let reference = new_reference();
        let r = reference.clone();
        let secret = SecretString::from(secrecy::ExposeSecret::expose_secret(secret).to_string());
        tokio::task::spawn_blocking(move || store.set(&r, &secret))
            .await
            .map_err(|_| blocking_err())?
            .map_err(secret_error)?;
        Ok(reference)
    }

    fn params(
        &self,
        account_id: &str,
        access_key_id: &str,
        secret: SecretString,
        cfg_j: r2dock_core::connections::Jurisdiction,
    ) -> ClientParams {
        ClientParams {
            account_id: account_id.to_string(),
            access_key_id: access_key_id.to_string(),
            secret,
            jurisdiction: cfg_j,
            endpoint_override: self.endpoint_override.clone(),
        }
    }

    /// A cached client for a saved connection, built from the stored secret.
    pub async fn client(self: &Arc<Self>, id: Uuid) -> Result<R2Client, AppError> {
        // Retry if the connection changes while the secret is being read.
        for _ in 0..3 {
            if let Some(c) = self.clients.lock().get(&id) {
                return Ok(c.clone());
            }
            let generation = self.generation_of(id);
            let cfg = self.get(id)?;
            let this = self.clone();
            let cfg_c = cfg.clone();
            let secret = tokio::task::spawn_blocking(move || this.read_secret(&cfg_c))
                .await
                .map_err(|_| blocking_err())??;
            let client = R2Client::new(&self.params(
                &cfg.account_id,
                &cfg.access_key_id,
                secret,
                cfg.jurisdiction,
            ));
            let mut clients = self.clients.lock();
            if self.generation_of(id) == generation {
                clients.insert(id, client.clone());
                return Ok(client);
            }
        }
        Err(AppError::internal(
            "The connection changed while connecting. Try again.",
        ))
    }

    /// Validate the form and resolve the secret to use (typed, or stored when editing).
    async fn prepare(self: &Arc<Self>, input: &ConnectionInput) -> Result<Prepared, AppError> {
        let name = validate_name(&input.name)?;
        let account_id = validate_account_id(&input.account_id)?;
        // When editing, an empty Access Key ID keeps the stored one (the
        // frontend only ever sees a shortened hint of it).
        let access_key_id = match input.id {
            Some(id) if input.access_key_id.trim().is_empty() => self.get(id)?.access_key_id,
            _ => validate_access_key_id(&input.access_key_id)?,
        };
        // One or more bucket names, separated by commas, spaces, or new lines.
        let mut buckets: Vec<String> = Vec::new();
        for raw in input
            .bucket
            .as_deref()
            .unwrap_or_default()
            .split(|c: char| c == ',' || c.is_whitespace())
            .filter(|b| !b.is_empty())
        {
            let b = validate_bucket_name(raw)?;
            if !buckets.contains(&b) {
                buckets.push(b);
            }
        }
        if buckets.len() > 50 {
            return Err(AppError::invalid_input("Enter at most 50 bucket names."));
        }
        let secret = match &input.secret {
            Some(s) if !s.trim().is_empty() => {
                validate_secret(s)?;
                (SecretString::from(s.trim().to_string()), true)
            }
            _ => match input.id {
                Some(id) => {
                    let cfg = self.get(id)?;
                    let this = self.clone();
                    let secret = tokio::task::spawn_blocking(move || this.read_secret(&cfg))
                        .await
                        .map_err(|_| blocking_err())??;
                    (secret, false)
                }
                None => return Err(AppError::invalid_input("Enter the Secret Access Key.")),
            },
        };
        // A typed token replaces the stored one; removal applies only when
        // nothing was typed.
        let api_token = match input.api_token.as_deref().map(str::trim) {
            Some(t) if !t.is_empty() => {
                validate_api_token(t)?;
                ApiTokenChange::Replace(SecretString::from(t.to_string()))
            }
            _ if input.remove_api_token => ApiTokenChange::Remove,
            _ => ApiTokenChange::Keep,
        };
        Ok(Prepared {
            name,
            account_id,
            access_key_id,
            buckets,
            secret: secret.0,
            secret_is_new: secret.1,
            api_token,
        })
    }

    async fn run_test(
        &self,
        p: &Prepared,
        input: &ConnectionInput,
    ) -> Result<(TestResult, TestOutcome), AppError> {
        let params = self.params(
            &p.account_id,
            &p.access_key_id,
            SecretString::from(secrecy::ExposeSecret::expose_secret(&p.secret).to_string()),
            input.jurisdiction,
        );
        let client = R2Client::new(&params);
        let bucket_error = |name: &str, detail: Option<String>| {
            let mut e = AppError::new(
                ErrorCode::NoSuchBucket,
                format!(
                    "No bucket named \u{201c}{name}\u{201d} was found for this account, or this key cannot open it."
                ),
            )
            .with_name(name);
            e.detail = detail;
            e
        };
        let first = p.buckets.first().map(String::as_str);
        let outcome = match client.test_connection(first).await {
            Ok(o) => o,
            Err(R2Error::AccessDenied { detail }) if first.is_none() => {
                // R2 reports bad keys as InvalidAccessKeyId or
                // SignatureDoesNotMatch, so AccessDenied here means the key
                // works but is an object-level token.
                let mut e = AppError::new(
                    ErrorCode::AccessDenied,
                    "Your key was accepted, but it is not allowed to list buckets. Object Read & Write tokens can only open buckets by name: enter one or more bucket names above, separated by commas.",
                );
                e.detail = detail;
                return Err(e);
            }
            Err(R2Error::NoSuchBucket { detail }) | Err(R2Error::AccessDenied { detail }) => {
                return Err(bucket_error(first.unwrap_or_default(), detail));
            }
            Err(e) => return Err(e.to_app_error()),
        };
        // Check every other bucket the user entered.
        let listed: Vec<String> = match &outcome {
            TestOutcome::Buckets(b) => b.iter().map(|x| x.name.clone()).collect(),
            TestOutcome::ScopedBucket(_) => Vec::new(),
        };
        for b in p.buckets.iter().skip(1) {
            if listed.contains(b) {
                continue;
            }
            match client.list_page(b, "", None, Some(1)).await {
                Ok(_) => {}
                Err(R2Error::NoSuchBucket { detail }) | Err(R2Error::AccessDenied { detail }) => {
                    return Err(bucket_error(b, detail));
                }
                Err(e) => return Err(e.to_app_error()),
            }
        }
        let warning = account_id_looks_unusual(&p.account_id).then(|| {
            "Account IDs are usually 32 characters. Check that you copied the right value."
                .to_string()
        });
        let result = match &outcome {
            TestOutcome::Buckets(b) => TestResult {
                message: if b.len() == 1 {
                    "Connected. Found 1 bucket.".to_string()
                } else {
                    format!("Connected. Found {} buckets.", b.len())
                },
                bucket_scoped: false,
                buckets: b.clone(),
                warning,
            },
            TestOutcome::ScopedBucket(_) => TestResult {
                message: match p.buckets.as_slice() {
                    [one] => format!("Connected to {one}. This key can only open buckets by name."),
                    many => format!(
                        "Connected to {}. This key can only open buckets by name.",
                        many.join(", ")
                    ),
                },
                bucket_scoped: true,
                buckets: p
                    .buckets
                    .iter()
                    .map(|name| r2dock_r2::dto::BucketEntry {
                        name: name.clone(),
                        created: None,
                    })
                    .collect(),
                warning,
            },
        };
        Ok((result, outcome))
    }

    /// Test credentials without saving anything.
    pub async fn test(self: &Arc<Self>, input: &ConnectionInput) -> Result<TestResult, AppError> {
        let prepared = self.prepare(input).await?;
        let (result, _) = self.run_test(&prepared, input).await?;
        if let Some(id) = input.id {
            self.status.lock().insert(id, ConnectionStatus::Online);
            self.commit(|f| {
                if let Some(c) = f.connections.iter_mut().find(|c| c.id == id) {
                    c.last_tested_at = Some(Utc::now());
                }
                Ok(())
            })
            .await?;
        }
        Ok(result)
    }

    /// Test, then save a new connection or update an existing one.
    pub async fn save(
        self: &Arc<Self>,
        input: &ConnectionInput,
    ) -> Result<ConnectionSummary, AppError> {
        let prepared = self.prepare(input).await?;
        let (_, outcome) = self.run_test(&prepared, input).await?;
        let bucket_scoped = matches!(outcome, TestOutcome::ScopedBucket(_));
        let id = input.id.unwrap_or_else(Uuid::new_v4);
        let existing = input.id.map(|id| self.get(id)).transpose()?;

        // Store a new secret under a new reference before switching the
        // config to it, so the old secret keeps working if anything fails.
        // Session secrets are only published when the config commits.
        let keychain_ref = |l: Option<&SecretLocation>| match l {
            Some(SecretLocation::Keychain { reference }) => Some(reference.clone()),
            _ => None,
        };
        let old_reference = keychain_ref(existing.as_ref().map(|c| &c.secret));
        let old_token_reference =
            keychain_ref(existing.as_ref().and_then(|c| c.api_token.as_ref()));
        let mut new_references: Vec<String> = Vec::new();
        let mut session_secret = None;
        let secret_location = if prepared.secret_is_new {
            match self.keychain.clone() {
                Some(store) => {
                    let reference = self.store_new_reference(store, &prepared.secret).await?;
                    new_references.push(reference.clone());
                    SecretLocation::Keychain { reference }
                }
                None => {
                    session_secret = Some(SecretString::from(
                        secrecy::ExposeSecret::expose_secret(&prepared.secret).to_string(),
                    ));
                    SecretLocation::Session
                }
            }
        } else {
            existing
                .as_ref()
                .map(|c| c.secret.clone())
                .ok_or_else(|| AppError::invalid_input("Enter the Secret Access Key."))?
        };
        let mut session_token = None;
        let token_location = match &prepared.api_token {
            ApiTokenChange::Keep => existing.as_ref().and_then(|c| c.api_token.clone()),
            ApiTokenChange::Remove => None,
            ApiTokenChange::Replace(token) => match self.keychain.clone() {
                Some(store) => match self.store_new_reference(store, token).await {
                    Ok(reference) => {
                        new_references.push(reference.clone());
                        Some(SecretLocation::Keychain { reference })
                    }
                    Err(e) => {
                        for r in new_references {
                            self.discard_reference(r).await;
                        }
                        return Err(e);
                    }
                },
                None => {
                    session_token = Some(SecretString::from(
                        secrecy::ExposeSecret::expose_secret(token).to_string(),
                    ));
                    Some(SecretLocation::Session)
                }
            },
        };
        let token_replaced = !matches!(prepared.api_token, ApiTokenChange::Keep);

        let mut manual = existing
            .as_ref()
            .map(|c| c.manual_buckets.clone())
            .unwrap_or_default();
        for b in &prepared.buckets {
            if !manual.contains(b) {
                manual.push(b.clone());
            }
        }
        let cfg = ConnectionConfig {
            id,
            name: prepared.name.clone(),
            account_id: prepared.account_id.clone(),
            access_key_id: prepared.access_key_id.clone(),
            jurisdiction: input.jurisdiction,
            secret: secret_location,
            api_token: token_location,
            manual_buckets: manual,
            bucket_scoped,
            public_base_urls: existing
                .as_ref()
                .map(|c| c.public_base_urls.clone())
                .unwrap_or_default(),
            favorite_buckets: existing
                .as_ref()
                .map(|c| c.favorite_buckets.clone())
                .unwrap_or_default(),
            created_at: existing.as_ref().map_or_else(Utc::now, |c| c.created_at),
            last_tested_at: Some(Utc::now()),
        };
        let replaced = prepared.secret_is_new;
        let keychain_used = self.keychain.is_some();
        let committed = self
            .commit_then(
                |f| {
                    match f.connections.iter_mut().find(|c| c.id == id) {
                        Some(slot) => *slot = cfg,
                        None => f.connections.push(cfg),
                    }
                    f.active = Some(id);
                    // Replaced secrets are recorded for deletion in the same
                    // save that stops using them, so they are never forgotten.
                    if replaced && let Some(old) = old_reference {
                        f.add_cleanup(old);
                    }
                    if token_replaced && let Some(old) = old_token_reference {
                        f.add_cleanup(old);
                    }
                    Ok(())
                },
                || {
                    // Runs while the new config becomes live: publish the
                    // session values and drop clients built from old values
                    // in the same step.
                    if let Some(secret) = &session_secret {
                        let _ = self.session.set(&id.to_string(), secret);
                    } else if replaced && keychain_used {
                        // A session secret left from an earlier save is no longer used.
                        let _ = self.session.delete(&id.to_string());
                    }
                    if let Some(token) = &session_token {
                        let _ = self.session.set(&api_session_key(id), token);
                    } else if token_replaced {
                        let _ = self.session.delete(&api_session_key(id));
                    }
                    self.invalidate(id);
                },
            )
            .await;
        if let Err(e) = committed {
            // Undo the secrets written above; the old config is still live
            // and nothing new was published, so cached clients stay valid.
            for reference in new_references {
                self.discard_reference(reference).await;
            }
            return Err(e);
        }
        self.missing.lock().remove(&id);
        self.status.lock().insert(id, ConnectionStatus::Online);
        self.cleanup_secrets().await;
        self.summary_of(id)
    }

    /// Delete a secret that was written but never committed. If that fails,
    /// remember the reference so a later cleanup removes it.
    async fn discard_reference(&self, reference: String) {
        let Some(store) = self.keychain.clone() else {
            return;
        };
        let r = reference.clone();
        let deleted = tokio::task::spawn_blocking(move || store.delete(&r))
            .await
            .is_ok_and(|r| r.is_ok());
        if !deleted {
            self.unsaved_cleanup.lock().push(reference);
            // The config save just failed, so record the reference in its
            // own file: it must not be forgotten if R2Dock quits now.
            self.persist_unsaved_cleanup().await;
        }
    }

    /// Remove a connection and delete its stored secret. The connection is
    /// removed from the saved config together with a record that its secret
    /// must be deleted, and the secret is deleted after that. If the
    /// credential store cannot delete it now, the record stays and deletion
    /// is retried at the next start or connection change.
    pub async fn remove(self: &Arc<Self>, id: Uuid) -> Result<(), AppError> {
        self.commit(|f| {
            let cfg = f
                .connections
                .iter()
                .find(|c| c.id == id)
                .cloned()
                .ok_or_else(|| AppError::not_found("Connection not found."))?;
            f.connections.retain(|c| c.id != id);
            if f.active == Some(id) {
                f.active = f.connections.first().map(|c| c.id);
            }
            if let SecretLocation::Keychain { reference } = cfg.secret {
                f.add_cleanup(reference);
            }
            if let Some(SecretLocation::Keychain { reference }) = cfg.api_token {
                f.add_cleanup(reference);
            }
            Ok(())
        })
        .await?;
        let _ = self.session.delete(&id.to_string());
        let _ = self.session.delete(&api_session_key(id));
        self.invalidate(id);
        self.status.lock().remove(&id);
        self.missing.lock().remove(&id);
        self.cleanup_secrets().await;
        Ok(())
    }

    pub async fn set_active(&self, id: Uuid) -> Result<(), AppError> {
        self.commit(|f| {
            if !f.connections.iter().any(|c| c.id == id) {
                return Err(AppError::not_found("Connection not found."));
            }
            f.active = Some(id);
            Ok(())
        })
        .await
    }

    async fn update<F>(&self, id: Uuid, f: F) -> Result<ConnectionSummary, AppError>
    where
        F: FnOnce(&mut ConnectionConfig) -> Result<(), AppError>,
    {
        self.commit(|file| {
            let cfg = file
                .connections
                .iter_mut()
                .find(|c| c.id == id)
                .ok_or_else(|| AppError::not_found("Connection not found."))?;
            f(cfg)
        })
        .await?;
        self.summary_of(id)
    }

    pub async fn add_bucket(&self, id: Uuid, bucket: &str) -> Result<ConnectionSummary, AppError> {
        let bucket = validate_bucket_name(bucket)?;
        self.update(id, |c| {
            if !c.manual_buckets.contains(&bucket) {
                c.manual_buckets.push(bucket);
            }
            Ok(())
        })
        .await
    }

    pub async fn remove_bucket(
        &self,
        id: Uuid,
        bucket: &str,
    ) -> Result<ConnectionSummary, AppError> {
        self.update(id, |c| {
            c.manual_buckets.retain(|b| b != bucket);
            c.favorite_buckets.retain(|b| b != bucket);
            c.public_base_urls.remove(bucket);
            Ok(())
        })
        .await
    }

    pub async fn set_public_url(
        &self,
        id: Uuid,
        bucket: &str,
        url: Option<&str>,
    ) -> Result<ConnectionSummary, AppError> {
        let bucket = validate_bucket_name(bucket)?;
        let url = match url.map(str::trim) {
            Some(u) if !u.is_empty() => Some(validate_public_base_url(u)?),
            _ => None,
        };
        self.update(id, |c| {
            match url {
                Some(u) => {
                    c.public_base_urls.insert(bucket, u);
                }
                None => {
                    c.public_base_urls.remove(&bucket);
                }
            }
            Ok(())
        })
        .await
    }

    pub async fn toggle_favorite(
        &self,
        id: Uuid,
        bucket: &str,
    ) -> Result<ConnectionSummary, AppError> {
        let bucket = validate_bucket_name(bucket)?;
        self.update(id, |c| {
            if let Some(i) = c.favorite_buckets.iter().position(|b| *b == bucket) {
                c.favorite_buckets.remove(i);
            } else {
                c.favorite_buckets.push(bucket);
            }
            Ok(())
        })
        .await
    }

    pub fn public_base_url(&self, id: Uuid, bucket: &str) -> Option<String> {
        self.file
            .read()
            .connections
            .iter()
            .find(|c| c.id == id)
            .and_then(|c| c.public_base_urls.get(bucket).cloned())
    }
}

struct Prepared {
    name: String,
    account_id: String,
    access_key_id: String,
    buckets: Vec<String>,
    secret: SecretString,
    secret_is_new: bool,
    api_token: ApiTokenChange,
}

#[cfg(test)]
mod tests;
