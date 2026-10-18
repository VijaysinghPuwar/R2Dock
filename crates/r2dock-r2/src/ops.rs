//! Object operations: buckets, details, folders, delete, copy, ranged reads.

use std::collections::BTreeMap;

use aws_sdk_s3::operation::get_object::GetObjectOutput;
use aws_sdk_s3::operation::head_object::HeadObjectOutput;
use aws_sdk_s3::primitives::ByteStream;
use aws_sdk_s3::types::{
    CompletedMultipartUpload, CompletedPart, Delete, MetadataDirective, ObjectIdentifier,
};
use r2dock_core::etag::is_single_part_md5;
use r2dock_core::keys::{base_name, display_name, percent_encode_key};
use r2dock_core::parts::{MAX_SINGLE_PUT, PartRange, part_size_for, plan_parts};

use crate::client::R2Client;
use crate::dto::{BucketEntry, KeyFailure, ObjectDetails};
use crate::error::R2Error;
use crate::list::to_rfc3339;

/// DeleteObjects accepts at most 1,000 keys per request.
pub const DELETE_BATCH: usize = 1000;

/// R2 extension header for a copy that must not replace an existing object.
/// <https://developers.cloudflare.com/r2/api/s3/extensions/>
const COPY_DESTINATION_IF_NONE_MATCH: &str = "cf-copy-destination-if-none-match";

/// One server-side copy.
#[derive(Debug, Clone, Copy)]
pub struct CopySpec<'a> {
    pub src_key: &'a str,
    pub dst_key: &'a str,
    /// Size of the source version being copied (chooses single or multipart).
    pub size: u64,
    /// Copy only this version of the source.
    pub source_etag: Option<&'a str>,
    /// Refuse to replace an existing destination. Atomic for copies up to
    /// 5 GiB (R2 `cf-copy-destination-if-none-match`); checked right before
    /// completion for larger copies.
    pub create_only: bool,
}

/// Whether two ETags name the same object version. Surrounding quotes are
/// ignored because some responses omit them.
pub fn same_etag(a: &str, b: &str) -> bool {
    let a = a.trim().trim_matches('"');
    let b = b.trim().trim_matches('"');
    !a.is_empty() && a == b
}

fn content_length(len: Option<i64>) -> u64 {
    u64::try_from(len.unwrap_or(0)).unwrap_or(0)
}

fn parse_http_date(s: &str) -> Option<aws_smithy_types::DateTime> {
    aws_smithy_types::DateTime::from_str(s, aws_smithy_types::date_time::Format::HttpDate).ok()
}

fn source_changed() -> R2Error {
    R2Error::PreconditionFailed {
        detail: Some("the source changed".to_string()),
    }
}

fn destination_exists() -> R2Error {
    R2Error::PreconditionFailed {
        detail: Some("the destination exists".to_string()),
    }
}

/// What a connection test found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TestOutcome {
    /// The key can list buckets.
    Buckets(Vec<BucketEntry>),
    /// The key cannot list buckets but can list the named bucket.
    ScopedBucket(String),
}

impl R2Client {
    pub async fn list_buckets(&self) -> Result<Vec<BucketEntry>, R2Error> {
        let out = self
            .inner
            .list_buckets()
            .customize()
            .config_override(Self::metadata_timeouts())
            .send()
            .await
            .map_err(R2Error::from_sdk)?;
        let mut buckets: Vec<BucketEntry> = out
            .buckets()
            .iter()
            .filter_map(|b| {
                Some(BucketEntry {
                    name: b.name()?.to_string(),
                    created: to_rfc3339(b.creation_date()),
                })
            })
            .collect();
        buckets.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(buckets)
    }

    /// Test credentials: list buckets, or fall back to the named bucket for
    /// bucket-scoped tokens. Auth failures are returned as errors.
    pub async fn test_connection(&self, bucket: Option<&str>) -> Result<TestOutcome, R2Error> {
        match self.list_buckets().await {
            Ok(buckets) => {
                if let Some(b) = bucket
                    && !buckets.iter().any(|x| x.name == b)
                {
                    // Confirm the named bucket is reachable.
                    self.list_page(b, "", None, Some(1)).await?;
                }
                Ok(TestOutcome::Buckets(buckets))
            }
            Err(R2Error::AccessDenied { detail }) => match bucket {
                Some(b) => {
                    self.list_page(b, "", None, Some(1)).await?;
                    Ok(TestOutcome::ScopedBucket(b.to_string()))
                }
                None => Err(R2Error::AccessDenied { detail }),
            },
            Err(other) => Err(other),
        }
    }

