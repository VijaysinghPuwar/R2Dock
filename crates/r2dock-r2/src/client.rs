//! Client construction (PLAN.md 8.1).

use std::time::Duration;

use aws_sdk_s3::config::retry::RetryConfig;
use aws_sdk_s3::config::timeout::TimeoutConfig;
use aws_sdk_s3::config::{
    BehaviorVersion, Credentials, Region, RequestChecksumCalculation, ResponseChecksumValidation,
    StalledStreamProtectionConfig,
};
use r2dock_core::connections::{Jurisdiction, endpoint_url};
use secrecy::{ExposeSecret, SecretString};

/// Connect timeout for every request.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Per-attempt timeout for metadata and listing calls.
pub const METADATA_ATTEMPT_TIMEOUT: Duration = Duration::from_secs(30);
/// A transfer stream that moves no bytes for this long is treated as stalled.
pub const STALL_GRACE: Duration = Duration::from_secs(60);

/// Everything needed to build a client. The secret is only exposed to the SDK.
pub struct ClientParams {
    pub account_id: String,
    pub access_key_id: String,
    pub secret: SecretString,
    pub jurisdiction: Jurisdiction,
    /// Used by tests to point at a local S3-compatible server.
    pub endpoint_override: Option<String>,
}

impl std::fmt::Debug for ClientParams {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClientParams")
            .field("account_id", &self.account_id)
            .field("jurisdiction", &self.jurisdiction)
            .finish_non_exhaustive()
    }
}

/// A configured S3 client for one R2 account.
#[derive(Clone, Debug)]
pub struct R2Client {
    pub(crate) inner: aws_sdk_s3::Client,
}

impl R2Client {
    pub fn new(params: &ClientParams) -> Self {
        let endpoint = params
            .endpoint_override
            .clone()
            .unwrap_or_else(|| endpoint_url(&params.account_id, params.jurisdiction));
        let credentials = Credentials::new(
            params.access_key_id.clone(),
            params.secret.expose_secret().to_string(),
            None,
            None,
            "r2dock",
        );
        let timeouts = TimeoutConfig::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .build();
        let config = aws_sdk_s3::Config::builder()
            .behavior_version(BehaviorVersion::latest())
            .region(Region::new("auto"))
            .endpoint_url(endpoint)
            .credentials_provider(credentials)
            .force_path_style(true)
            .request_checksum_calculation(RequestChecksumCalculation::WhenRequired)
            .response_checksum_validation(ResponseChecksumValidation::WhenRequired)
            .retry_config(RetryConfig::standard().with_max_attempts(3))
            .timeout_config(timeouts)
            .stalled_stream_protection(
                StalledStreamProtectionConfig::enabled()
                    .grace_period(STALL_GRACE)
                    .build(),
            )
            .build();
        Self {
            inner: aws_sdk_s3::Client::from_conf(config),
        }
    }

    /// The underlying SDK client, for the transfer engine.
    pub fn sdk(&self) -> &aws_sdk_s3::Client {
        &self.inner
    }

    /// Config override that applies the metadata attempt timeout.
    pub(crate) fn metadata_timeouts() -> aws_sdk_s3::config::Builder {
        aws_sdk_s3::Config::builder().timeout_config(
            TimeoutConfig::builder()
                .connect_timeout(CONNECT_TIMEOUT)
                .operation_attempt_timeout(METADATA_ATTEMPT_TIMEOUT)
                .build(),
        )
    }
}
