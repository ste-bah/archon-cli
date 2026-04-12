// TASK-AGS-304: RemoteDiscoverySource — HTTP-based agent discovery with
// moka TTL cache and stale fallback (EC-DISCOVERY-002).

use std::sync::Arc;
use std::time::Duration;

use moka::sync::Cache;
use tracing::{debug, warn};

use crate::agents::catalog::{DiscoveryCatalog, DiscoveryError};
use crate::agents::metadata::{AgentMetadata, AgentState, ResourceReq, SourceKind};
use crate::agents::schema::AgentSchemaValidator;

/// Report from a remote discovery load.
#[derive(Debug)]
pub struct RemoteLoadReport {
    pub loaded: usize,
    pub invalid: usize,
    pub stale: bool,
}

/// Loads agent metadata from a remote HTTP registry.
/// Uses moka TTL cache with stale fallback on HTTP failure.
pub struct RemoteDiscoverySource {
    url: String,
    ttl: Duration,
    cache: Cache<String, Arc<Vec<u8>>>,
    client: reqwest::Client,
    validator: Arc<AgentSchemaValidator>,
}

impl RemoteDiscoverySource {
    pub fn new(url: String, ttl_secs: u64, validator: Arc<AgentSchemaValidator>) -> Self {
        let ttl = Duration::from_secs(ttl_secs);
        let cache = Cache::builder()
            .time_to_live(ttl)
            .build();
        Self {
            url,
            ttl,
            cache,
            client: reqwest::Client::new(),
            validator,
        }
    }

    /// Fetch agent metadata from the remote registry and insert into catalog.
    pub async fn load_all(
        &self,
        catalog: &DiscoveryCatalog,
    ) -> Result<RemoteLoadReport, DiscoveryError> {
        let (bytes, fresh) = self.fetch_with_cache().await?;

        let value: serde_json::Value = serde_json::from_slice(&bytes)
            .map_err(|e| {
                warn!(url = %self.url, "malformed JSON from remote registry");
                DiscoveryError::Parse(format!("JSON parse error from {}: {e}", self.url))
            })?;

        let arr = value
            .as_array()
            .ok_or_else(|| {
                warn!(url = %self.url, "remote registry response is not a JSON array");
                DiscoveryError::Parse(format!("expected JSON array from {}", self.url))
            })?;

        let mut loaded = 0;
        let mut invalid = 0;

        for element in arr {
            let meta = self.parse_element(element, fresh);
            match &meta.state {
                AgentState::Valid | AgentState::Stale => loaded += 1,
                AgentState::Invalid(_) => invalid += 1,
            }
            if let Err(e) = catalog.insert(meta) {
                warn!("failed to insert remote agent: {e}");
            }
        }

        Ok(RemoteLoadReport {
            loaded,
            invalid,
            stale: !fresh,
        })
    }

    /// Fetch bytes from cache or HTTP, with stale fallback.
    async fn fetch_with_cache(&self) -> Result<(Vec<u8>, bool), DiscoveryError> {
        // Check cache first
        if let Some(cached) = self.cache.get(&self.url) {
            debug!(url = %self.url, "serving from cache (fresh)");
            return Ok((cached.as_ref().clone(), true));
        }

        // Try HTTP fetch
        match self.client.get(&self.url).send().await {
            Ok(resp) => {
                if resp.status().is_success() {
                    let bytes = resp
                        .bytes()
                        .await
                        .map_err(|e| DiscoveryError::Http(e.to_string()))?;
                    let bytes = bytes.to_vec();
                    self.cache.insert(self.url.clone(), Arc::new(bytes.clone()));
                    debug!(url = %self.url, len = bytes.len(), "fetched from remote");
                    Ok((bytes, true))
                } else {
                    let status = resp.status();
                    warn!(url = %self.url, status = %status, "HTTP error from remote");
                    self.try_stale_fallback()
                }
            }
            Err(e) => {
                warn!(url = %self.url, error = %e, "HTTP request failed");
                self.try_stale_fallback()
            }
        }
    }

