//! Public links for objects: which base URL to use and where it came from.
//!
//! A bucket can be reached publicly through a custom domain or the managed
//! `r2.dev` domain, both configured in the Cloudflare dashboard. R2Dock can
//! read them through the Cloudflare API when the user saved an optional API
//! token. A public base URL set by hand in Bucket settings always wins.
//!
//! Domain names come from a remote API, so they are treated as untrusted:
//! only plain host names are accepted and links always use https.

use serde::Serialize;
use ts_rs::TS;

use crate::connections::public_url;

/// Where the public link shown for an object came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum PublicLinkSource {
    /// Set by hand in Bucket settings.
    PublicUrl,
    /// A custom domain connected to the bucket in Cloudflare.
    CustomDomain,
    /// The bucket's managed `r2.dev` subdomain.
    R2Dev,
}

/// One custom domain as reported by the Cloudflare API.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomDomain {
    pub domain: String,
    pub enabled: bool,
    /// Ownership and certificate are both active.
    pub active: bool,
}

/// Public domains detected for one bucket.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BucketDomains {
    pub custom: Vec<CustomDomain>,
    /// The `r2.dev` domain, only when public access through it is enabled.
    pub managed: Option<String>,
}

impl BucketDomains {
    /// The custom domain to use: enabled ones only, fully active ones first,
    /// otherwise in the order Cloudflare returned them.
    pub fn preferred_custom(&self) -> Option<&str> {
        let enabled = || self.custom.iter().filter(|d| d.enabled);
        enabled()
            .find(|d| d.active)
            .or_else(|| enabled().next())
            .map(|d| d.domain.as_str())
    }
}

/// Accept a plain DNS host name such as `media.example.com`.
///
/// Returns it in lowercase. Ports, paths, user info, IP literals, wildcards,
/// and anything outside letters, digits, hyphens, and dots are refused.
pub fn validate_domain_host(raw: &str) -> Option<String> {
    let host = raw.trim().trim_end_matches('.').to_ascii_lowercase();
    if host.is_empty() || host.len() > 253 || !host.contains('.') {
        return None;
    }
    let labels_ok = host.split('.').all(|label| {
        !label.is_empty()
            && label.len() <= 63
            && label
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
            && !label.starts_with('-')
            && !label.ends_with('-')
    });
    // The last label of a real host name is never all digits (rules out IPv4).
    let tld_ok = host
        .rsplit('.')
        .next()
        .is_some_and(|tld| !tld.bytes().all(|b| b.is_ascii_digit()));
    (labels_ok && tld_ok).then_some(host)
}

/// The base URL for objects in a bucket, with its source.
///
/// Order: the manual public base URL, then a detected custom domain, then
/// the `r2.dev` domain when it is enabled.
pub fn resolve_base(
    manual: Option<&str>,
    detected: Option<&BucketDomains>,
) -> Option<(String, PublicLinkSource)> {
    if let Some(base) = manual.filter(|b| !b.trim().is_empty()) {
        return Some((
            base.trim_end_matches('/').to_string(),
            PublicLinkSource::PublicUrl,
        ));
    }
    let detected = detected?;
    if let Some(host) = detected.preferred_custom() {
        return Some((format!("https://{host}"), PublicLinkSource::CustomDomain));
    }
    detected
        .managed
        .as_ref()
        .map(|host| (format!("https://{host}"), PublicLinkSource::R2Dev))
}

