//! Job-level retries (PLAN.md 8.11). The SDK already retries each request up
//! to 3 times; these retries cover a whole job and use longer waits.

use std::time::Duration;

use r2dock_core::{AppError, ErrorCode};
use r2dock_r2::R2Error;

/// Maximum attempts per job, including the first.
pub const MAX_ATTEMPTS: u32 = 3;

/// Backoff before attempt `attempt` (2 = first retry): 2 s, 8 s, 30 s, with jitter.
pub fn backoff(attempt: u32) -> Duration {
    let base = match attempt {
        0..=2 => 2.0,
        3 => 8.0,
        _ => 30.0,
    };
    let jitter: f64 = rand::random::<f64>() * 0.4 + 0.8;
    Duration::from_secs_f64(base * jitter)
}

/// A job failure with enough information to decide what happens next.
#[derive(Debug, Clone)]
pub struct JobError {
    pub app: AppError,
    pub retryable: bool,
    /// The credentials were rejected; stop all jobs of the connection.
    pub auth: bool,
    pub cancelled: bool,
}

impl JobError {
    pub fn fatal(app: AppError) -> Self {
        Self {
            app,
            retryable: false,
            auth: false,
            cancelled: false,
        }
    }

    pub fn retryable(app: AppError) -> Self {
        Self {
            app: app.retryable(true),
            retryable: true,
            auth: false,
            cancelled: false,
        }
    }

    pub fn cancelled() -> Self {
        Self {
            app: AppError::new(ErrorCode::Cancelled, "Cancelled."),
            retryable: false,
            auth: false,
            cancelled: true,
        }
    }

    pub fn file_changed() -> Self {
        Self::fatal(AppError::new(
            ErrorCode::FileChanged,
            "File changed while uploading. Upload again to send the latest version.",
        ))
    }

    pub fn io(err: &std::io::Error, action: &str) -> Self {
        let app = r2dock_core::app_error::io_error(err, action);
        // Disk full and permission problems do not fix themselves.
        let retryable = matches!(
            err.kind(),
            std::io::ErrorKind::Interrupted
                | std::io::ErrorKind::TimedOut
                | std::io::ErrorKind::WouldBlock
        );
        Self {
            app: app.retryable(retryable),
            retryable,
            auth: false,
            cancelled: false,
        }
    }
}

impl From<R2Error> for JobError {
    fn from(err: R2Error) -> Self {
        if err == R2Error::Cancelled {
            return Self::cancelled();
        }
        // A bad digest means the bytes changed between hashing and sending.
        if let R2Error::Service { code, .. } = &err
            && (code == "BadDigest" || code == "InvalidDigest")
        {
            return Self::file_changed();
        }
        Self {
            app: err.to_app_error(),
            retryable: err.is_retryable(),
            auth: err.is_auth_failure(),
            cancelled: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_ranges() {
        for _ in 0..50 {
            let b = backoff(2).as_secs_f64();
            assert!((1.6..=2.4).contains(&b));
            let c = backoff(3).as_secs_f64();
            assert!((6.4..=9.6).contains(&c));
            let d = backoff(9).as_secs_f64();
            assert!((24.0..=36.0).contains(&d));
        }
    }

    #[test]
    fn classification() {
        assert!(JobError::from(R2Error::Timeout).retryable);
        let auth = JobError::from(R2Error::SignatureMismatch { detail: None });
        assert!(auth.auth && !auth.retryable);
        assert!(JobError::from(R2Error::Cancelled).cancelled);
        let digest = JobError::from(R2Error::Service {
            code: "BadDigest".into(),
            status: 400,
            detail: None,
        });
        assert_eq!(digest.app.code, ErrorCode::FileChanged);
        let disk = JobError::io(
            &std::io::Error::from(std::io::ErrorKind::StorageFull),
            "write",
        );
        assert!(!disk.retryable);
        assert_eq!(disk.app.code, ErrorCode::DiskFull);
    }
}
