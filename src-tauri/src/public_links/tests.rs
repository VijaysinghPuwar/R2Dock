use std::time::Duration;

use r2dock_core::connections::Jurisdiction;
use r2dock_core::public_domain::PublicLinkSource;
use r2dock_secrets::{MemoryStore, SecretStore};
use r2dock_testkit::cloudflare::{
    AUTH_ERROR, CUSTOM_DOMAINS_EMPTY, CUSTOM_DOMAINS_OK, MANAGED_DOMAIN_OK, MockCloudflare, Reply,
};
use r2dock_testkit::{ACCESS_KEY, LocalS3, SECRET_KEY};

use super::*;
use crate::dto::ConnectionInput;

const TOKEN: &str = "fake-api-token-value-0000";
const KEY: &str = "Audiobook G/caf\u{e9} 01.mp3";
const ENCODED: &str = "Audiobook%20G/caf%C3%A9%2001.mp3";

fn input(api_token: Option<&str>) -> ConnectionInput {
    ConnectionInput {
        id: None,
        name: "Personal R2".into(),
        account_id: "00000000000000000000000000000000".into(),
        access_key_id: ACCESS_KEY.into(),
        secret: Some(SECRET_KEY.into()),
        bucket: None,
        jurisdiction: Jurisdiction::Default,
        api_token: api_token.map(str::to_string),
        remove_api_token: false,
    }
}

struct Setup {
    _s3: LocalS3,
    _dir: tempfile::TempDir,
    svc: Arc<ConnectionService>,
    id: Uuid,
}

async fn setup(api_token: Option<&str>) -> Setup {
    let s3 = LocalS3::start(&["media-library"]).await;
    let dir = tempfile::tempdir().unwrap();
    let svc = Arc::new(ConnectionService::load(
        dir.path().join("connections.json"),
        Some(Arc::new(MemoryStore::new()) as Arc<dyn SecretStore>),
        Some(s3.endpoint.clone()),
    ));
    let id = svc.save(&input(api_token)).await.unwrap().id;
    Setup {
        _s3: s3,
        _dir: dir,
        svc,
        id,
    }
}

fn links(mock: &MockCloudflare) -> PublicLinks {
    PublicLinks::new(Some(
        CfApiClient::with_base_url(&mock.base, Duration::from_secs(5)).unwrap(),
    ))
}

#[tokio::test]
async fn manual_url_wins_and_needs_no_api_call() {
    let s = setup(Some(TOKEN)).await;
    let mock = MockCloudflare::start(
        Reply::Json(200, CUSTOM_DOMAINS_OK),
        Reply::Json(200, MANAGED_DOMAIN_OK),
    )
    .await;
    s.svc
        .set_public_url(s.id, "media-library", Some("https://cdn.example.com"))
        .await
        .unwrap();
    let link = links(&mock)
        .resolve(&s.svc, s.id, "media-library", KEY)
        .await
        .unwrap();
    assert_eq!(
        link.url.as_deref(),
        Some(format!("https://cdn.example.com/{ENCODED}").as_str())
    );
    assert_eq!(link.source, Some(PublicLinkSource::PublicUrl));
    assert!(mock.requests().is_empty());
}

#[tokio::test]
async fn custom_domain_is_found_and_cached() {
    let s = setup(Some(TOKEN)).await;
    let mock = MockCloudflare::start(
        Reply::Json(200, CUSTOM_DOMAINS_OK),
        Reply::Json(200, MANAGED_DOMAIN_OK),
    )
    .await;
    let links = links(&mock);
    let link = links
        .resolve(&s.svc, s.id, "media-library", KEY)
        .await
        .unwrap();
    assert_eq!(link.status, PublicLinkStatus::Found);
    assert_eq!(link.source, Some(PublicLinkSource::CustomDomain));
    assert_eq!(
        link.url.as_deref(),
        Some(format!("https://media.example.com/{ENCODED}").as_str())
    );
    // A second file in the same bucket reuses the lookup.
    links
        .resolve(&s.svc, s.id, "media-library", "other.txt")
        .await
        .unwrap();
    assert_eq!(mock.requests().len(), 2, "one custom and one managed call");

    // Changing the connection drops the cached result.
    let mut edit = input(None);
    edit.id = Some(s.id);
    edit.secret = None;
    s.svc.save(&edit).await.unwrap();
    links
        .resolve(&s.svc, s.id, "media-library", KEY)
        .await
        .unwrap();
    assert_eq!(mock.requests().len(), 4);
}