/// The public link for one object key, with its source.
pub fn resolve_link(
    manual: Option<&str>,
    detected: Option<&BucketDomains>,
    key: &str,
) -> Option<(String, PublicLinkSource)> {
    resolve_base(manual, detected).map(|(base, source)| (public_url(&base, key), source))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn custom(domain: &str, enabled: bool, active: bool) -> CustomDomain {
        CustomDomain {
            domain: domain.into(),
            enabled,
            active,
        }
    }

    #[test]
    fn domain_validation() {
        let cases: &[(&str, Option<&str>)] = &[
            ("media.example.com", Some("media.example.com")),
            ("Media.Example.COM.", Some("media.example.com")),
            (
                "pub-00000000000000000000000000000000.r2.dev",
                Some("pub-00000000000000000000000000000000.r2.dev"),
            ),
            ("localhost", None),
            ("", None),
            ("media.example.com:8443", None),
            ("media.example.com/path", None),
            ("user@media.example.com", None),
            ("https://media.example.com", None),
            ("*.example.com", None),
            ("-bad.example.com", None),
            ("bad-.example.com", None),
            ("a..example.com", None),
            ("127.0.0.1", None),
            ("media example.com", None),
            ("m\u{e9}dia.example.com", None),
            ("media.example.com\n", Some("media.example.com")),
            ("evil.com\r\nx", None),
        ];
        for (input, want) in cases {
            assert_eq!(
                validate_domain_host(input).as_deref(),
                *want,
                "input {input:?}"
            );
        }
        assert!(validate_domain_host(&format!("{}.com", "a".repeat(64))).is_none());
        assert!(validate_domain_host(&format!("{}.com", "a".repeat(63))).is_some());
    }

    #[test]
    fn preferred_custom_domain() {
        let d = BucketDomains {
            custom: vec![
                custom("off.example.com", false, true),
                custom("pending.example.com", true, false),
                custom("media.example.com", true, true),
            ],
            managed: None,
        };
        assert_eq!(d.preferred_custom(), Some("media.example.com"));
        let d = BucketDomains {
            custom: vec![
                custom("off.example.com", false, true),
                custom("pending.example.com", true, false),
            ],
            managed: None,
        };
        assert_eq!(d.preferred_custom(), Some("pending.example.com"));
        let d = BucketDomains {
            custom: vec![custom("off.example.com", false, true)],
            managed: None,
        };
        assert_eq!(d.preferred_custom(), None);
    }

    #[test]
    fn resolution_order() {
        let managed = "pub-00000000000000000000000000000000.r2.dev";
        let both = BucketDomains {
            custom: vec![custom("media.example.com", true, true)],
            managed: Some(managed.into()),
        };
        let only_managed = BucketDomains {
            custom: vec![custom("off.example.com", false, false)],
            managed: Some(managed.into()),
        };
        let none = BucketDomains::default();

        assert_eq!(
            resolve_base(Some("https://cdn.example.com/"), Some(&both)),
            Some((
                "https://cdn.example.com".into(),
                PublicLinkSource::PublicUrl
            ))
        );
        assert_eq!(
            resolve_base(None, Some(&both)),
            Some((
                "https://media.example.com".into(),
                PublicLinkSource::CustomDomain
            ))
        );
        assert_eq!(
            resolve_base(Some("  "), Some(&only_managed)),
            Some((format!("https://{managed}"), PublicLinkSource::R2Dev))
        );
        assert_eq!(resolve_base(None, Some(&none)), None);
        assert_eq!(resolve_base(None, None), None);
    }

    #[test]
    fn links_encode_each_segment() {
        let d = BucketDomains {
            custom: vec![custom("media.example.com", true, true)],
            managed: None,
        };
        let cases = [
            (
                "Audiobook G/cover art.webp",
                "https://media.example.com/Audiobook%20G/cover%20art.webp",
            ),
            (
                "caf\u{e9}/na\u{ef}ve #1?.mp3",
                "https://media.example.com/caf%C3%A9/na%C3%AFve%20%231%3F.mp3",
            ),
            ("a+b/100%.txt", "https://media.example.com/a%2Bb/100%25.txt"),
            (
                "\u{65e5}\u{672c}/\u{1f3b5}.flac",
                "https://media.example.com/%E6%97%A5%E6%9C%AC/%F0%9F%8E%B5.flac",
            ),
        ];
        for (key, want) in cases {
            let (url, source) = resolve_link(None, Some(&d), key).unwrap();
            assert_eq!(url, want);
            assert_eq!(source, PublicLinkSource::CustomDomain);
        }
    }
}
