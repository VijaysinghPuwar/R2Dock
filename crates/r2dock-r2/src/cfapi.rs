//! A small read-only client for the Cloudflare REST API, used only to find a
//! bucket's public domains (custom domains and the managed `r2.dev` domain).
//!
//! Endpoints (Cloudflare API reference, R2 > Buckets > Domains):
//!
//! ```text
//! GET /accounts/{account_id}/r2/buckets/{bucket_name}/domains/custom
//!   -> { success, errors, messages,
//!        result: { domains: [ { domain, enabled,
//!                               status: { ownership, ssl }, minTLS?, zoneId?, zoneName?, ciphers? } ] } }
//! GET /accounts/{account_id}/r2/buckets/{bucket_name}/domains/managed
//!   -> { success, errors, messages, result: { bucketId, domain, enabled } }
//! ```
//!
//! Both take `Authorization: Bearer <API token>` and an optional
//! `cf-r2-jurisdiction` header (`default`, `eu`, `fedramp`, ...).
//!
//! The token is sent only to the fixed Cloudflare API host, never logged,
//! and never part of an error. Responses are size capped and every domain
//! name is validated before use.

use std::time::Duration;

use r2dock_core::connections::Jurisdiction;
use r2dock_core::public_domain::{BucketDomains, CustomDomain, validate_domain_host};
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use thiserror::Error;

/// The Cloudflare API base URL.
pub const CLOUDFLARE_API: &str = "https://api.cloudflare.com/client/v4";
/// Responses larger than this are refused.
const MAX_BODY: usize = 1024 * 1024;
const TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug, Clone, Copy, Error, PartialEq, Eq)]
pub enum CfApiError {
    /// 401: the token is not valid (wrong, expired, or revoked).
    #[error("the API token was not accepted")]
    Unauthorized,
    /// 403: the token works but may not read this bucket's settings.
    #[error("the API token cannot read this bucket's settings")]
    Forbidden,
    #[error("the bucket was not found through the Cloudflare API")]
    NotFound,
    #[error("rate limited by the Cloudflare API")]
    RateLimited,
    #[error("the Cloudflare API returned a server error")]
    Server,
    #[error("the Cloudflare API could not be reached")]
    Network,
    #[error("the Cloudflare API did not answer in time")]
    Timeout,
    #[error("the Cloudflare API returned an unexpected response")]
    InvalidResponse,
}

impl CfApiError {
    /// The token itself is the problem (as opposed to the network).
    pub fn is_permission(self) -> bool {
        matches!(self, Self::Unauthorized | Self::Forbidden)
    }
}

#[derive(Clone)]
pub struct CfApiClient {
    http: reqwest::Client,
    base: String,
}

impl CfApiClient {
    /// A client for the real Cloudflare API.
    pub fn cloudflare() -> Result<Self, CfApiError> {
        Self::with_base_url(CLOUDFLARE_API, TIMEOUT)
    }

