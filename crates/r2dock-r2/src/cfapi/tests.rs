use std::time::Duration;

use r2dock_testkit::cloudflare::{
    AUTH_ERROR, CUSTOM_DOMAINS_OK, MANAGED_DOMAIN_OK, MockCloudflare, Reply,
};
use tokio::net::TcpListener;

use super::*;

const ACCOUNT: &str = "00000000000000000000000000000000";
const TOKEN: &str = "fake-api-token-value-0000";
const R2_DEV: &str = "pub-00000000000000000000000000000000.r2.dev";

fn token() -> SecretString {
    SecretString::from(TOKEN.to_string())
}

fn client(mock: &MockCloudflare) -> CfApiClient {
    CfApiClient::with_base_url(&mock.base, Duration::from_secs(5)).unwrap()
}

#[test]
fn parses_documented_custom_response() {
    let domains = parse_custom(CUSTOM_DOMAINS_OK.as_bytes()).unwrap();
    assert_eq!(
        domains,
        vec![
            CustomDomain {
                domain: "prefix.example-domain.one.com".into(),
                enabled: false,
                active: false
            },
            CustomDomain {
                domain: "media.example.com".into(),
                enabled: true,
                active: true
            },
        ]
    );
}

#[test]
fn drops_unsafe_domains_and_rejects_bad_envelopes() {
    let body = r#"{"success":true,"errors":[],"messages":[],"result":{"domains":[
        {"domain":"<img src=x>.example.com","enabled":true},
        {"domain":"media.example.com:8443","enabled":true},
        {"domain":"media.example.com","enabled":true}
    ]}}"#;
    let domains = parse_custom(body.as_bytes()).unwrap();
    assert_eq!(domains.len(), 1);
    assert_eq!(domains[0].domain, "media.example.com");
    assert!(!domains[0].active, "no status means not known to be active");

    let empty = r#"{"success":true,"errors":[],"messages":[],"result":{"domains":[]}}"#;
    assert!(parse_custom(empty.as_bytes()).unwrap().is_empty());

    assert_eq!(
        parse_custom(AUTH_ERROR.as_bytes()),
        Err(CfApiError::InvalidResponse)
    );
    assert_eq!(parse_custom(b"not json"), Err(CfApiError::InvalidResponse));
}

#[test]
fn parses_managed_domain() {
    assert_eq!(
        parse_managed(MANAGED_DOMAIN_OK.as_bytes())
            .unwrap()
            .as_deref(),
        Some(R2_DEV)
    );
    let disabled = MANAGED_DOMAIN_OK.replace("\"enabled\": true", "\"enabled\": false");
    assert_eq!(parse_managed(disabled.as_bytes()).unwrap(), None);
    let foreign = MANAGED_DOMAIN_OK.replace(R2_DEV, "media.example.com");
    assert_eq!(parse_managed(foreign.as_bytes()).unwrap(), None);
}

#[test]
fn jurisdiction_headers() {
    assert_eq!(jurisdiction_header(Jurisdiction::Default), None);
    assert_eq!(jurisdiction_header(Jurisdiction::Eu), Some("eu"));
    assert_eq!(jurisdiction_header(Jurisdiction::Fedramp), Some("fedramp"));
}

#[tokio::test]
async fn fetches_both_domains_with_bearer_token_and_jurisdiction() {
    let mock = MockCloudflare::start(
        Reply::Json(200, CUSTOM_DOMAINS_OK),
        Reply::Json(200, MANAGED_DOMAIN_OK),
    )
    .await;
    let domains = client(&mock)
        .bucket_domains(ACCOUNT, "media-library", Jurisdiction::Eu, &token())
        .await
        .unwrap();
    assert_eq!(domains.preferred_custom(), Some("media.example.com"));
    assert_eq!(domains.managed.as_deref(), Some(R2_DEV));
    let heads = mock.requests();
    assert_eq!(heads.len(), 2);
    for head in &heads {
        let lower = head.to_ascii_lowercase();
        assert!(lower.contains(&format!(
            "authorization: bearer {}",
            TOKEN.to_ascii_lowercase()
        )));
        assert!(lower.contains("cf-r2-jurisdiction: eu"));
        assert!(head.contains(&format!(
            "/client/v4/accounts/{ACCOUNT}/r2/buckets/media-library/domains/"
        )));
    }
}