    pub async fn head_object(&self, bucket: &str, key: &str) -> Result<ObjectDetails, R2Error> {
        let out = self.head_raw(bucket, key).await?;
        let etag = out.e_tag().map(str::to_string);
        let sse_c = out.sse_customer_algorithm().is_some();
        let name = base_name(key).to_string();
        Ok(ObjectDetails {
            key: key.to_string(),
            display_name: display_name(&name),
            name,
            size: content_length(out.content_length()),
            content_type: out.content_type().map(str::to_string),
            last_modified: to_rfc3339(out.last_modified()),
            etag_is_md5: !sse_c && etag.as_deref().is_some_and(is_single_part_md5),
            etag,
            storage_class: out
                .storage_class()
                .map_or_else(|| "STANDARD".to_string(), |s| s.as_str().to_string()),
            cache_control: out.cache_control().map(str::to_string),
            content_encoding: out.content_encoding().map(str::to_string),
            metadata: out
                .metadata()
                .map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
                .unwrap_or_else(BTreeMap::new),
        })
    }

    /// Whether an object exists. Missing objects return `Ok(false)`.
    pub async fn exists(&self, bucket: &str, key: &str) -> Result<bool, R2Error> {
        match self.head_object(bucket, key).await {
            Ok(_) => Ok(true),
            Err(R2Error::NoSuchKey { .. }) => Ok(false),
            Err(e) => Err(e),
        }
    }

    /// Create an empty folder marker (`prefix` must end with `/`).
    /// An existing marker is not an error.
    pub async fn create_folder(&self, bucket: &str, prefix: &str) -> Result<(), R2Error> {
        let result = self
            .inner
            .put_object()
            .bucket(bucket)
            .key(prefix)
            .content_length(0)
            .body(ByteStream::from_static(b""))
            .if_none_match("*")
            .send()
            .await
            .map_err(R2Error::from_sdk);
        match result {
            Ok(_) | Err(R2Error::PreconditionFailed { .. }) => Ok(()),
            Err(e) => Err(e),
        }
    }

    pub async fn delete_object(&self, bucket: &str, key: &str) -> Result<(), R2Error> {
        self.inner
            .delete_object()
            .bucket(bucket)
            .key(key)
            .send()
            .await
            .map_err(R2Error::from_sdk)?;
        Ok(())
    }