    /// A client for another base URL. Used by tests with a local server.
    #[doc(hidden)]
    pub fn with_base_url(base: &str, timeout: Duration) -> Result<Self, CfApiError> {
        let http = reqwest::Client::builder()
            .https_only(base.starts_with("https://"))
            .redirect(reqwest::redirect::Policy::none())
            .user_agent(concat!("R2Dock/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(timeout)
            .timeout(timeout)
            .build()
            .map_err(|_| CfApiError::Network)?;
        Ok(Self {
            http,
            base: base.trim_end_matches('/').to_string(),
        })
    }

    /// Custom domains and the managed `r2.dev` domain of one bucket.
    ///
    /// A failure to read the custom domains is returned as an error. A
    /// failure to read the managed domain only leaves it out.
    pub async fn bucket_domains(
        &self,
        account_id: &str,
        bucket: &str,
        jurisdiction: Jurisdiction,
        token: &SecretString,
    ) -> Result<BucketDomains, CfApiError> {
        // Both values become part of the URL path: accept only safe characters.
        let safe = |s: &str, extra: u8| {
            !s.is_empty()
                && s.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == extra || b == b'-')
        };
        if !safe(account_id, b'-') || !safe(bucket, b'.') {
            return Err(CfApiError::InvalidResponse);
        }
        let base = format!(
            "{}/accounts/{account_id}/r2/buckets/{bucket}/domains",
            self.base
        );
        let custom_url = format!("{base}/custom");
        let managed_url = format!("{base}/managed");
        let (custom, managed) = tokio::join!(
            self.get(&custom_url, jurisdiction, token),
            self.get(&managed_url, jurisdiction, token),
        );
        let custom = parse_custom(&custom?)?;
        let managed = managed
            .ok()
            .and_then(|body| parse_managed(&body).ok().flatten());
        Ok(BucketDomains { custom, managed })
    }

    async fn get(
        &self,
        url: &str,
        jurisdiction: Jurisdiction,
        token: &SecretString,
    ) -> Result<Vec<u8>, CfApiError> {
        let mut request = self
            .http
            .get(url)
            .bearer_auth(token.expose_secret())
            .header(reqwest::header::ACCEPT, "application/json");
        if let Some(value) = jurisdiction_header(jurisdiction) {
            request = request.header("cf-r2-jurisdiction", value);
        }
        let mut response = request.send().await.map_err(map_send_error)?;
        match response.status().as_u16() {
            200..=299 => {}
            401 => return Err(CfApiError::Unauthorized),
            403 => return Err(CfApiError::Forbidden),
            404 => return Err(CfApiError::NotFound),
            429 => return Err(CfApiError::RateLimited),
            500..=599 => return Err(CfApiError::Server),
            _ => return Err(CfApiError::InvalidResponse),
        }
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(map_send_error)? {
            if body.len() + chunk.len() > MAX_BODY {
                return Err(CfApiError::InvalidResponse);
            }
            body.extend_from_slice(&chunk);
        }
        Ok(body)
    }
}

fn map_send_error(err: reqwest::Error) -> CfApiError {
    if err.is_timeout() {
        CfApiError::Timeout
    } else {
        CfApiError::Network
    }
}

/// The `cf-r2-jurisdiction` header value. The default jurisdiction sends none.
pub fn jurisdiction_header(jurisdiction: Jurisdiction) -> Option<&'static str> {
    match jurisdiction {
        Jurisdiction::Default => None,
        Jurisdiction::Eu => Some("eu"),
        Jurisdiction::Fedramp => Some("fedramp"),
    }
}

#[derive(Deserialize)]
struct Envelope<T> {
    success: bool,
    result: Option<T>,
}

#[derive(Deserialize)]
struct CustomResult {
    #[serde(default)]
    domains: Vec<RawCustom>,
}

#[derive(Deserialize)]
struct RawCustom {
    domain: String,
    #[serde(default)]
    enabled: bool,
    #[serde(default)]
    status: Option<RawStatus>,
}

#[derive(Deserialize)]
struct RawStatus {
    #[serde(default)]
    ownership: Option<String>,
    #[serde(default)]
    ssl: Option<String>,
}

#[derive(Deserialize)]
struct ManagedResult {
    domain: String,
    #[serde(default)]
    enabled: bool,
}

fn envelope<T: for<'de> Deserialize<'de>>(body: &[u8]) -> Result<T, CfApiError> {
    let env: Envelope<T> = serde_json::from_slice(body).map_err(|_| CfApiError::InvalidResponse)?;
    match (env.success, env.result) {
        (true, Some(result)) => Ok(result),
        _ => Err(CfApiError::InvalidResponse),
    }
}

/// Parse a "list custom domains" response. Entries with an invalid domain
/// name are dropped.
pub fn parse_custom(body: &[u8]) -> Result<Vec<CustomDomain>, CfApiError> {
    let result: CustomResult = envelope(body)?;
    Ok(result
        .domains
        .into_iter()
        .filter_map(|raw| {
            let domain = validate_domain_host(&raw.domain)?;
            let active = raw.status.is_some_and(|s| {
                s.ownership.as_deref() == Some("active") && s.ssl.as_deref() == Some("active")
            });
            Some(CustomDomain {
                domain,
                enabled: raw.enabled,
                active,
            })
        })
        .collect())
}

/// Parse a "get managed domain" response: the `r2.dev` domain when public
/// access through it is enabled.
pub fn parse_managed(body: &[u8]) -> Result<Option<String>, CfApiError> {
    let result: ManagedResult = envelope(body)?;
    if !result.enabled {
        return Ok(None);
    }
    Ok(validate_domain_host(&result.domain).filter(|d| d.ends_with(".r2.dev")))
}

#[cfg(test)]
mod tests;
