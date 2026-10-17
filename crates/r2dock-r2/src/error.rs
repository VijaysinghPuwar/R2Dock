//! Mapping SDK errors to R2Dock errors (PLAN.md 8.11).
//!
//! Messages never contain secrets, signatures, or raw server responses.

use aws_sdk_s3::error::{ProvideErrorMetadata, SdkError};
use aws_sdk_s3::operation::RequestId;
use r2dock_core::{AppError, ErrorCode};
use thiserror::Error;

#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum R2Error {
    #[error("access key id not recognized")]
    InvalidAccessKey { detail: Option<String> },
    #[error("secret does not match")]
    SignatureMismatch { detail: Option<String> },
    #[error("access denied")]
    AccessDenied { detail: Option<String> },
    #[error("bucket not found")]
    NoSuchBucket { detail: Option<String> },
    #[error("object not found")]
    NoSuchKey { detail: Option<String> },
    #[error("precondition failed")]
    PreconditionFailed { detail: Option<String> },
    #[error("rate limited")]
    RateLimited { detail: Option<String> },
    #[error("server error")]
    Server { status: u16, detail: Option<String> },
    #[error("network failure")]
    Network,
    #[error("timeout")]
    Timeout,
    #[error("unexpected response from R2")]
    Protocol { detail: String },
    #[error("R2 error {code}")]
    Service {
        code: String,
        status: u16,
        detail: Option<String>,
    },
    #[error("request could not be built")]
    Construction,
    #[error("cancelled")]
    Cancelled,
}

fn detail_of<E: ProvideErrorMetadata + RequestId>(err: &E, status: u16) -> Option<String> {
    let code = err.code().unwrap_or("HTTP");
    match err.request_id() {
        Some(id) => Some(format!("{code} ({status}), request id {id}")),
        None => Some(format!("{code} ({status})")),
    }
}

impl R2Error {
    /// Map any SDK operation error.
    pub fn from_sdk<E, R>(err: SdkError<E, R>) -> Self
    where
        E: ProvideErrorMetadata + RequestId + std::fmt::Debug,
        R: HasStatus,
    {
        match err {
            SdkError::TimeoutError(_) => Self::Timeout,
            SdkError::DispatchFailure(d) => {
                if d.is_timeout() {
                    Self::Timeout
                } else {
                    Self::Network
                }
            }
            SdkError::ResponseError(_) => Self::Protocol {
                detail: "response could not be read".to_string(),
            },
            SdkError::ConstructionFailure(_) => Self::Construction,
            SdkError::ServiceError(se) => {
                let status = se.raw().status_code();
                Self::from_service(se.err(), status)
            }
            _ => Self::Network,
        }
    }

    fn from_service<E: ProvideErrorMetadata + RequestId>(err: &E, status: u16) -> Self {
        let detail = detail_of(err, status);
        match err.code() {
            Some("InvalidAccessKeyId") => Self::InvalidAccessKey { detail },
            Some("SignatureDoesNotMatch") => Self::SignatureMismatch { detail },
            Some("AccessDenied") | Some("Unauthorized") => Self::AccessDenied { detail },
            Some("NoSuchBucket") => Self::NoSuchBucket { detail },
            Some("NoSuchKey") | Some("NotFound") => Self::NoSuchKey { detail },
            Some("PreconditionFailed") => Self::PreconditionFailed { detail },
            Some("SlowDown") | Some("TooManyRequests") => Self::RateLimited { detail },
            _ => match status {
                401 | 403 => Self::AccessDenied { detail },
                404 => Self::NoSuchKey { detail },
                412 => Self::PreconditionFailed { detail },
                429 => Self::RateLimited { detail },
                500..=599 => Self::Server { status, detail },
                _ => Self::Service {
                    code: err.code().unwrap_or("Unknown").to_string(),
                    status,
                    detail,
                },
            },
        }
    }

