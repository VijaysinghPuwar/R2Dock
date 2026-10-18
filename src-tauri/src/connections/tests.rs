use super::*;
use r2dock_core::connections::Jurisdiction;
use r2dock_testkit::{ACCESS_KEY, LocalS3, SECRET_KEY};

fn input(secret: Option<&str>, id: Option<Uuid>) -> ConnectionInput {
    ConnectionInput {
        id,
        name: "Personal R2".into(),
        account_id: "a1b2c3d4e5f60718293a4b5c6d7e9f0e".into(),
        access_key_id: ACCESS_KEY.into(),
        secret: secret.map(str::to_string),
        bucket: None,
        jurisdiction: Jurisdiction::Default,
        api_token: None,
        remove_api_token: false,
    }
}

async fn service(s3: &LocalS3, dir: &std::path::Path, keychain: bool) -> Arc<ConnectionService> {
    let store: Option<Arc<dyn SecretStore>> =
        keychain.then(|| Arc::new(MemoryStore::new()) as Arc<dyn SecretStore>);
    Arc::new(ConnectionService::load(
        dir.join("connections.json"),
        store,
        Some(s3.endpoint.clone()),
    ))
}

#[tokio::test]
async fn save_edit_replace_remove_lifecycle() {
    let s3 = LocalS3::start(&["alpha", "beta"]).await;
    let dir = tempfile::tempdir().unwrap();
    let svc = service(&s3, dir.path(), true).await;

    let t = svc.test(&input(Some(SECRET_KEY), None)).await.unwrap();
    assert_eq!(t.message, "Connected. Found 2 buckets.");
    assert!(t.warning.is_none());

    let saved = svc.save(&input(Some(SECRET_KEY), None)).await.unwrap();
    assert!(saved.active);
    assert_eq!(saved.secret_storage, SecretStorage::Keychain);
    assert_eq!(saved.status, ConnectionStatus::Online);

    // The secret never reaches the config file or the summary.
    let on_disk = std::fs::read_to_string(dir.path().join("connections.json")).unwrap();
    assert!(!on_disk.contains(SECRET_KEY));
    assert!(
        !serde_json::to_string(&svc.list())
            .unwrap()
            .contains(SECRET_KEY)
    );
    assert!(!saved.access_key_hint.contains(ACCESS_KEY));

    // A client can be built from the stored secret.
    let client = svc.client(saved.id).await.unwrap();
    assert_eq!(client.list_buckets().await.unwrap().len(), 2);

    // Edit without a secret or access key keeps the stored ones.
    let mut edit = input(None, Some(saved.id));
    edit.name = "Renamed".into();
    edit.access_key_id = String::new();
    let edited = svc.save(&edit).await.unwrap();
    assert_eq!(edited.name, "Renamed");
    svc.test(&input(None, Some(saved.id))).await.unwrap();

    // A wrong replacement secret is rejected and the old one keeps working.
    let bad = svc
        .save(&input(Some("wrong-secret"), Some(saved.id)))
        .await
        .unwrap_err();
    assert!(
        matches!(
            bad.code,
            ErrorCode::SignatureMismatch | ErrorCode::AccessDenied
        ),
        "{bad:?}"
    );
    assert!(!bad.message.contains("wrong-secret"));
    svc.client(saved.id).await.unwrap();

    svc.remove(saved.id).await.unwrap();
    assert!(svc.list().is_empty());
}

#[tokio::test]
async fn session_only_when_keychain_unavailable() {
    let s3 = LocalS3::start(&["alpha"]).await;
    let dir = tempfile::tempdir().unwrap();
    let svc = service(&s3, dir.path(), false).await;
    let saved = svc.save(&input(Some(SECRET_KEY), None)).await.unwrap();
    assert_eq!(saved.secret_storage, SecretStorage::Session);
    svc.client(saved.id).await.unwrap();

    // After a restart the session secret is gone.
    let svc2 = service(&s3, dir.path(), false).await;
    let list = svc2.list();
    assert_eq!(list[0].secret_storage, SecretStorage::Missing);
    assert_eq!(list[0].status, ConnectionStatus::NeedsAttention);
    let err = svc2.client(saved.id).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::CredentialMissing);
}

