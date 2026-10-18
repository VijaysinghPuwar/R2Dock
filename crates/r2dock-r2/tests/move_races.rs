//! Move safety under concurrent changes. A mock server changes objects at
//! exact request boundaries; every case must keep the changed data.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::sync::Arc;

use r2dock_core::connections::Jurisdiction;
use r2dock_r2::bulk::{MoveItem, MoveReport, move_items};
use r2dock_r2::{ClientParams, R2Client};
use support::mock_s3::{Hook, MockS3, Model, Req};
use tokio_util::sync::CancellationToken;

fn client(s3: &MockS3) -> R2Client {
    R2Client::new(&ClientParams {
        account_id: "test".into(),
        access_key_id: "test".into(),
        secret: "test".to_string().into(),
        jurisdiction: Jurisdiction::Default,
        endpoint_override: Some(s3.endpoint.clone()),
    })
}

async fn run_move(model: Model, size: u64, overwrite: bool, hook: Hook) -> (MockS3, MoveReport) {
    let s3 = MockS3::start(model, hook).await;
    let c = client(&s3);
    let report = move_items(
        &c,
        "bkt",
        vec![MoveItem {
            src: "src".into(),
            dst: "dst".into(),
            size,
        }],
        overwrite,
        &CancellationToken::new(),
        |_| {},
    )
    .await
    .unwrap();
    (s3, report)
}

fn with_source() -> Model {
    let mut m = Model::default();
    m.put("src", b"OLD!");
    m
}

fn no_hook() -> Hook {
    Arc::new(|_: &Req, _: &mut Model| {})
}

#[tokio::test]
async fn plain_move_completes() {
    let (s3, report) = run_move(with_source(), 4, false, no_hook()).await;
    assert_eq!(report.moved, 1, "{report:?}");
    let m = s3.model.lock().unwrap();
    assert!(m.body("src").is_none());
    assert_eq!(m.body("dst"), Some(&b"OLD!"[..]));
    // The copy asked R2 to refuse an existing destination.
    let copy = m.requests.iter().find(|r| r.is("PUT", "dst")).unwrap();
    assert_eq!(copy.header("cf-copy-destination-if-none-match"), Some("*"));
    assert!(copy.header("x-amz-copy-source-if-match").is_some());
}

#[tokio::test]
async fn overwrite_move_does_not_send_destination_condition() {
    let mut model = with_source();
    model.put("dst", b"PREV");
    let (s3, report) = run_move(model, 4, true, no_hook()).await;
    assert_eq!(report.moved, 1, "{report:?}");
    let m = s3.model.lock().unwrap();
    assert_eq!(m.body("dst"), Some(&b"OLD!"[..]));
    let copy = m.requests.iter().find(|r| r.is("PUT", "dst")).unwrap();
    assert!(copy.header("cf-copy-destination-if-none-match").is_none());
}

/// T21 case 1: the source changes right after the copy. The new version
/// must not be deleted.
#[tokio::test]
async fn source_changed_after_copy_is_kept() {
    let hook: Hook = Arc::new(|r: &Req, m: &mut Model| {
        if r.is("PUT", "dst") && r.status == 200 {
            m.put("src", b"NEW!");
        }
    });
    let (s3, report) = run_move(with_source(), 4, false, hook).await;
    let m = s3.model.lock().unwrap();
    assert_eq!(m.body("src"), Some(&b"NEW!"[..]), "{:?}", m.sequence());
    assert_eq!(report.moved, 0);
    assert_eq!(report.copied_not_deleted.len(), 1, "{report:?}");
    assert!(!m.requests.iter().any(|r| r.is("DELETE", "src")));
}

/// T21 case 2: the destination appears after the existence check. The copy
/// must not replace it.
#[tokio::test]
async fn destination_created_after_check_is_not_overwritten() {
    let hook: Hook = Arc::new(|r: &Req, m: &mut Model| {
        if r.is("HEAD", "dst") && r.status == 404 {
            m.put("dst", b"RACE");
        }
    });
    let (s3, report) = run_move(with_source(), 4, false, hook).await;
    let m = s3.model.lock().unwrap();
    assert_eq!(m.body("dst"), Some(&b"RACE"[..]), "{:?}", m.sequence());
    assert_eq!(m.body("src"), Some(&b"OLD!"[..]));
    assert_eq!(report.moved, 0);
    assert_eq!(report.failures.len(), 1, "{report:?}");
}

