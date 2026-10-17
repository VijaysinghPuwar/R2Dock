//! Read-only checks against a real R2 account. Skipped unless run with
//! `--ignored` and these environment variables set:
//! `R2DOCK_LIVE_ACCOUNT`, `R2DOCK_LIVE_KEY_ID`, `R2DOCK_LIVE_SECRET`,
//! `R2DOCK_LIVE_BUCKET`. Nothing is written or deleted.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use r2dock_core::connections::Jurisdiction;
use r2dock_r2::ops::TestOutcome;
use r2dock_r2::{ClientParams, R2Client, R2Error};

fn live() -> Option<(R2Client, String)> {
    let get = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
    let client = R2Client::new(&ClientParams {
        account_id: get("R2DOCK_LIVE_ACCOUNT")?,
        access_key_id: get("R2DOCK_LIVE_KEY_ID")?,
        secret: get("R2DOCK_LIVE_SECRET")?.into(),
        jurisdiction: Jurisdiction::Default,
        endpoint_override: None,
    });
    Some((client, get("R2DOCK_LIVE_BUCKET")?))
}

#[tokio::test]
#[ignore = "needs a real R2 account (read-only)"]
async fn live_read_only_connection_check() {
    let Some((client, bucket)) = live() else {
        eprintln!("live R2 variables not set; skipping");
        return;
    };
    // Listing buckets: allowed for admin tokens, AccessDenied for object tokens.
    match client.list_buckets().await {
        Ok(b) => eprintln!("ListBuckets allowed: {} buckets", b.len()),
        Err(e) => {
            eprintln!("ListBuckets refused: {e:?}");
            assert!(
                matches!(e, R2Error::AccessDenied { .. }),
                "a valid object token should get AccessDenied, got {e:?}"
            );
        }
    }
    // Naming the bucket must work for both token kinds.
    let outcome = client.test_connection(Some(&bucket)).await.unwrap();
    eprintln!(
        "test_connection with bucket: {}",
        match outcome {
            TestOutcome::Buckets(_) => "admin token",
            TestOutcome::ScopedBucket(_) => "object token (bucket by name)",
        }
    );
    let page = client.list_page(&bucket, "", None, Some(5)).await.unwrap();
    eprintln!(
        "listed {} folders and {} objects at the top of {bucket}",
        page.folders.len(),
        page.objects.len()
    );
    // A wrong secret is reported as an auth failure, not AccessDenied.
    let bad = R2Client::new(&ClientParams {
        account_id: std::env::var("R2DOCK_LIVE_ACCOUNT").unwrap(),
        access_key_id: std::env::var("R2DOCK_LIVE_KEY_ID").unwrap(),
        secret: "0".repeat(64).into(),
        jurisdiction: Jurisdiction::Default,
        endpoint_override: None,
    });
    let err = bad.list_page(&bucket, "", None, Some(1)).await.unwrap_err();
    eprintln!("wrong secret gives: {err:?}");
    assert!(
        err.is_auth_failure(),
        "wrong secret should be an auth failure, got {err:?}"
    );
}