#[tokio::test]
async fn secret_removed_outside_app_is_reported() {
    let s3 = LocalS3::start(&["alpha"]).await;
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(MemoryStore::new());
    let svc = Arc::new(ConnectionService::load(
        dir.path().join("connections.json"),
        Some(store.clone() as Arc<dyn SecretStore>),
        Some(s3.endpoint.clone()),
    ));
    let saved = svc.save(&input(Some(SECRET_KEY), None)).await.unwrap();
    let SecretLocation::Keychain { reference } = svc.get(saved.id).unwrap().secret else {
        panic!()
    };
    store.delete(&reference).unwrap();
    svc.clients.lock().clear();
    let err = svc.client(saved.id).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::CredentialMissing);
    assert!(err.message.contains("removed outside R2Dock"));
    assert_eq!(svc.list()[0].secret_storage, SecretStorage::Missing);
}

#[tokio::test]
async fn several_buckets_can_be_entered() {
    let s3 = LocalS3::start(&["alpha", "beta"]).await;
    let dir = tempfile::tempdir().unwrap();
    let svc = service(&s3, dir.path(), true).await;

    let mut i = input(Some(SECRET_KEY), None);
    i.bucket = Some("alpha, missing-bucket".into());
    let err = svc.test(&i).await.unwrap_err();
    assert!(err.message.contains("missing-bucket"), "{err:?}");

    let mut i = input(Some(SECRET_KEY), None);
    i.bucket = Some(" alpha,beta\nalpha ".into());
    let saved = svc.save(&i).await.unwrap();
    assert_eq!(
        saved.manual_buckets,
        vec!["alpha".to_string(), "beta".to_string()]
    );

    let mut i = input(Some(SECRET_KEY), None);
    i.bucket = Some("alpha, Not_Valid".into());
    assert_eq!(
        svc.test(&i).await.unwrap_err().code,
        ErrorCode::InvalidInput
    );
}

#[tokio::test]
async fn validation_and_scoped_buckets() {
    let s3 = LocalS3::start(&["alpha"]).await;
    let dir = tempfile::tempdir().unwrap();
    let svc = service(&s3, dir.path(), true).await;
    let mut i = input(Some(SECRET_KEY), None);
    i.account_id = "evil.com/x".into();
    assert_eq!(
        svc.test(&i).await.unwrap_err().code,
        ErrorCode::InvalidInput
    );
    let mut i = input(None, None);
    i.secret = None;
    assert_eq!(
        svc.test(&i).await.unwrap_err().code,
        ErrorCode::InvalidInput
    );

    let mut i = input(Some(SECRET_KEY), None);
    i.bucket = Some("missing-bucket".into());
    let err = svc.test(&i).await.unwrap_err();
    assert!(err.message.contains("missing-bucket"), "{err:?}");

    let mut i = input(Some(SECRET_KEY), None);
    i.account_id = "abc123".into();
    let t = svc.test(&i).await.unwrap();
    assert!(t.warning.is_some());

    let saved = svc.save(&input(Some(SECRET_KEY), None)).await.unwrap();
    let s = svc
        .set_public_url(saved.id, "alpha", Some("https://media.example.com/"))
        .await
        .unwrap();
    assert_eq!(
        s.public_base_urls.get("alpha").map(String::as_str),
        Some("https://media.example.com")
    );
    assert!(
        svc.set_public_url(saved.id, "alpha", Some("javascript:alert(1)"))
            .await
            .is_err()
    );
    let s = svc.toggle_favorite(saved.id, "alpha").await.unwrap();
    assert_eq!(s.favorite_buckets, vec!["alpha".to_string()]);
    let s = svc.toggle_favorite(saved.id, "alpha").await.unwrap();
    assert!(s.favorite_buckets.is_empty());
}