/// T21 case 3: the destination is replaced with same-length content after
/// the copy. Size alone matches, so the ETag must catch it.
#[tokio::test]
async fn destination_replaced_after_copy_keeps_source() {
    let hook: Hook = Arc::new(|r: &Req, m: &mut Model| {
        if r.is("PUT", "dst") && r.status == 200 {
            m.put("dst", b"RACE");
        }
    });
    let (s3, report) = run_move(with_source(), 4, false, hook).await;
    let m = s3.model.lock().unwrap();
    assert_eq!(m.body("src"), Some(&b"OLD!"[..]), "{:?}", m.sequence());
    assert_eq!(report.moved, 0);
    assert_eq!(
        report.failures.len() + report.copied_not_deleted.len(),
        1,
        "{report:?}"
    );
    assert!(!m.requests.iter().any(|r| r.is("DELETE", "src")));
}

/// The source changes between the first inspection and the copy: the copy
/// is refused by the source condition and nothing is deleted.
#[tokio::test]
async fn source_changed_before_copy_is_refused() {
    let hook: Hook = Arc::new(|r: &Req, m: &mut Model| {
        if r.is("HEAD", "src") {
            m.put("src", b"NEW!");
        }
    });
    let (s3, report) = run_move(with_source(), 4, false, hook).await;
    let m = s3.model.lock().unwrap();
    assert_eq!(m.body("src"), Some(&b"NEW!"[..]));
    assert!(m.body("dst").is_none());
    assert_eq!(report.failures.len(), 1, "{report:?}");
}

/// A delete that fails after a verified copy is reported as copied only.
#[tokio::test]
async fn failed_delete_is_copied_only() {
    let mut model = with_source();
    model.fail_deletes = true;
    let (s3, report) = run_move(model, 4, false, no_hook()).await;
    assert_eq!(report.moved, 0);
    assert_eq!(report.copied_not_deleted.len(), 1, "{report:?}");
    let m = s3.model.lock().unwrap();
    assert_eq!(m.body("src"), Some(&b"OLD!"[..]));
    assert_eq!(m.body("dst"), Some(&b"OLD!"[..]));
}

/// The copy happens but its response is lost (the SDK retries, and the
/// retry is refused because the destination now exists). The outcome is
/// ambiguous, so the source is kept.
#[tokio::test]
async fn lost_copy_response_keeps_source() {
    let mut model = with_source();
    model.lose_copy_responses = true;
    let (s3, report) = run_move(model, 4, false, no_hook()).await;
    assert_eq!(report.moved, 0);
    assert_eq!(report.failures.len(), 1, "{report:?}");
    let m = s3.model.lock().unwrap();
    assert_eq!(m.body("src"), Some(&b"OLD!"[..]));
    assert!(!m.requests.iter().any(|r| r.is("DELETE", "src")));
}

/// The source was removed by someone else after the copy: the copy is
/// verified, nothing is left to delete, and the move counts as done.
#[tokio::test]
async fn source_removed_after_copy_counts_as_moved() {
    let hook: Hook = Arc::new(|r: &Req, m: &mut Model| {
        if r.is("PUT", "dst") && r.status == 200 {
            m.objects.remove("src");
        }
    });
    let (s3, report) = run_move(with_source(), 4, false, hook).await;
    assert_eq!(report.moved, 1, "{report:?}");
    let m = s3.model.lock().unwrap();
    assert_eq!(m.body("dst"), Some(&b"OLD!"[..]));
    assert!(!m.requests.iter().any(|r| r.is("DELETE", "src")));
}

fn is_part(r: &Req, n: u32) -> bool {
    r.method == "PUT" && r.query.split('&').any(|kv| kv == format!("partNumber={n}"))
}

const BIG: u64 = 5 * 1024 * 1024 * 1024 + 1;

fn big_source(headers: Vec<(String, String)>) -> Model {
    let mut m = Model::default();
    m.put_sized("src", b"BIG!", BIG, headers);
    m
}

/// Multipart copy: UploadPartCopy carries no source condition (R2 lists it as
/// unsupported), and the source is checked again before completing.
#[tokio::test]
async fn multipart_move_completes_without_part_conditions() {
    let (s3, report) = run_move(big_source(Vec::new()), BIG, false, no_hook()).await;
    assert_eq!(report.moved, 1, "{report:?}");
    let m = s3.model.lock().unwrap();
    let parts: Vec<&Req> = m
        .requests
        .iter()
        .filter(|r| r.method == "PUT" && r.query.contains("uploadId"))
        .collect();
    assert!(parts.len() > 1);
    assert!(
        parts
            .iter()
            .all(|r| r.header("x-amz-copy-source-if-match").is_none())
    );
    assert!(m.body("src").is_none());
}