    /// Return stale cache data if available, otherwise error.
    fn try_stale_fallback(&self) -> Result<(Vec<u8>, bool), DiscoveryError> {
        // moka evicts after TTL, so stale data may not be available.
        // We re-check in case it hasn't been evicted yet.
        if let Some(stale) = self.cache.get(&self.url) {
            debug!(url = %self.url, "using stale cache fallback");
            Ok((stale.as_ref().clone(), false))
        } else {
            Err(DiscoveryError::Http(format!(
                "remote fetch failed and no cached data available for {}",
                self.url
            )))
        }
    }

    /// Parse a single JSON element into AgentMetadata.
    fn parse_element(&self, element: &serde_json::Value, fresh: bool) -> AgentMetadata {
        // Validate against schema
        let state = match self.validator.validate(element) {
            Ok(()) => {
                if fresh {
                    AgentState::Valid
                } else {
                    AgentState::Stale
                }
            }
            Err(report) => AgentState::Invalid(report.reason()),
        };

        let name = element["name"]
            .as_str()
            .unwrap_or("unknown-remote")
            .to_string();

        let version = element["version"]
            .as_str()
            .and_then(|s| semver::Version::parse(s).ok())
            .unwrap_or(semver::Version::new(0, 0, 0));

        let category = element["category"]
            .as_str()
            .unwrap_or("remote")
            .to_string();

        AgentMetadata {
            name,
            version,
            description: element["description"]
                .as_str()
                .unwrap_or("")
                .to_string(),
            category,
            tags: element["tags"]
                .as_array()
                .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                .unwrap_or_default(),
            capabilities: element["capabilities"]
                .as_array()
                .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                .unwrap_or_default(),
            input_schema: element
                .get("input_schema")
                .cloned()
                .unwrap_or(serde_json::json!({})),
            output_schema: element
                .get("output_schema")
                .cloned()
                .unwrap_or(serde_json::json!({})),
            resource_requirements: element
                .get("resource_requirements")
                .and_then(|v| serde_json::from_value(v.clone()).ok())
                .unwrap_or_default(),
            dependencies: element
                .get("dependencies")
                .and_then(|d| serde_json::from_value(d.clone()).ok())
                .unwrap_or_default(),
            source_path: std::path::PathBuf::new(),
            source_kind: SourceKind::Remote,
            state,
            loaded_at: chrono::Utc::now(),
        }
    }

    /// Spawn a background retry task that re-fetches at half-TTL intervals
    /// until a successful fresh fetch.
    pub fn spawn_retry(
        self: Arc<Self>,
        catalog: Arc<DiscoveryCatalog>,
    ) -> tokio::task::JoinHandle<()> {
        let retry_interval = self.ttl / 2;
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(retry_interval).await;
                match self.load_all(&catalog).await {
                    Ok(report) if !report.stale => {
                        debug!("background retry succeeded, fresh data loaded");
                        break;
                    }
                    Ok(_) => {
                        debug!("background retry returned stale data, will retry");
                    }
                    Err(e) => {
                        warn!("background retry failed: {e}");
                    }
                }
            }
        })
    }

    /// The URL this source fetches from.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Inserts raw bytes into the cache as if they were a previous fetch.
    /// Intended for testing stale fallback behavior.
    #[doc(hidden)]
    pub fn seed_cache(&self, bytes: Vec<u8>) {
        self.cache.insert(self.url.clone(), Arc::new(bytes));
    }

    /// Invalidate the cache entry for this URL.
    /// Intended for testing TTL expiry behavior.
    #[doc(hidden)]
    pub fn invalidate_cache(&self) {
        self.cache.invalidate(&self.url);
    }
}

impl ResourceReq {
    // Default is already implemented via Default trait in metadata.rs
}