/// A credential store whose deletes can be made to fail, and which
/// remembers every reference written to it.
#[derive(Default)]
struct FlakyStore {
    inner: MemoryStore,
    fail_delete: std::sync::atomic::AtomicBool,
    written: Mutex<Vec<String>>,
}

impl FlakyStore {
    fn stored(&self) -> Vec<String> {
        self.written
            .lock()
            .iter()
            .filter(|r| self.inner.get(r).is_ok())
            .cloned()
            .collect()
    }
    fn set_fail_delete(&self, fail: bool) {
        self.fail_delete
            .store(fail, std::sync::atomic::Ordering::SeqCst);
    }
}

impl SecretStore for FlakyStore {
    fn backend(&self) -> r2dock_secrets::Backend {
        r2dock_secrets::Backend::native()
    }
    fn set(&self, reference: &str, secret: &SecretString) -> Result<(), SecretError> {
        self.written.lock().push(reference.to_string());
        self.inner.set(reference, secret)
    }
    fn get(&self, reference: &str) -> Result<SecretString, SecretError> {
        self.inner.get(reference)
    }
    fn delete(&self, reference: &str) -> Result<(), SecretError> {
        if self.fail_delete.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(SecretError::Unavailable);
        }
        self.inner.delete(reference)
    }
}

fn with_store(
    s3: &LocalS3,
    path: &std::path::Path,
    store: Option<Arc<FlakyStore>>,
) -> Arc<ConnectionService> {
    Arc::new(ConnectionService::load(
        path.to_path_buf(),
        store.map(|s| s as Arc<dyn SecretStore>),
        Some(s3.endpoint.clone()),
    ))
}

fn reference_of(svc: &ConnectionService, id: Uuid) -> String {
    match svc.get(id).unwrap().secret {
        SecretLocation::Keychain { reference } => reference,
        SecretLocation::Session => panic!("expected a keychain secret"),
    }
}

fn fail_writes(svc: &ConnectionService, fail: bool) {
    svc.fail_writes
        .store(fail, std::sync::atomic::Ordering::SeqCst);
}

#[tokio::test]
async fn removal_while_store_unavailable_keeps_a_cleanup_record() {
    let s3 = LocalS3::start(&["alpha"]).await;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("connections.json");
    let store = Arc::new(FlakyStore::default());
    let svc = with_store(&s3, &path, Some(store.clone()));
    let saved = svc.save(&input(Some(SECRET_KEY), None)).await.unwrap();
    let reference = reference_of(&svc, saved.id);

    // Next start: the credential store is unavailable.
    let offline = with_store(&s3, &path, None);
    offline.remove(saved.id).await.unwrap();
    assert!(offline.list().is_empty());
    assert_eq!(offline.pending_secret_cleanup(), 1);
    let on_disk = std::fs::read_to_string(&path).unwrap();
    assert!(on_disk.contains(&reference), "cleanup record is saved");
    assert!(!on_disk.contains(SECRET_KEY));
    assert_eq!(store.stored(), vec![reference.clone()]);

    // The start after that: the store works again and the secret goes.
    let online = with_store(&s3, &path, Some(store.clone()));
    assert_eq!(online.cleanup_secrets().await, 0);
    assert!(store.stored().is_empty());
    assert!(!std::fs::read_to_string(&path).unwrap().contains(&reference));
    // Repeating is harmless.
    assert_eq!(online.cleanup_secrets().await, 0);
}

#[tokio::test]
async fn failed_secret_deletes_are_retried() {
    let s3 = LocalS3::start(&["alpha"]).await;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("connections.json");
    let store = Arc::new(FlakyStore::default());
    let svc = with_store(&s3, &path, Some(store.clone()));
    let saved = svc.save(&input(Some(SECRET_KEY), None)).await.unwrap();
    let old = reference_of(&svc, saved.id);

    // Replace the secret while deletes fail: the old entry is kept on
    // record, and the new one is used.
    store.set_fail_delete(true);
    svc.save(&input(Some(SECRET_KEY), Some(saved.id)))
        .await
        .unwrap();
    let new = reference_of(&svc, saved.id);
    assert_ne!(old, new);
    assert_eq!(svc.pending_secret_cleanup(), 1);
    svc.client(saved.id).await.unwrap();

    // Remove while deletes still fail: both are on record.
    svc.remove(saved.id).await.unwrap();
    assert_eq!(svc.pending_secret_cleanup(), 2);
    assert_eq!(store.stored().len(), 2);

    store.set_fail_delete(false);
    assert_eq!(svc.cleanup_secrets().await, 0);
    assert!(store.stored().is_empty());
}