#[tokio::test]
async fn default_jurisdiction_sends_no_header() {
    let mock = MockCloudflare::start(
        Reply::Json(200, CUSTOM_DOMAINS_OK),
        Reply::Json(200, MANAGED_DOMAIN_OK),
    )
    .await;
    client(&mock)
        .bucket_domains(ACCOUNT, "media-library", Jurisdiction::Default, &token())
        .await
        .unwrap();
    assert!(
        mock.requests()
            .iter()
            .all(|h| !h.to_ascii_lowercase().contains("cf-r2-jurisdiction"))
    );
}

#[tokio::test]
async fn forbidden_and_unauthorized_are_typed_and_never_carry_the_token() {
    for (status, want) in [
        (403, CfApiError::Forbidden),
        (401, CfApiError::Unauthorized),
    ] {
        let mock = MockCloudflare::start(
            Reply::Json(status, AUTH_ERROR),
            Reply::Json(status, AUTH_ERROR),
        )
        .await;
        let err = client(&mock)
            .bucket_domains(ACCOUNT, "media-library", Jurisdiction::Default, &token())
            .await
            .unwrap_err();
        assert_eq!(err, want);
        assert!(err.is_permission());
        assert!(!format!("{err} {err:?}").contains(TOKEN));
    }
}

#[tokio::test]
async fn managed_failure_keeps_custom_domains() {
    let mock = MockCloudflare::start(
        Reply::Json(200, CUSTOM_DOMAINS_OK),
        Reply::Json(403, AUTH_ERROR),
    )
    .await;
    let domains = client(&mock)
        .bucket_domains(ACCOUNT, "media-library", Jurisdiction::Default, &token())
        .await
        .unwrap();
    assert_eq!(domains.preferred_custom(), Some("media.example.com"));
    assert_eq!(domains.managed, None);
}

#[tokio::test]
async fn slow_server_times_out() {
    let mock = MockCloudflare::start(Reply::Hang, Reply::Hang).await;
    let client = CfApiClient::with_base_url(&mock.base, Duration::from_millis(300)).unwrap();
    let started = std::time::Instant::now();
    let err = client
        .bucket_domains(ACCOUNT, "media-library", Jurisdiction::Default, &token())
        .await
        .unwrap_err();
    assert_eq!(err, CfApiError::Timeout);
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[tokio::test]
async fn unreachable_server_is_a_network_error() {
    // Bind then drop, so nothing listens on the port.
    let port = TcpListener::bind("127.0.0.1:0")
        .await
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let client =
        CfApiClient::with_base_url(&format!("http://127.0.0.1:{port}"), Duration::from_secs(2))
            .unwrap();
    let err = client
        .bucket_domains(ACCOUNT, "media-library", Jurisdiction::Default, &token())
        .await
        .unwrap_err();
    assert_eq!(err, CfApiError::Network);
}

#[tokio::test]
async fn unsafe_path_values_are_refused_before_any_request() {
    let mock = MockCloudflare::start(
        Reply::Json(200, CUSTOM_DOMAINS_OK),
        Reply::Json(200, MANAGED_DOMAIN_OK),
    )
    .await;
    let client = client(&mock);
    for (account, bucket) in [("../x", "media-library"), (ACCOUNT, "a/b"), (ACCOUNT, "")] {
        assert!(
            client
                .bucket_domains(account, bucket, Jurisdiction::Default, &token())
                .await
                .is_err()
        );
    }
    assert!(mock.requests().is_empty());
}

#[test]
fn real_client_targets_the_cloudflare_api_over_https() {
    assert!(CLOUDFLARE_API.starts_with("https://api.cloudflare.com/"));
    assert!(CfApiClient::cloudflare().is_ok());
}
