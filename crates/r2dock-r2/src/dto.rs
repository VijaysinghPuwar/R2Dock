//! Data shapes returned to the frontend. Object names are untrusted and are
//! always rendered as text; `display_name` makes hidden characters visible.

use std::collections::BTreeMap;

use serde::Serialize;
use ts_rs::TS;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BucketEntry {
    pub name: String,
    /// RFC 3339 timestamp, when R2 reports one.
    pub created: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FolderEntry {
    /// Full prefix including the trailing slash.
    pub prefix: String,
    pub name: String,
    pub display_name: String,
    pub unusual: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ObjectEntry {
    pub key: String,
    pub name: String,
    pub display_name: String,
    #[ts(type = "number")]
    pub size: u64,
    pub last_modified: Option<String>,
    pub etag: Option<String>,
    pub storage_class: Option<String>,
    pub unusual: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ListPage {
    pub prefix: String,
    pub folders: Vec<FolderEntry>,
    pub objects: Vec<ObjectEntry>,
    pub next_token: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ObjectDetails {
    pub key: String,
    pub name: String,
    pub display_name: String,
    #[ts(type = "number")]
    pub size: u64,
    pub content_type: Option<String>,
    pub last_modified: Option<String>,
    pub etag: Option<String>,
    /// True only when the ETag is known to be an MD5 of the whole file.
    pub etag_is_md5: bool,
    pub storage_class: String,
    pub cache_control: Option<String>,
    pub content_encoding: Option<String>,
    pub metadata: BTreeMap<String, String>,
}

/// One key that could not be deleted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct KeyFailure {
    pub key: String,
    pub message: String,
}

/// Total size of everything under a prefix.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PrefixSize {
    #[ts(type = "number")]
    pub objects: u64,
    #[ts(type = "number")]
    pub bytes: u64,
    pub calculated_at: String,
}