#[tokio::test]
async fn session_fallback_records_the_replaced_keychain_secret() {
    let s3 = LocalS3::start(&["alpha"]).await;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("connections.json");
    let store = Arc::new(FlakyStore::default());
    let svc = with_store(&s3, &path, Some(store.clone()));
    let saved = svc.save(&input(Some(SECRET_KEY), None)).await.unwrap();
    let old = reference_of(&svc, saved.id);

    let offline = with_store(&s3, &path, None);
    let s = offline
        .save(&input(Some(SECRET_KEY), Some(saved.id)))
        .await
        .unwrap();
    assert_eq!(s.secret_storage, SecretStorage::Session);
    assert_eq!(offline.pending_secret_cleanup(), 1);

    let online = with_store(&s3, &path, Some(store.clone()));
    assert_eq!(online.cleanup_secrets().await, 0);
    assert!(store.inner.get(&old).is_err());
}

#[tokio::test]
async fn failed_saves_leave_the_committed_state() {
    let s3 = LocalS3::start(&["alpha"]).await;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("connections.json");
    let store = Arc::new(FlakyStore::default());
    let svc = with_store(&s3, &path, Some(store.clone()));
    let saved = svc.save(&input(Some(SECRET_KEY), None)).await.unwrap();
    let reference = reference_of(&svc, saved.id);
    svc.client(saved.id).await.unwrap();
    let disk_before = std::fs::read_to_string(&path).unwrap();

    fail_writes(&svc, true);
    assert!(
        svc.set_public_url(saved.id, "alpha", Some("https://media.example.com"))
            .await
            .is_err()
    );
    assert_eq!(svc.public_base_url(saved.id, "alpha"), None);
    assert!(svc.toggle_favorite(saved.id, "alpha").await.is_err());
    assert!(svc.list()[0].favorite_buckets.is_empty());

    // Replacing the secret fails: the old reference stays live, the new
    // secret is deleted again, and the cached client is kept.
    let mut edit = input(Some(SECRET_KEY), Some(saved.id));
    edit.name = "Renamed".into();
    assert!(svc.save(&edit).await.is_err());
    assert_eq!(svc.list()[0].name, "Personal R2");
    assert_eq!(reference_of(&svc, saved.id), reference);
    assert_eq!(store.stored(), vec![reference.clone()]);
    assert!(svc.clients.lock().contains_key(&saved.id));

    // Removing fails: the connection and its secret stay.
    assert!(svc.remove(saved.id).await.is_err());
    assert_eq!(svc.list().len(), 1);
    assert_eq!(store.stored(), vec![reference.clone()]);
    assert_eq!(svc.pending_secret_cleanup(), 0);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), disk_before);

    // A secret that cannot be deleted after a failed save is remembered
    // and recorded by the next successful save.
    store.set_fail_delete(true);
    assert!(svc.save(&edit).await.is_err());
    assert_eq!(svc.pending_secret_cleanup(), 1);
    fail_writes(&svc, false);
    store.set_fail_delete(false);
    let s = svc.save(&edit).await.unwrap();
    assert_eq!(s.name, "Renamed");
    assert_eq!(svc.cleanup_secrets().await, 0);
    assert_eq!(store.stored(), vec![reference_of(&svc, saved.id)]);
    assert!(!svc.clients.lock().contains_key(&saved.id));
}