    /// Whether the same request may succeed if repeated later.
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            Self::RateLimited { .. }
                | Self::Server { .. }
                | Self::Network
                | Self::Timeout
                | Self::Protocol { .. }
        )
    }

    /// Whether this error means the credentials themselves are bad.
    pub fn is_auth_failure(&self) -> bool {
        matches!(
            self,
            Self::InvalidAccessKey { .. } | Self::SignatureMismatch { .. }
        )
    }

    pub fn detail(&self) -> Option<&str> {
        match self {
            Self::InvalidAccessKey { detail }
            | Self::SignatureMismatch { detail }
            | Self::AccessDenied { detail }
            | Self::NoSuchBucket { detail }
            | Self::NoSuchKey { detail }
            | Self::PreconditionFailed { detail }
            | Self::RateLimited { detail }
            | Self::Server { detail, .. }
            | Self::Service { detail, .. } => detail.as_deref(),
            Self::Protocol { detail } => Some(detail),
            _ => None,
        }
    }

    /// Convert to the IPC error shape with plain wording.
    pub fn to_app_error(&self) -> AppError {
        let (code, message) = match self {
            Self::InvalidAccessKey { .. } => (
                ErrorCode::InvalidAccessKey,
                "The Access Key ID was not recognized. Check that you copied the R2 token's Access Key ID.",
            ),
            Self::SignatureMismatch { .. } => (
                ErrorCode::SignatureMismatch,
                "The Secret Access Key does not match this Access Key ID. Paste the secret again or create a new token.",
            ),
            Self::AccessDenied { .. } => (ErrorCode::AccessDenied, "This key cannot do that."),
            Self::NoSuchBucket { .. } => (
                ErrorCode::NoSuchBucket,
                "Bucket not found, or this key cannot see it.",
            ),
            Self::NoSuchKey { .. } => (ErrorCode::NoSuchKey, "File no longer exists."),
            Self::PreconditionFailed { .. } => (
                ErrorCode::AlreadyExists,
                "Already exists, or the object changed.",
            ),
            Self::RateLimited { .. } => {
                (ErrorCode::RateLimited, "R2 is busy. Try again in a moment.")
            }
            Self::Server { .. } => (ErrorCode::ServerError, "R2 returned an error. Try again."),
            Self::Network => (
                ErrorCode::Network,
                "Could not connect. Check your internet connection or proxy settings, then try again.",
            ),
            Self::Timeout => (ErrorCode::Timeout, "R2 did not respond in time. Try again."),
            Self::Protocol { .. } => (
                ErrorCode::ServerError,
                "R2 sent an unexpected response. Try again.",
            ),
            Self::Service { .. } => (ErrorCode::ServerError, "R2 could not complete the request."),
            Self::Construction => (ErrorCode::Internal, "The request could not be prepared."),
            Self::Cancelled => (ErrorCode::Cancelled, "Cancelled."),
        };
        let mut app = AppError::new(code, message).retryable(self.is_retryable());
        if let Some(d) = self.detail() {
            app = app.with_detail(d.to_string());
        }
        app
    }
}

/// Access to the HTTP status of a raw response, abstracted for testing.
pub trait HasStatus {
    fn status_code(&self) -> u16;
}

impl HasStatus for aws_smithy_runtime_api::http::Response {
    fn status_code(&self) -> u16 {
        self.status().as_u16()
    }
}

impl From<R2Error> for AppError {
    fn from(err: R2Error) -> Self {
        err.to_app_error()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aws_sdk_s3::error::ErrorMetadata;

    struct Fake(ErrorMetadata);
    impl ProvideErrorMetadata for Fake {
        fn meta(&self) -> &ErrorMetadata {
            &self.0
        }
    }
    impl RequestId for Fake {
        fn request_id(&self) -> Option<&str> {
            Some("req-1")
        }
    }

    fn fake(code: Option<&str>) -> Fake {
        let mut b = ErrorMetadata::builder();
        if let Some(c) = code {
            b = b.code(c);
        }
        Fake(b.message("raw server text with secrets?").build())
    }

    #[test]
    fn maps_codes() {
        assert!(matches!(
            R2Error::from_service(&fake(Some("InvalidAccessKeyId")), 403),
            R2Error::InvalidAccessKey { .. }
        ));
        assert!(matches!(
            R2Error::from_service(&fake(Some("SignatureDoesNotMatch")), 403),
            R2Error::SignatureMismatch { .. }
        ));
        assert!(matches!(
            R2Error::from_service(&fake(Some("NoSuchBucket")), 404),
            R2Error::NoSuchBucket { .. }
        ));
        assert!(matches!(
            R2Error::from_service(&fake(None), 404),
            R2Error::NoSuchKey { .. }
        ));
        assert!(matches!(
            R2Error::from_service(&fake(None), 412),
            R2Error::PreconditionFailed { .. }
        ));
        assert!(matches!(
            R2Error::from_service(&fake(None), 429),
            R2Error::RateLimited { .. }
        ));
        assert!(matches!(
            R2Error::from_service(&fake(None), 503),
            R2Error::Server { status: 503, .. }
        ));
        assert!(matches!(
            R2Error::from_service(&fake(Some("Weird")), 400),
            R2Error::Service { .. }
        ));
    }

    #[test]
    fn retry_classes() {
        assert!(R2Error::Timeout.is_retryable());
        assert!(
            R2Error::Server {
                status: 500,
                detail: None
            }
            .is_retryable()
        );
        assert!(!R2Error::AccessDenied { detail: None }.is_retryable());
        assert!(R2Error::SignatureMismatch { detail: None }.is_auth_failure());
    }

    #[test]
    fn app_error_has_no_raw_message() {
        let err = R2Error::from_service(&fake(Some("AccessDenied")), 403).to_app_error();
        assert!(!err.message.contains("raw server text"));
        assert!(
            !err.detail
                .as_deref()
                .unwrap_or("")
                .contains("raw server text")
        );
        assert_eq!(
            err.detail.as_deref(),
            Some("AccessDenied (403), request id req-1")
        );
    }
}
