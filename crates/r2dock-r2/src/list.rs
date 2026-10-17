//! Listing with defensive pagination (PLAN.md 8.3).

use aws_sdk_s3::primitives::DateTime as SdkDateTime;
use aws_sdk_s3::types::Object;
use r2dock_core::keys::{base_name, display_name, is_unusual_key};
use tokio_util::sync::CancellationToken;

use crate::client::R2Client;
use crate::dto::{FolderEntry, ListPage, ObjectEntry};
use crate::error::R2Error;

pub const PAGE_SIZE: i32 = 1000;

pub(crate) fn to_rfc3339(dt: Option<&SdkDateTime>) -> Option<String> {
    let dt = dt?;
    chrono::DateTime::from_timestamp(dt.secs(), dt.subsec_nanos()).map(|d| d.to_rfc3339())
}

fn object_entry(obj: &Object) -> Option<ObjectEntry> {
    let key = obj.key()?.to_string();
    let name = base_name(&key).to_string();
    Some(ObjectEntry {
        display_name: display_name(&name),
        unusual: is_unusual_key(&key),
        name,
        size: u64::try_from(obj.size().unwrap_or(0)).unwrap_or(0),
        last_modified: to_rfc3339(obj.last_modified()),
        etag: obj.e_tag().map(str::to_string),
        storage_class: obj.storage_class().map(|s| s.as_str().to_string()),
        key,
    })
}

impl R2Client {
    /// One page of a folder listing (`/` delimiter). The folder's own marker
    /// object is hidden.
    pub async fn list_page(
        &self,
        bucket: &str,
        prefix: &str,
        token: Option<&str>,
        max_keys: Option<i32>,
    ) -> Result<ListPage, R2Error> {
        let out = self
            .inner
            .list_objects_v2()
            .bucket(bucket)
            .prefix(prefix)
            .delimiter("/")
            .max_keys(max_keys.unwrap_or(PAGE_SIZE))
            .set_continuation_token(token.map(str::to_string))
            .customize()
            .config_override(Self::metadata_timeouts())
            .send()
            .await
            .map_err(R2Error::from_sdk)?;

        let next_token =
            check_next_token(out.is_truncated(), out.next_continuation_token(), token)?;

        let folders = out
            .common_prefixes()
            .iter()
            .filter_map(|p| p.prefix())
            .filter(|p| *p != prefix)
            .map(|p| {
                let name = base_name(p).to_string();
                FolderEntry {
                    prefix: p.to_string(),
                    display_name: display_name(&name),
                    unusual: is_unusual_key(p),
                    name,
                }
            })
            .collect();
        let objects = out
            .contents()
            .iter()
            .filter(|o| o.key() != Some(prefix))
            .filter_map(object_entry)
            .collect();
        Ok(ListPage {
            prefix: prefix.to_string(),
            folders,
            objects,
            next_token,
        })
    }

    /// Prefix search across a bucket (no delimiter), one page at a time.
    pub async fn search_prefix_page(
        &self,
        bucket: &str,
        prefix: &str,
        token: Option<&str>,
    ) -> Result<ListPage, R2Error> {
        let out = self
            .inner
            .list_objects_v2()
            .bucket(bucket)
            .prefix(prefix)
            .max_keys(PAGE_SIZE)
            .set_continuation_token(token.map(str::to_string))
            .customize()
            .config_override(Self::metadata_timeouts())
            .send()
            .await
            .map_err(R2Error::from_sdk)?;
        let next_token =
            check_next_token(out.is_truncated(), out.next_continuation_token(), token)?;
        Ok(ListPage {
            prefix: prefix.to_string(),
            folders: Vec::new(),
            objects: out.contents().iter().filter_map(object_entry).collect(),
            next_token,
        })
    }

    /// Visit every object under `prefix` (no delimiter). Stops early when the
    /// visitor returns `false` or the token is cancelled.
    pub async fn for_each_object<F>(
        &self,
        bucket: &str,
        prefix: &str,
        cancel: &CancellationToken,
        mut visit: F,
    ) -> Result<(), R2Error>
    where
        F: FnMut(ObjectEntry) -> bool,
    {
        let mut token: Option<String> = None;
        loop {
            if cancel.is_cancelled() {
                return Err(R2Error::Cancelled);
            }
            let page = tokio::select! {
                () = cancel.cancelled() => return Err(R2Error::Cancelled),
                page = self.search_prefix_page(bucket, prefix, token.as_deref()) => page?,
            };
            for obj in page.objects {
                if !visit(obj) {
                    return Ok(());
                }
            }
            match page.next_token {
                Some(t) => token = Some(t),
                None => return Ok(()),
            }
        }
    }
}

/// Fail on a truncated page without a token, or a token that repeats.
pub(crate) fn check_next_token(
    is_truncated: Option<bool>,
    next: Option<&str>,
    previous: Option<&str>,
) -> Result<Option<String>, R2Error> {
    if !is_truncated.unwrap_or(false) {
        return Ok(None);
    }
    match next {
        None | Some("") => Err(R2Error::Protocol {
            detail: "listing was truncated without a continuation token".to_string(),
        }),
        Some(t) if Some(t) == previous => Err(R2Error::Protocol {
            detail: "listing repeated a continuation token".to_string(),
        }),
        Some(t) => Ok(Some(t.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pagination_guards() {
        assert_eq!(check_next_token(Some(false), None, None).unwrap(), None);
        assert_eq!(check_next_token(None, Some("x"), None).unwrap(), None);
        assert_eq!(
            check_next_token(Some(true), Some("b"), Some("a"))
                .unwrap()
                .as_deref(),
            Some("b")
        );
        assert!(check_next_token(Some(true), None, None).is_err());
        assert!(check_next_token(Some(true), Some(""), None).is_err());
        assert!(check_next_token(Some(true), Some("a"), Some("a")).is_err());
    }
}