#[tokio::test]
async fn failed_session_save_restores_the_session_secret() {
    let s3 = LocalS3::start(&["alpha"]).await;
    let dir = tempfile::tempdir().unwrap();
    let svc = service(&s3, dir.path(), false).await;
    fail_writes(&svc, true);
    assert!(svc.save(&input(Some(SECRET_KEY), None)).await.is_err());
    assert!(svc.list().is_empty());
    fail_writes(&svc, false);
    let saved = svc.save(&input(Some(SECRET_KEY), None)).await.unwrap();
    fail_writes(&svc, true);
    assert!(
        svc.save(&input(Some(SECRET_KEY), Some(saved.id)))
            .await
            .is_err()
    );
    assert_eq!(svc.list()[0].secret_storage, SecretStorage::Session);
    svc.clients.lock().clear();
    svc.client(saved.id).await.unwrap();
}

const TOKEN_A: &str = "fake-api-token-aaaa";
const TOKEN_B: &str = "fake-api-token-bbbb";

fn with_token(token: Option<&str>, id: Option<Uuid>) -> ConnectionInput {
    let mut i = input(if id.is_some() { None } else { Some(SECRET_KEY) }, id);
    i.api_token = token.map(str::to_string);
    i
}

async fn token_of(svc: &Arc<ConnectionService>, id: Uuid) -> Option<String> {
    svc.api_token(id)
        .await
        .unwrap()
        .map(|s| secrecy::ExposeSecret::expose_secret(&s).to_string())
}

fn token_reference(svc: &ConnectionService, id: Uuid) -> String {
    match svc.get(id).unwrap().api_token {
        Some(SecretLocation::Keychain { reference }) => reference,
        other => panic!("expected a keychain token, got {other:?}"),
    }
}

#[tokio::test]
async fn api_token_is_stored_like_the_secret() {
    let s3 = LocalS3::start(&["alpha"]).await;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("connections.json");
    let store = Arc::new(FlakyStore::default());
    let svc = with_store(&s3, &path, Some(store.clone()));

    let saved = svc.save(&with_token(Some(TOKEN_A), None)).await.unwrap();
    assert!(saved.has_api_token);
    assert_eq!(token_of(&svc, saved.id).await.as_deref(), Some(TOKEN_A));
    let first = token_reference(&svc, saved.id);
    let on_disk = std::fs::read_to_string(&path).unwrap();
    assert!(!on_disk.contains(TOKEN_A));
    assert!(
        !serde_json::to_string(&svc.list())
            .unwrap()
            .contains(TOKEN_A)
    );

    // Editing without a token keeps it.
    let kept = svc.save(&with_token(None, Some(saved.id))).await.unwrap();
    assert!(kept.has_api_token);
    assert_eq!(token_reference(&svc, saved.id), first);

    // Replacing stores a new entry and deletes the old one.
    svc.save(&with_token(Some(TOKEN_B), Some(saved.id)))
        .await
        .unwrap();
    assert_eq!(token_of(&svc, saved.id).await.as_deref(), Some(TOKEN_B));
    let second = token_reference(&svc, saved.id);
    assert_ne!(first, second);
    assert!(store.inner.get(&first).is_err());

    // Removing deletes it.
    let mut remove = with_token(None, Some(saved.id));
    remove.remove_api_token = true;
    let s = svc.save(&remove).await.unwrap();
    assert!(!s.has_api_token);
    assert_eq!(token_of(&svc, saved.id).await, None);
    assert!(store.inner.get(&second).is_err());

    // Removing the connection deletes both its secret and its token.
    svc.save(&with_token(Some(TOKEN_A), Some(saved.id)))
        .await
        .unwrap();
    assert_eq!(store.stored().len(), 2);
    svc.remove(saved.id).await.unwrap();
    assert!(store.stored().is_empty());
    assert_eq!(svc.pending_secret_cleanup(), 0);
}

#[tokio::test]
async fn invalid_api_token_is_refused_before_anything_is_stored() {
    let s3 = LocalS3::start(&["alpha"]).await;
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(FlakyStore::default());
    let svc = with_store(
        &s3,
        &dir.path().join("connections.json"),
        Some(store.clone()),
    );
    let err = svc
        .save(&with_token(Some("has space"), None))
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidInput);
    assert!(!err.message.contains("has space"));
    assert!(store.stored().is_empty());
}