/// Multipart copy: the source changes while parts are copied. The upload is
/// aborted and the source is kept.
#[tokio::test]
async fn multipart_source_changed_during_parts_is_aborted() {
    let hook: Hook = Arc::new(|r: &Req, m: &mut Model| {
        if is_part(r, 1) {
            let size = m.objects["src"].size;
            m.put_sized("src", b"NEW!", size, Vec::new());
        }
    });
    let (s3, report) = run_move(big_source(Vec::new()), BIG, false, hook).await;
    let m = s3.model.lock().unwrap();
    assert_eq!(m.body("src"), Some(&b"NEW!"[..]));
    assert!(m.body("dst").is_none(), "{:?}", m.sequence());
    assert_eq!(report.failures.len(), 1, "{report:?}");
    assert!(
        m.requests
            .iter()
            .any(|r| r.method == "DELETE" && r.query.contains("uploadId"))
    );
}

/// Multipart copy: the destination appears while parts are copied. The
/// upload is aborted rather than completed over it.
#[tokio::test]
async fn multipart_destination_created_during_parts_is_not_overwritten() {
    let hook: Hook = Arc::new(|r: &Req, m: &mut Model| {
        if r.method == "POST" && r.query.contains("uploads") {
            m.put("dst", b"RACE");
        }
    });
    let (s3, report) = run_move(big_source(Vec::new()), BIG, false, hook).await;
    let m = s3.model.lock().unwrap();
    assert_eq!(m.body("dst"), Some(&b"RACE"[..]), "{:?}", m.sequence());
    assert_eq!(m.body("src"), Some(&b"BIG!"[..]));
    assert_eq!(report.failures.len(), 1, "{report:?}");
}

/// F18: the multipart create request carries every supported metadata field.
#[tokio::test]
async fn multipart_copy_keeps_metadata() {
    let headers = vec![
        ("content-type".to_string(), "video/mp4".to_string()),
        ("cache-control".to_string(), "max-age=60".to_string()),
        ("content-encoding".to_string(), "identity".to_string()),
        (
            "content-disposition".to_string(),
            "attachment; filename=\"a.mp4\"".to_string(),
        ),
        ("content-language".to_string(), "fr".to_string()),
        (
            "expires".to_string(),
            "Wed, 21 Oct 2026 07:28:00 GMT".to_string(),
        ),
        ("x-amz-storage-class".to_string(), "STANDARD_IA".to_string()),
        ("x-amz-meta-origin".to_string(), "camera".to_string()),
    ];
    let (s3, report) = run_move(big_source(headers), BIG, false, no_hook()).await;
    assert_eq!(report.moved, 1, "{report:?}");
    let m = s3.model.lock().unwrap();
    let create = m
        .requests
        .iter()
        .find(|r| r.method == "POST" && r.query.contains("uploads"))
        .unwrap();
    assert_eq!(create.header("content-type"), Some("video/mp4"));
    assert_eq!(create.header("cache-control"), Some("max-age=60"));
    assert_eq!(create.header("content-encoding"), Some("identity"));
    assert_eq!(
        create.header("content-disposition"),
        Some("attachment; filename=\"a.mp4\"")
    );
    assert_eq!(create.header("content-language"), Some("fr"));
    assert_eq!(
        create.header("expires"),
        Some("Wed, 21 Oct 2026 07:28:00 GMT")
    );
    assert_eq!(create.header("x-amz-storage-class"), Some("STANDARD_IA"));
    assert_eq!(create.header("x-amz-meta-origin"), Some("camera"));
}

/// Absent optional fields are not invented on the create request.
#[tokio::test]
async fn multipart_copy_omits_absent_metadata() {
    let (s3, report) = run_move(big_source(Vec::new()), BIG, false, no_hook()).await;
    assert_eq!(report.moved, 1, "{report:?}");
    let m = s3.model.lock().unwrap();
    let create = m
        .requests
        .iter()
        .find(|r| r.method == "POST" && r.query.contains("uploads"))
        .unwrap();
    for h in [
        "content-disposition",
        "content-language",
        "expires",
        "x-amz-storage-class",
        "cache-control",
    ] {
        assert!(create.header(h).is_none(), "{h} was sent");
    }
}