#[tokio::test]
async fn r2_dev_is_used_without_a_custom_domain() {
    let s = setup(Some(TOKEN)).await;
    let mock = MockCloudflare::start(
        Reply::Json(200, CUSTOM_DOMAINS_EMPTY),
        Reply::Json(200, MANAGED_DOMAIN_OK),
    )
    .await;
    let link = links(&mock)
        .resolve(&s.svc, s.id, "media-library", KEY)
        .await
        .unwrap();
    assert_eq!(link.source, Some(PublicLinkSource::R2Dev));
    assert_eq!(
        link.url.as_deref(),
        Some(format!("https://pub-00000000000000000000000000000000.r2.dev/{ENCODED}").as_str())
    );
}

#[tokio::test]
async fn statuses_without_a_link() {
    // No token and no manual URL: nothing is fetched.
    let s = setup(None).await;
    let mock = MockCloudflare::start(
        Reply::Json(200, CUSTOM_DOMAINS_OK),
        Reply::Json(200, MANAGED_DOMAIN_OK),
    )
    .await;
    let link = links(&mock)
        .resolve(&s.svc, s.id, "media-library", KEY)
        .await
        .unwrap();
    assert_eq!(link.status, PublicLinkStatus::NotConfigured);
    assert!(mock.requests().is_empty());

    let s = setup(Some(TOKEN)).await;
    let cases = [
        (Reply::Json(403, AUTH_ERROR), PublicLinkStatus::TokenDenied),
        (Reply::Json(401, AUTH_ERROR), PublicLinkStatus::TokenDenied),
        (
            Reply::Json(200, CUSTOM_DOMAINS_EMPTY),
            PublicLinkStatus::NoDomain,
        ),
        (Reply::Json(500, "{}"), PublicLinkStatus::Unavailable),
    ];
    for (custom, want) in cases {
        let managed = match custom {
            Reply::Json(200, _) => Reply::Json(403, AUTH_ERROR),
            other => other,
        };
        let mock = MockCloudflare::start(custom, managed).await;
        let link = links(&mock)
            .resolve(&s.svc, s.id, "media-library", KEY)
            .await
            .unwrap();
        assert_eq!(link.status, want);
        assert_eq!(link.url, None);
    }
}

#[tokio::test]
async fn token_removed_outside_the_app_is_reported() {
    let s3 = LocalS3::start(&["media-library"]).await;
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(MemoryStore::new());
    let svc = Arc::new(ConnectionService::load(
        dir.path().join("connections.json"),
        Some(store.clone() as Arc<dyn SecretStore>),
        Some(s3.endpoint.clone()),
    ));
    let id = svc.save(&input(Some(TOKEN))).await.unwrap().id;
    let Some(r2dock_core::connections::SecretLocation::Keychain { reference }) =
        svc.get(id).unwrap().api_token
    else {
        panic!("expected a keychain token")
    };
    store.delete(&reference).unwrap();
    let mock = MockCloudflare::start(
        Reply::Json(200, CUSTOM_DOMAINS_OK),
        Reply::Json(200, MANAGED_DOMAIN_OK),
    )
    .await;
    let link = links(&mock)
        .resolve(&svc, id, "media-library", KEY)
        .await
        .unwrap();
    assert_eq!(link.status, PublicLinkStatus::TokenMissing);
    assert!(mock.requests().is_empty());
}