    /// Delete up to 1,000 keys. Returns keys that failed; a request-level error
    /// is returned as `Err`.
    pub async fn delete_batch(
        &self,
        bucket: &str,
        keys: &[String],
    ) -> Result<Vec<KeyFailure>, R2Error> {
        if keys.is_empty() {
            return Ok(Vec::new());
        }
        if keys.len() > DELETE_BATCH {
            return Err(R2Error::Construction);
        }
        let objects = keys
            .iter()
            .map(|k| ObjectIdentifier::builder().key(k).build())
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| R2Error::Construction)?;
        let delete = Delete::builder()
            .set_objects(Some(objects))
            .quiet(true)
            .build()
            .map_err(|_| R2Error::Construction)?;
        let out = self
            .inner
            .delete_objects()
            .bucket(bucket)
            .delete(delete)
            .send()
            .await
            .map_err(R2Error::from_sdk)?;
        Ok(out
            .errors()
            .iter()
            .map(|e| KeyFailure {
                key: e.key().unwrap_or_default().to_string(),
                message: match e.code() {
                    Some("AccessDenied") => "This key cannot delete it.".to_string(),
                    Some(code) => format!("Could not delete ({code})."),
                    None => "Could not delete.".to_string(),
                },
            })
            .collect())
    }

    /// Server-side copy that keeps content type and metadata. Objects larger
    /// than 5 GiB use multipart copy. Returns the new object's ETag when the
    /// service reports one.
    ///
    /// `spec.source_etag` binds the copy to one version of the source, so a
    /// source that changed after it was inspected is not copied.
    /// `spec.create_only` asks R2 to refuse the copy when the destination
    /// already exists (see [`CopySpec`]).
    pub async fn copy_object(
        &self,
        bucket: &str,
        spec: &CopySpec<'_>,
    ) -> Result<Option<String>, R2Error> {
        let source = format!("{}/{}", bucket, percent_encode_key(spec.src_key));
        if spec.size <= MAX_SINGLE_PUT {
            let mut req = self
                .inner
                .copy_object()
                .bucket(bucket)
                .key(spec.dst_key)
                .copy_source(source)
                .set_copy_source_if_match(spec.source_etag.map(str::to_string))
                .metadata_directive(MetadataDirective::Copy)
                .customize();
            if spec.create_only {
                // R2 extension: evaluated when the copy is committed, so a
                // destination created after any earlier check is not replaced.
                req = req.mutate_request(|r| {
                    r.headers_mut().insert(COPY_DESTINATION_IF_NONE_MATCH, "*");
                });
            }
            let out = req.send().await.map_err(R2Error::from_sdk)?;
            return Ok(out
                .copy_object_result()
                .and_then(|r| r.e_tag())
                .map(str::to_string));
        }
        self.multipart_copy(bucket, &source, spec).await
    }

    /// Copy above 5 GiB. R2 does not support conditional headers on
    /// `UploadPartCopy` or `CompleteMultipartUpload`, so the source and (for
    /// `create_only`) the destination are checked again right before the
    /// upload is completed. A change in that short window is not detected.
    async fn multipart_copy(
        &self,
        bucket: &str,
        source: &str,
        spec: &CopySpec<'_>,
    ) -> Result<Option<String>, R2Error> {
        let head = self.head_raw(bucket, spec.src_key).await?;
        // The source must still be the version that was planned.
        let pinned = match (spec.source_etag, head.e_tag()) {
            (Some(want), Some(have)) if same_etag(want, have) => want.to_string(),
            (None, Some(have)) => have.to_string(),
            _ => return Err(source_changed()),
        };
        if content_length(head.content_length()) != spec.size {
            return Err(source_changed());
        }
        if spec.create_only && self.exists(bucket, spec.dst_key).await? {
            return Err(destination_exists());
        }
        let part_size = part_size_for(spec.size).map_err(|_| R2Error::Construction)?;
        let parts = plan_parts(spec.size, part_size).map_err(|_| R2Error::Construction)?;
        let created = self
            .inner
            .create_multipart_upload()
            .bucket(bucket)
            .key(spec.dst_key)
            .set_content_type(head.content_type().map(str::to_string))
            .set_cache_control(head.cache_control().map(str::to_string))
            .set_content_encoding(head.content_encoding().map(str::to_string))
            .set_content_disposition(head.content_disposition().map(str::to_string))
            .set_content_language(head.content_language().map(str::to_string))
            .set_expires(head.expires_string().and_then(parse_http_date))
            .set_storage_class(head.storage_class().cloned())
            .set_metadata(head.metadata().cloned())
            .send()
            .await
            .map_err(R2Error::from_sdk)?;
        let upload_id = created.upload_id().ok_or(R2Error::Protocol {
            detail: "missing upload id".to_string(),
        })?;
        let result = self
            .copy_parts_and_complete(bucket, source, spec, upload_id, &parts, &pinned)
            .await;
        if result.is_err() {
            let _ = self.abort_multipart(bucket, spec.dst_key, upload_id).await;
        }
        result
    }

    async fn copy_parts_and_complete(
        &self,
        bucket: &str,
        source: &str,
        spec: &CopySpec<'_>,
        upload_id: &str,
        parts: &[PartRange],
        pinned: &str,
    ) -> Result<Option<String>, R2Error> {
        let mut completed = Vec::with_capacity(parts.len());
        for p in parts {
            let range = format!("bytes={}-{}", p.offset, p.offset + p.len - 1);
            // No x-amz-copy-source-if-match: R2 lists it as unsupported on
            // UploadPartCopy. The source is checked again before completing.
            let out = self
                .inner
                .upload_part_copy()
                .bucket(bucket)
                .key(spec.dst_key)
                .upload_id(upload_id)
                .part_number(p.number)
                .copy_source(source)
                .copy_source_range(range)
                .send()
                .await
                .map_err(R2Error::from_sdk)?;
            let etag = out
                .copy_part_result()
                .and_then(|r| r.e_tag())
                .map(str::to_string);
            completed.push(
                CompletedPart::builder()
                    .part_number(p.number)
                    .set_e_tag(etag)
                    .build(),
            );
        }
        let now = self.head_raw(bucket, spec.src_key).await?;
        if !now.e_tag().is_some_and(|e| same_etag(e, pinned))
            || content_length(now.content_length()) != spec.size
        {
            return Err(source_changed());
        }
        if spec.create_only && self.exists(bucket, spec.dst_key).await? {
            return Err(destination_exists());
        }
        let done = self
            .inner
            .complete_multipart_upload()
            .bucket(bucket)
            .key(spec.dst_key)
            .upload_id(upload_id)
            .multipart_upload(
                CompletedMultipartUpload::builder()
                    .set_parts(Some(completed))
                    .build(),
            )
            .send()
            .await
            .map_err(R2Error::from_sdk)?;
        Ok(done.e_tag().map(str::to_string))
    }

    async fn head_raw(&self, bucket: &str, key: &str) -> Result<HeadObjectOutput, R2Error> {
        self.inner
            .head_object()
            .bucket(bucket)
            .key(key)
            .customize()
            .config_override(Self::metadata_timeouts())
            .send()
            .await
            .map_err(R2Error::from_sdk)
    }

    pub async fn abort_multipart(
        &self,
        bucket: &str,
        key: &str,
        upload_id: &str,
    ) -> Result<(), R2Error> {
        self.inner
            .abort_multipart_upload()
            .bucket(bucket)
            .key(key)
            .upload_id(upload_id)
            .send()
            .await
            .map_err(R2Error::from_sdk)?;
        Ok(())
    }

    /// Read at most `max_bytes` from the start of an object (for text previews).
    /// Returns the bytes and whether the object is longer.
    pub async fn read_head_bytes(
        &self,
        bucket: &str,
        key: &str,
        max_bytes: u64,
    ) -> Result<(Vec<u8>, bool), R2Error> {
        let out = self
            .get_object(
                bucket,
                key,
                Some(format!("bytes=0-{}", max_bytes.saturating_sub(1))),
            )
            .await?;
        let total = out
            .content_range()
            .and_then(|r| r.rsplit('/').next())
            .and_then(|t| t.parse::<u64>().ok());
        let mut body = out.body.into_async_read();
        let mut buf = Vec::new();
        let mut limited = tokio::io::AsyncReadExt::take(&mut body, max_bytes);
        tokio::io::AsyncReadExt::read_to_end(&mut limited, &mut buf)
            .await
            .map_err(|_| R2Error::Network)?;
        let truncated = total.is_some_and(|t| t > buf.len() as u64);
        Ok((buf, truncated))
    }

    /// Start a GET, optionally with a Range header value.
    pub async fn get_object(
        &self,
        bucket: &str,
        key: &str,
        range: Option<String>,
    ) -> Result<GetObjectOutput, R2Error> {
        self.inner
            .get_object()
            .bucket(bucket)
            .key(key)
            .set_range(range)
            .send()
            .await
            .map_err(R2Error::from_sdk)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn etag_comparison() {
        assert!(same_etag("\"abc\"", "abc"));
        assert!(same_etag("\"abc-2\"", "\"abc-2\""));
        assert!(!same_etag("\"abc\"", "\"abd\""));
        assert!(!same_etag("\"\"", "\"\""));
        assert!(!same_etag("", ""));
    }

    #[test]
    fn expires_parsing() {
        assert!(parse_http_date("Wed, 21 Oct 2026 07:28:00 GMT").is_some());
        assert!(parse_http_date("not a date").is_none());
    }
}
