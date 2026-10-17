//! Object operations: buckets, details, folders, delete, copy, ranged reads.

use std::collections::BTreeMap;

use aws_sdk_s3::operation::get_object::GetObjectOutput;
use aws_sdk_s3::primitives::ByteStream;
use aws_sdk_s3::types::{
    CompletedMultipartUpload, CompletedPart, Delete, MetadataDirective, ObjectIdentifier,
};
use r2dock_core::etag::is_single_part_md5;
use r2dock_core::keys::{base_name, display_name, percent_encode_key};
use r2dock_core::parts::{MAX_SINGLE_PUT, part_size_for, plan_parts};

use crate::client::R2Client;
use crate::dto::{BucketEntry, KeyFailure, ObjectDetails};
use crate::error::R2Error;
use crate::list::to_rfc3339;

/// DeleteObjects accepts at most 1,000 keys per request.
pub const DELETE_BATCH: usize = 1000;

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
        let out = self
            .inner
            .head_object()
            .bucket(bucket)
            .key(key)
            .customize()
            .config_override(Self::metadata_timeouts())
            .send()
            .await
            .map_err(R2Error::from_sdk)?;
        let etag = out.e_tag().map(str::to_string);
        let sse_c = out.sse_customer_algorithm().is_some();
        let name = base_name(key).to_string();
        Ok(ObjectDetails {
            key: key.to_string(),
            display_name: display_name(&name),
            name,
            size: u64::try_from(out.content_length().unwrap_or(0)).unwrap_or(0),
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
    /// than 5 GiB use multipart copy.
    ///
    /// `if_match` binds the copy to one version of the source (its ETag), so
    /// a source that changed after it was inspected is not copied.
    pub async fn copy_object(
        &self,
        bucket: &str,
        src_key: &str,
        dst_key: &str,
        size: u64,
        if_match: Option<&str>,
    ) -> Result<(), R2Error> {
        let source = format!("{}/{}", bucket, percent_encode_key(src_key));
        if size <= MAX_SINGLE_PUT {
            self.inner
                .copy_object()
                .bucket(bucket)
                .key(dst_key)
                .copy_source(source)
                .set_copy_source_if_match(if_match.map(str::to_string))
                .metadata_directive(MetadataDirective::Copy)
                .send()
                .await
                .map_err(R2Error::from_sdk)?;
            return Ok(());
        }
        self.multipart_copy(bucket, &source, src_key, dst_key, size, if_match)
            .await
    }

    async fn multipart_copy(
        &self,
        bucket: &str,
        source: &str,
        src_key: &str,
        dst_key: &str,
        size: u64,
        if_match: Option<&str>,
    ) -> Result<(), R2Error> {
        let head = self.head_object(bucket, src_key).await?;
        // The source must still be the version that was planned.
        if head.size != size || if_match.is_some_and(|e| head.etag.as_deref() != Some(e)) {
            return Err(R2Error::PreconditionFailed {
                detail: Some("the source changed".to_string()),
            });
        }
        let if_match = if_match.map(str::to_string).or_else(|| head.etag.clone());
        let part_size = part_size_for(size).map_err(|_| R2Error::Construction)?;
        let parts = plan_parts(size, part_size).map_err(|_| R2Error::Construction)?;
        let created = self
            .inner
            .create_multipart_upload()
            .bucket(bucket)
            .key(dst_key)
            .set_content_type(head.content_type.clone())
            .set_cache_control(head.cache_control.clone())
            .set_content_encoding(head.content_encoding.clone())
            .set_metadata(Some(head.metadata.clone().into_iter().collect()))
            .send()
            .await
            .map_err(R2Error::from_sdk)?;
        let upload_id = created.upload_id().ok_or(R2Error::Protocol {
            detail: "missing upload id".to_string(),
        })?;
        let mut completed = Vec::with_capacity(parts.len());
        for p in &parts {
            let range = format!("bytes={}-{}", p.offset, p.offset + p.len - 1);
            let result = self
                .inner
                .upload_part_copy()
                .bucket(bucket)
                .key(dst_key)
                .upload_id(upload_id)
                .part_number(p.number)
                .copy_source(source)
                .copy_source_range(range)
                .set_copy_source_if_match(if_match.clone())
                .send()
                .await
                .map_err(R2Error::from_sdk);
            match result {
                Ok(out) => {
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
                Err(e) => {
                    let _ = self.abort_multipart(bucket, dst_key, upload_id).await;
                    return Err(e);
                }
            }
        }
        let done = self
            .inner
            .complete_multipart_upload()
            .bucket(bucket)
            .key(dst_key)
            .upload_id(upload_id)
            .multipart_upload(
                CompletedMultipartUpload::builder()
                    .set_parts(Some(completed))
                    .build(),
            )
            .send()
            .await
            .map_err(R2Error::from_sdk);
        if let Err(e) = done {
            let _ = self.abort_multipart(bucket, dst_key, upload_id).await;
            return Err(e);
        }
        Ok(())
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
