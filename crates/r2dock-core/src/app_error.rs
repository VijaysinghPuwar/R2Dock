//! The serializable error shape that crosses the IPC boundary.
//!
//! Messages are short, plain, and never contain secrets, signatures, raw
//! server responses, or full local paths.

use serde::Serialize;
use ts_rs::TS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ErrorCode {
    InvalidInput,
    NotFound,
    InvalidAccessKey,
    SignatureMismatch,
    AccessDenied,
    NoSuchBucket,
    NoSuchKey,
    AlreadyExists,
    RateLimited,
    ServerError,
    Network,
    Timeout,
    Tls,
    InvalidAccount,
    DiskFull,
    Io,
    CredentialStoreUnavailable,
    CredentialMissing,
    ConfirmationRequired,
    Cancelled,
    FileChanged,
    Internal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AppError {
    pub code: ErrorCode,
    /// One plain sentence suitable for the UI.
    pub message: String,
    /// Optional technical detail (error code, request id). Never a secret.
    pub detail: Option<String>,
    pub retryable: bool,
    /// Bucket names, object keys, or file names that appear in `message` or
    /// `detail`. Never sent to the frontend; the activity log uses them to
    /// hide those names when the user has not allowed names in the log.
    #[serde(skip)]
    #[ts(skip)]
    pub names: Vec<String>,
}

impl AppError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            detail: None,
            retryable: false,
            names: Vec::new(),
        }
    }

    /// Mark a name that the message or detail contains (see [`Self::names`]).
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.names.push(name.into());
        self
    }

    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    pub fn retryable(mut self, retryable: bool) -> Self {
        self.retryable = retryable;
        self
    }

    pub fn invalid_input(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::InvalidInput, message)
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::NotFound, message)
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Internal, message)
    }
}

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}: {}", self.code, self.message)
    }
}

impl std::error::Error for AppError {}

/// Map a local I/O error to a user-facing error without exposing the path.
pub fn io_error(err: &std::io::Error, action: &str) -> AppError {
    use std::io::ErrorKind;
    match err.kind() {
        ErrorKind::NotFound => AppError::new(
            ErrorCode::NotFound,
            format!("Could not {action}: the file or folder no longer exists."),
        ),
        ErrorKind::PermissionDenied => AppError::new(
            ErrorCode::Io,
            format!(
                "Could not {action}. It may be open in another app or you may not have permission."
            ),
        ),
        ErrorKind::StorageFull => AppError::new(ErrorCode::DiskFull, "Not enough disk space."),
        ErrorKind::AlreadyExists => AppError::new(
            ErrorCode::AlreadyExists,
            "A file with that name already exists.",
        ),
        _ => AppError::new(ErrorCode::Io, format!("Could not {action}."))
            .with_detail(format!("{:?}", err.kind())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn io_errors_never_include_paths() {
        let err = std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "/Users/alice/secret.txt",
        );
        let mapped = io_error(&err, "read file");
        assert!(!mapped.message.contains("alice"));
        assert!(mapped.detail.is_none());
    }

    #[test]
    fn marked_names_never_cross_ipc() {
        let e = AppError::invalid_input("Same name: private/key.txt").with_name("private/key.txt");
        assert_eq!(e.names, vec!["private/key.txt".to_string()]);
        let json = serde_json::to_value(&e).unwrap();
        assert!(json.get("names").is_none(), "{json}");
    }

    #[test]
    fn storage_full_maps_to_disk_full() {
        let err = std::io::Error::from(std::io::ErrorKind::StorageFull);
        assert_eq!(io_error(&err, "write").code, ErrorCode::DiskFull);
    }
}
