//! Public links for objects (PLAN.md 8.9): a manual public base URL first,
//! otherwise a custom domain or the `r2.dev` domain read from the Cloudflare
//! API with the connection's optional API token.
//!
//! Lookups run only when the UI asks for a link (no polling) and are cached
//! in memory per connection and bucket for a short time. A cached result is
//! dropped as soon as the connection's settings or keys change.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use r2dock_core::AppError;
use r2dock_core::public_domain::{BucketDomains, resolve_link};
use r2dock_r2::cfapi::{CfApiClient, CfApiError};
use r2dock_secrets::SecretError;
use uuid::Uuid;

use crate::connections::ConnectionService;
use crate::dto::{PublicLink, PublicLinkStatus};

/// How long found domains are reused.
const FOUND_TTL: Duration = Duration::from_secs(5 * 60);
/// How long a failed lookup is reused before trying again.
const FAILED_TTL: Duration = Duration::from_secs(30);
/// At most this many buckets are cached.
const CACHE_CAP: usize = 256;

type CacheKey = (Uuid, String);

#[derive(Clone)]
enum Lookup {
    Found(BucketDomains),
    Failed(PublicLinkStatus),
}

struct Cached {
    at: Instant,
    generation: u64,
    lookup: Lookup,
}

impl Cached {
    fn fresh(&self, generation: u64) -> bool {
        let ttl = match self.lookup {
            Lookup::Found(_) => FOUND_TTL,
            Lookup::Failed(_) => FAILED_TTL,
        };
        self.generation == generation && self.at.elapsed() < ttl
    }
}

pub struct PublicLinks {
    /// `None` when the HTTP client could not be built on this system.
    api: Option<CfApiClient>,
    cache: Mutex<HashMap<CacheKey, Cached>>,
    /// One lookup at a time per bucket, so a burst of requests makes one call.
    inflight: Mutex<HashMap<CacheKey, Arc<tokio::sync::Mutex<()>>>>,
}

impl PublicLinks {
    pub fn new(api: Option<CfApiClient>) -> Self {
        Self {
            api,
            cache: Mutex::new(HashMap::new()),
            inflight: Mutex::new(HashMap::new()),
        }
    }

    /// The public link for one object, with where it came from.
    pub async fn resolve(
        &self,
        connections: &Arc<ConnectionService>,
        id: Uuid,
        bucket: &str,
        key: &str,
    ) -> Result<PublicLink, AppError> {
        let cfg = connections.get(id)?;
        let found = |url: String, source| PublicLink {
            url: Some(url),
            source: Some(source),
            status: PublicLinkStatus::Found,
        };
        let none = |status| PublicLink {
            url: None,
            source: None,
            status,
        };
        if let Some(manual) = cfg.public_base_urls.get(bucket) {
            return Ok(resolve_link(Some(manual), None, key)
                .map(|(url, source)| found(url, source))
                .unwrap_or_else(|| none(PublicLinkStatus::NotConfigured)));
        }
        if cfg.api_token.is_none() {
            return Ok(none(PublicLinkStatus::NotConfigured));
        }
        Ok(match self.domains(connections, id, bucket).await {
            Lookup::Found(domains) => match resolve_link(None, Some(&domains), key) {
                Some((url, source)) => found(url, source),
                None => none(PublicLinkStatus::NoDomain),
            },
            Lookup::Failed(status) => none(status),
        })
    }

    async fn domains(
        &self,
        connections: &Arc<ConnectionService>,
        id: Uuid,
        bucket: &str,
    ) -> Lookup {
        let key: CacheKey = (id, bucket.to_string());
        if let Some(hit) = self.cached(&key, connections.generation_of(id)) {
            return hit;
        }
        let gate = self.inflight.lock().entry(key.clone()).or_default().clone();
        let _turn = gate.lock().await;
        // Another request may have finished the lookup while this one waited.
        let generation = connections.generation_of(id);
        if let Some(hit) = self.cached(&key, generation) {
            return hit;
        }
        let lookup = self.fetch(connections, id, bucket).await;
        self.store(key.clone(), generation, lookup.clone());
        self.inflight.lock().remove(&key);
        lookup
    }

    fn cached(&self, key: &CacheKey, generation: u64) -> Option<Lookup> {
        self.cache
            .lock()
            .get(key)
            .filter(|c| c.fresh(generation))
            .map(|c| c.lookup.clone())
    }

    fn store(&self, key: CacheKey, generation: u64, lookup: Lookup) {
        let mut cache = self.cache.lock();
        cache.retain(|_, c| c.at.elapsed() < FOUND_TTL);
        if cache.len() >= CACHE_CAP
            && let Some(oldest) = cache
                .iter()
                .min_by_key(|(_, c)| c.at)
                .map(|(k, _)| k.clone())
        {
            cache.remove(&oldest);
        }
        cache.insert(
            key,
            Cached {
                at: Instant::now(),
                generation,
                lookup,
            },
        );
    }

    async fn fetch(&self, connections: &Arc<ConnectionService>, id: Uuid, bucket: &str) -> Lookup {
        let Ok(cfg) = connections.get(id) else {
            return Lookup::Failed(PublicLinkStatus::NotConfigured);
        };
        let token = match connections.api_token(id).await {
            Ok(Some(token)) => token,
            Ok(None) => return Lookup::Failed(PublicLinkStatus::NotConfigured),
            Err(SecretError::NotFound) => return Lookup::Failed(PublicLinkStatus::TokenMissing),
            Err(_) => return Lookup::Failed(PublicLinkStatus::Unavailable),
        };
        let Some(api) = &self.api else {
            return Lookup::Failed(PublicLinkStatus::Unavailable);
        };
        match api
            .bucket_domains(&cfg.account_id, bucket, cfg.jurisdiction, &token)
            .await
        {
            Ok(domains) => Lookup::Found(domains),
            Err(e) if e.is_permission() => Lookup::Failed(PublicLinkStatus::TokenDenied),
            Err(e) => {
                tracing::debug!(error = %e, "public domain lookup failed");
                Lookup::Failed(match e {
                    CfApiError::NotFound => PublicLinkStatus::NoDomain,
                    _ => PublicLinkStatus::Unavailable,
                })
            }
        }
    }
}

#[cfg(test)]
mod tests;
