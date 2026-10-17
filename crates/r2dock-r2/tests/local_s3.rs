#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use r2dock_core::connections::Jurisdiction;
use r2dock_r2::ops::TestOutcome;
use r2dock_r2::{ClientParams, R2Client, R2Error};
use r2dock_testkit::{ACCESS_KEY, LocalS3, SECRET_KEY};
use tokio_util::sync::CancellationToken;

fn client(s3: &LocalS3, secret: &str) -> R2Client {
    R2Client::new(&ClientParams {
        account_id: "test".into(),
        access_key_id: ACCESS_KEY.into(),
        secret: secret.to_string().into(),
        jurisdiction: Jurisdiction::Default,
        endpoint_override: Some(s3.endpoint.clone()),
    })
}

async fn put(c: &R2Client, bucket: &str, key: &str, body: &[u8]) {
    c.sdk()
        .put_object()
        .bucket(bucket)
        .key(key)
        .body(body.to_vec().into())
        .send()
        .await
        .unwrap();
}

#[tokio::test]
async fn test_connection_lists_buckets() {
    let s3 = LocalS3::start(&["alpha", "beta"]).await;
    let c = client(&s3, SECRET_KEY);
    match c.test_connection(None).await.unwrap() {
        TestOutcome::Buckets(b) => {
            let names: Vec<_> = b.iter().map(|x| x.name.as_str()).collect();
            assert_eq!(names, ["alpha", "beta"]);
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[tokio::test]
async fn wrong_secret_is_an_auth_failure() {
    let s3 = LocalS3::start(&["alpha"]).await;
    let c = client(&s3, "wrong-secret");
    let err = c.test_connection(None).await.unwrap_err();
    assert!(
        err.is_auth_failure() || matches!(err, R2Error::AccessDenied { .. }),
        "{err:?}"
    );
    let app = err.to_app_error();
    assert!(!app.message.contains("wrong-secret"));
    assert!(!app.detail.unwrap_or_default().contains("wrong-secret"));
}

#[tokio::test]
async fn listing_hides_marker_and_paginates() {
    let s3 = LocalS3::start(&["bkt"]).await;
    let c = client(&s3, SECRET_KEY);
    c.create_folder("bkt", "docs/").await.unwrap();
    // Creating it twice is fine.
    c.create_folder("bkt", "docs/").await.unwrap();
    for i in 0..5 {
        put(&c, "bkt", &format!("docs/f{i}.txt"), b"x").await;
    }
    put(&c, "bkt", "docs/sub/inner.txt", b"y").await;
    put(&c, "bkt", "top.txt", b"z").await;

    let root = c.list_page("bkt", "", None, None).await.unwrap();
    assert_eq!(root.folders.len(), 1);
    assert_eq!(root.folders[0].prefix, "docs/");
    assert_eq!(root.objects.len(), 1);

    let mut seen = Vec::new();
    let mut token = None;
    loop {
        let page = c
            .list_page("bkt", "docs/", token.as_deref(), Some(2))
            .await
            .unwrap();
        assert!(
            page.objects.iter().all(|o| o.key != "docs/"),
            "marker must be hidden"
        );
        seen.extend(page.objects.into_iter().map(|o| o.key));
        seen.extend(page.folders.into_iter().map(|f| f.prefix));
        match page.next_token {
            Some(t) => token = Some(t),
            None => break,
        }
    }
    seen.sort();
    seen.dedup();
    assert_eq!(seen.len(), 6, "{seen:?}");
    assert!(seen.contains(&"docs/sub/".to_string()));
}

#[tokio::test]
async fn head_copy_delete() {
    let s3 = LocalS3::start(&["bkt"]).await;
    let c = client(&s3, SECRET_KEY);
    put(&c, "bkt", "a dir/file one.txt", b"hello world").await;
    let d = c.head_object("bkt", "a dir/file one.txt").await.unwrap();
    assert_eq!(d.size, 11);
    assert!(c.exists("bkt", "a dir/file one.txt").await.unwrap());
    assert!(!c.exists("bkt", "missing.txt").await.unwrap());
    assert!(matches!(
        c.head_object("bkt", "missing.txt").await,
        Err(R2Error::NoSuchKey { .. })
    ));

    c.copy_object(
        "bkt",
        "a dir/file one.txt",
        "copied/file two.txt",
        d.size,
        d.etag.as_deref(),
    )
    .await
    .unwrap();
    assert_eq!(
        c.head_object("bkt", "copied/file two.txt")
            .await
            .unwrap()
            .size,
        11
    );

    let failures = c
        .delete_batch(
            "bkt",
            &[
                "a dir/file one.txt".to_string(),
                "copied/file two.txt".to_string(),
            ],
        )
        .await
        .unwrap();
    assert!(failures.is_empty(), "{failures:?}");
    assert!(!c.exists("bkt", "copied/file two.txt").await.unwrap());
}

#[tokio::test]
async fn ranged_preview_reports_truncation() {
    let s3 = LocalS3::start(&["bkt"]).await;
    let c = client(&s3, SECRET_KEY);
    put(&c, "bkt", "long.txt", &vec![b'a'; 5000]).await;
    let (bytes, truncated) = c.read_head_bytes("bkt", "long.txt", 1024).await.unwrap();
    assert_eq!(bytes.len(), 1024);
    assert!(truncated);
    put(&c, "bkt", "short.txt", b"tiny").await;
    let (bytes, truncated) = c.read_head_bytes("bkt", "short.txt", 1024).await.unwrap();
    assert_eq!(bytes, b"tiny");
    assert!(!truncated);
}

#[tokio::test]
async fn for_each_object_visits_everything() {
    let s3 = LocalS3::start(&["bkt"]).await;
    let c = client(&s3, SECRET_KEY);
    for i in 0..7 {
        put(&c, "bkt", &format!("p/{i}/x.bin"), b"12345").await;
    }
    let cancel = CancellationToken::new();
    let mut total = 0;
    let mut count = 0;
    c.for_each_object("bkt", "p/", &cancel, |o| {
        total += o.size;
        count += 1;
        true
    })
    .await
    .unwrap();
    assert_eq!((count, total), (7, 35));

    cancel.cancel();
    let err = c
        .for_each_object("bkt", "p/", &cancel, |_| true)
        .await
        .unwrap_err();
    assert_eq!(err, R2Error::Cancelled);
}

#[tokio::test]
async fn missing_bucket_maps_cleanly() {
    let s3 = LocalS3::start(&["bkt"]).await;
    let c = client(&s3, SECRET_KEY);
    let err = c.list_page("nope", "", None, None).await.unwrap_err();
    assert!(
        matches!(
            err,
            R2Error::NoSuchBucket { .. } | R2Error::NoSuchKey { .. }
        ),
        "{err:?}"
    );
}

#[tokio::test]
async fn bulk_delete_and_move() {
    use r2dock_r2::bulk::{collect_objects, delete_keys, move_items, plan_move};
    let s3 = LocalS3::start(&["bkt"]).await;
    let c = client(&s3, SECRET_KEY);
    let cancel = CancellationToken::new();
    for i in 0..12 {
        put(
            &c,
            "bkt",
            &format!("src/f{i}.txt"),
            format!("{i}").as_bytes(),
        )
        .await;
    }
    put(&c, "bkt", "src/sub/deep.txt", b"deep").await;
    put(&c, "bkt", "dest/f3.txt", b"existing").await;

    let objects = collect_objects(&c, "bkt", &[], &["src/".to_string()], &cancel)
        .await
        .unwrap();
    assert_eq!(objects.len(), 13);

    // Move the folder into dest/: f3 conflicts in dest/src/? No: dest/src/f3.txt is free.
    let items = plan_move(&objects, &[], &["src/".to_string()], "dest/", None).unwrap();
    let report = move_items(&c, "bkt", items, false, &cancel, |_| {})
        .await
        .unwrap();
    assert_eq!(report.moved, 13);
    assert!(report.failures.is_empty());
    assert!(!c.exists("bkt", "src/f0.txt").await.unwrap());
    assert_eq!(
        c.head_object("bkt", "dest/src/sub/deep.txt")
            .await
            .unwrap()
            .size,
        4
    );

    // Moving a file onto an existing name fails and keeps the source.
    put(&c, "bkt", "f3.txt", b"top").await;
    let objects = collect_objects(&c, "bkt", &["f3.txt".to_string()], &[], &cancel)
        .await
        .unwrap();
    let items = plan_move(&objects, &["f3.txt".to_string()], &[], "dest/", None).unwrap();
    let report = move_items(&c, "bkt", items, false, &cancel, |_| {})
        .await
        .unwrap();
    assert_eq!(report.moved, 0);
    assert_eq!(report.failures.len(), 1);
    assert!(c.exists("bkt", "f3.txt").await.unwrap());

    // Delete everything under dest/ in batches.
    let objects = collect_objects(&c, "bkt", &[], &["dest/".to_string()], &cancel)
        .await
        .unwrap();
    let keys: Vec<String> = objects.into_iter().map(|o| o.key).collect();
    let n = keys.len() as u64;
    let seen = std::sync::atomic::AtomicU64::new(0);
    let (deleted, failures) = delete_keys(&c, "bkt", keys, &cancel, |p| {
        seen.store(p, std::sync::atomic::Ordering::Relaxed)
    })
    .await
    .unwrap();
    assert_eq!(deleted, n);
    assert!(failures.is_empty());
    assert_eq!(seen.load(std::sync::atomic::Ordering::Relaxed), n);
    let left = collect_objects(&c, "bkt", &[], &["dest/".to_string()], &cancel)
        .await
        .unwrap();
    assert!(left.is_empty());
}