#[tokio::test]
async fn session_api_token_is_published_only_after_commit() {
    let s3 = LocalS3::start(&["alpha"]).await;
    let dir = tempfile::tempdir().unwrap();
    let svc = service(&s3, dir.path(), false).await;
    let saved = svc.save(&with_token(Some(TOKEN_A), None)).await.unwrap();
    assert!(saved.has_api_token);
    assert_eq!(token_of(&svc, saved.id).await.as_deref(), Some(TOKEN_A));

    // A failed save publishes nothing: the old values stay in use and the
    // cached client stays valid.
    svc.client(saved.id).await.unwrap();
    fail_writes(&svc, true);
    assert!(
        svc.save(&with_token(Some(TOKEN_B), Some(saved.id)))
            .await
            .is_err()
    );
    assert_eq!(token_of(&svc, saved.id).await.as_deref(), Some(TOKEN_A));
    assert!(svc.clients.lock().contains_key(&saved.id));
    fail_writes(&svc, false);

    // A successful save publishes the new value and drops the old client
    // in the same step.
    let generation = svc.generation_of(saved.id);
    svc.save(&with_token(Some(TOKEN_B), Some(saved.id)))
        .await
        .unwrap();
    assert_eq!(token_of(&svc, saved.id).await.as_deref(), Some(TOKEN_B));
    assert!(!svc.clients.lock().contains_key(&saved.id));
    assert!(svc.generation_of(saved.id) > generation);

    // A restart forgets session tokens.
    let svc2 = service(&s3, dir.path(), false).await;
    assert!(!svc2.list()[0].has_api_token);
}

#[tokio::test]
async fn orphaned_secret_survives_a_restart_until_deleted() {
    let s3 = LocalS3::start(&["alpha"]).await;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("connections.json");
    let store = Arc::new(FlakyStore::default());
    let svc = with_store(&s3, &path, Some(store.clone()));

    // The config save fails and so does the compensating delete.
    fail_writes(&svc, true);
    store.set_fail_delete(true);
    assert!(svc.save(&with_token(Some(TOKEN_A), None)).await.is_err());
    let orphans = store.stored();
    assert_eq!(orphans.len(), 2, "secret and token were written");
    assert_eq!(svc.pending_secret_cleanup(), 2);
    let side = std::fs::read_to_string(cleanup_path(&path)).unwrap();
    for r in &orphans {
        assert!(side.contains(r.as_str()));
    }
    assert!(!side.contains(SECRET_KEY) && !side.contains(TOKEN_A));
    drop(svc);

    // Next start: the record is still known and the cleanup deletes them.
    store.set_fail_delete(false);
    let next = with_store(&s3, &path, Some(store.clone()));
    assert_eq!(next.pending_secret_cleanup(), 2);
    assert_eq!(next.cleanup_secrets().await, 0);
    assert!(store.stored().is_empty());
    assert!(!cleanup_path(&path).exists());
}

#[tokio::test]
async fn orphan_record_is_folded_into_the_next_saved_config() {
    let s3 = LocalS3::start(&["alpha"]).await;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("connections.json");
    let store = Arc::new(FlakyStore::default());
    let svc = with_store(&s3, &path, Some(store.clone()));
    fail_writes(&svc, true);
    store.set_fail_delete(true);
    assert!(svc.save(&input(Some(SECRET_KEY), None)).await.is_err());
    assert!(cleanup_path(&path).exists());

    // The next save works but deletes still fail: the reference moves into
    // the config and the side file goes away.
    fail_writes(&svc, false);
    svc.save(&input(Some(SECRET_KEY), None)).await.unwrap();
    assert!(!cleanup_path(&path).exists());
    let orphan = store.stored()[0].clone();
    assert!(std::fs::read_to_string(&path).unwrap().contains(&orphan));
}
