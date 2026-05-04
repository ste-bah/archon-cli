use std::collections::BTreeMap;

use chrono::{DateTime, NaiveDate, Utc};
use regex::Regex;
use serde::{Deserialize, Serialize};

use super::spoof_cache::{cache_file, read_cache, read_fresh_cache, touch_cache, write_cache};

const MAX_SCHEMA_VERSION: u32 = 1;
const BUNDLED_MANIFEST: &str = include_str!("../../../resources/codex-compat.json");
const RESERVED_HEADERS: &[&str] = &[
    "authorization",
    "chatgpt-account-id",
    "content-type",
    "accept",
    "session_id",
    "x-client-request-id",
    "user-agent",
    "openai-beta",
];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SpoofConfig {
    pub originator: String,
    pub user_agent: String,
    pub client_id: String,
    pub openai_beta: String,
    #[serde(default)]
    pub extra_headers: BTreeMap<String, String>,
}

impl Default for SpoofConfig {
    fn default() -> Self {
        Self {
            originator: "openclaw".into(),
            user_agent: format!("openclaw/{}", env!("CARGO_PKG_VERSION")),
            client_id: "app_EMoamEEZ73f0CkXaXp7hrann".into(),
            openai_beta: "responses=experimental".into(),
            extra_headers: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CodexProviderConfig {
    pub enabled: bool,
    pub spoof: CodexSpoofPartialConfig,
    pub manifest: CodexManifestConfig,
}

impl Default for CodexProviderConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            spoof: CodexSpoofPartialConfig::default(),
            manifest: CodexManifestConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct CodexSpoofPartialConfig {
    pub originator: Option<String>,
    pub user_agent: Option<String>,
    pub client_id: Option<String>,
    pub openai_beta: Option<String>,
    pub extra_headers: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CodexManifestConfig {
    pub fetch_url: String,
    pub ttl_seconds: u64,
    pub cache_dir: String,
}

impl Default for CodexManifestConfig {
    fn default() -> Self {
        Self {
            fetch_url: "https://raw.githubusercontent.com/ste-bah/archon-cli/main/crates/archon-llm/resources/codex-compat.json".into(),
            ttl_seconds: 21_600,
            cache_dir: "~/.archon/cache/codex-compat".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolvedSource {
    EnvVar,
    ConfigToml,
    FetchedManifest {
        fetched_at: DateTime<Utc>,
        url: String,
    },
    BundledManifest,
}

#[derive(Debug, Clone)]
pub struct SpoofResolution {
    pub config: SpoofConfig,
    pub primary_source: ResolvedSource,
    pub per_field_fallbacks: BTreeMap<String, ResolvedSource>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub schema_version: u32,
    pub codex_cli_version: String,
    pub openclaw_version: String,
    pub spoof: SpoofConfig,
    pub compatible_through: String,
    pub minimum_archon_version: String,
}

#[derive(Debug, thiserror::Error)]
pub enum SpoofError {
    #[error("Codex provider disabled")]
    Disabled,
    #[error("impersonation rejected for {field}: {reason}")]
    ImpersonationRejected {
        field: String,
        value: String,
        reason: String,
    },
    #[error("manifest fetch failed: {0}")]
    FetchFailed(String),
    #[error("manifest cache corrupt: {0}")]
    CacheCorrupt(String),
    #[error("bundled manifest missing or invalid: {0}")]
    BundledMissing(String),
    #[error("minimum archon version too low: {0}")]
    MinVersionTooLow(String),
    #[error("unsupported manifest schema version: {0}")]
    SchemaVersionUnsupported(u32),
    #[error("validation error: {0}")]
    Validation(String),
}

impl SpoofConfig {
    pub fn validate(&self) -> Result<(), SpoofError> {
        validate_regex("originator", &self.originator, r"^[a-zA-Z0-9_-]{1,64}$")?;
        reject_impersonation("originator", &self.originator)?;
        validate_len_no_crlf("user_agent", &self.user_agent, 1, 256)?;
        reject_impersonation("user_agent", &self.user_agent)?;
        validate_regex("client_id", &self.client_id, r"^app_[A-Za-z0-9_-]{16,128}$")?;
        validate_regex("openai_beta", &self.openai_beta, r"^responses=[a-z0-9_-]+$")?;
        validate_extra_headers(&self.extra_headers)?;
        Ok(())
    }
}

impl Manifest {
    pub fn validate(&self) -> Result<(), SpoofError> {
        if self.schema_version > MAX_SCHEMA_VERSION {
            return Err(SpoofError::SchemaVersionUnsupported(self.schema_version));
        }
        if self.schema_version < MAX_SCHEMA_VERSION {
            tracing::warn!("Codex manifest schema is older than supported; using anyway");
        }
        if version_gt(&self.minimum_archon_version, env!("CARGO_PKG_VERSION")) {
            return Err(SpoofError::MinVersionTooLow(
                self.minimum_archon_version.clone(),
            ));
        }
        if let Some(through) = parse_manifest_date(&self.compatible_through)
            && through < Utc::now()
        {
            tracing::warn!("Codex spoof manifest expired; consider refreshing");
        }
        self.spoof.validate()
    }
}

pub async fn resolve(
    config: &CodexProviderConfig,
    http: &reqwest::Client,
) -> Result<SpoofResolution, SpoofError> {
    if disabled_by_env() || !config.enabled {
        return Err(SpoofError::Disabled);
    }
    if mixed_resolution_enabled() {
        return resolve_mixed(config, http).await;
    }

    let bundled = bundled_spoof_or_default()?;
    if any_env_set() {
        return resolution_from_env(&bundled);
    }
    if config.spoof.originator.is_some() && config.spoof.user_agent.is_some() {
        return resolution_from_config(config, &bundled);
    }
    let manifest_cfg = manifest_config_with_env(&config.manifest);
    if let Ok(manifest) = fetch_manifest(http, &manifest_cfg).await {
        let fetched_at = Utc::now();
        let source = ResolvedSource::FetchedManifest {
            fetched_at,
            url: manifest_cfg.fetch_url.clone(),
        };
        let resolved = manifest.spoof;
        resolved.validate()?;
        return Ok(SpoofResolution {
            config: resolved,
            primary_source: source,
            per_field_fallbacks: BTreeMap::new(),
        });
    }
    Ok(SpoofResolution {
        config: bundled,
        primary_source: ResolvedSource::BundledManifest,
        per_field_fallbacks: BTreeMap::new(),
    })
}

pub fn bundled_manifest() -> Result<Manifest, SpoofError> {
    let manifest: Manifest = serde_json::from_str(BUNDLED_MANIFEST)
        .map_err(|e| SpoofError::BundledMissing(e.to_string()))?;
    manifest.validate()?;
    Ok(manifest)
}

pub async fn fetch_manifest(
    http: &reqwest::Client,
    config: &CodexManifestConfig,
) -> Result<Manifest, SpoofError> {
    let cache_path = cache_file(config);
    if let Some(cached) = read_fresh_cache(&cache_path, config.ttl_seconds, &config.fetch_url)? {
        cached.manifest.validate()?;
        return Ok(cached.manifest);
    }

    let response = http
        .get(&config.fetch_url)
        .send()
        .await
        .map_err(|e| SpoofError::FetchFailed(e.to_string()))?;
    if response.status().as_u16() == 304 {
        touch_cache(&cache_path)?;
        let cached = read_cache(&cache_path)?;
        cached.manifest.validate()?;
        return Ok(cached.manifest);
    }
    if !response.status().is_success() {
        return Err(SpoofError::FetchFailed(format!(
            "HTTP {} from {}",
            response.status(),
            config.fetch_url
        )));
    }
    let body = response
        .text()
        .await
        .map_err(|e| SpoofError::FetchFailed(e.to_string()))?;
    let manifest: Manifest =
        serde_json::from_str(&body).map_err(|e| SpoofError::FetchFailed(e.to_string()))?;
    manifest.validate()?;
    write_cache(&cache_path, &manifest, &config.fetch_url)?;
    Ok(manifest)
}

fn resolution_from_env(bundled: &SpoofConfig) -> Result<SpoofResolution, SpoofError> {
    let mut fallbacks = BTreeMap::new();
    let mut config = bundled.clone();
    assign_env(
        "originator",
        "ARCHON_CODEX_ORIGINATOR",
        &mut config.originator,
        &mut fallbacks,
    );
    assign_env(
        "user_agent",
        "ARCHON_CODEX_USER_AGENT",
        &mut config.user_agent,
        &mut fallbacks,
    );
    assign_env(
        "client_id",
        "ARCHON_CODEX_CLIENT_ID",
        &mut config.client_id,
        &mut fallbacks,
    );
    assign_env(
        "openai_beta",
        "ARCHON_CODEX_BETA",
        &mut config.openai_beta,
        &mut fallbacks,
    );
    config.validate()?;
    Ok(SpoofResolution {
        config,
        primary_source: ResolvedSource::EnvVar,
        per_field_fallbacks: fallbacks,
    })
}

fn resolution_from_config(
    cfg: &CodexProviderConfig,
    bundled: &SpoofConfig,
) -> Result<SpoofResolution, SpoofError> {
    let mut fallbacks = BTreeMap::new();
    let mut config = bundled.clone();
    assign_config(
        "originator",
        &cfg.spoof.originator,
        &mut config.originator,
        &mut fallbacks,
    );
    assign_config(
        "user_agent",
        &cfg.spoof.user_agent,
        &mut config.user_agent,
        &mut fallbacks,
    );
    assign_config(
        "client_id",
        &cfg.spoof.client_id,
        &mut config.client_id,
        &mut fallbacks,
    );
    assign_config(
        "openai_beta",
        &cfg.spoof.openai_beta,
        &mut config.openai_beta,
        &mut fallbacks,
    );
    config.extra_headers = cfg.spoof.extra_headers.clone();
    config.validate()?;
    Ok(SpoofResolution {
        config,
        primary_source: ResolvedSource::ConfigToml,
        per_field_fallbacks: fallbacks,
    })
}

async fn resolve_mixed(
    config: &CodexProviderConfig,
    http: &reqwest::Client,
) -> Result<SpoofResolution, SpoofError> {
    let bundled = bundled_spoof_or_default()?;
    let manifest_cfg = manifest_config_with_env(&config.manifest);
    let fetched = fetch_manifest(http, &manifest_cfg)
        .await
        .ok()
        .map(|m| m.spoof);
    let mut resolved = fetched.unwrap_or(bundled);
    merge_partial(&mut resolved, &config.spoof);
    for (env, target) in [
        ("ARCHON_CODEX_ORIGINATOR", &mut resolved.originator),
        ("ARCHON_CODEX_USER_AGENT", &mut resolved.user_agent),
        ("ARCHON_CODEX_CLIENT_ID", &mut resolved.client_id),
        ("ARCHON_CODEX_BETA", &mut resolved.openai_beta),
    ] {
        if let Ok(value) = std::env::var(env)
            && !value.trim().is_empty()
        {
            *target = value;
        }
    }
    resolved.validate()?;
    Ok(SpoofResolution {
        config: resolved,
        primary_source: ResolvedSource::EnvVar,
        per_field_fallbacks: BTreeMap::new(),
    })
}

fn manifest_config_with_env(config: &CodexManifestConfig) -> CodexManifestConfig {
    let mut resolved = config.clone();
    if let Ok(fetch_url) = std::env::var("ARCHON_CODEX_FETCH_URL")
        && !fetch_url.trim().is_empty()
    {
        resolved.fetch_url = fetch_url;
    }
    resolved
}

fn bundled_spoof_or_default() -> Result<SpoofConfig, SpoofError> {
    match bundled_manifest() {
        Ok(manifest) => Ok(manifest.spoof),
        Err(err) => {
            tracing::warn!("falling back to hardcoded Codex spoof config: {err}");
            let fallback = SpoofConfig::default();
            fallback.validate()?;
            Ok(fallback)
        }
    }
}

fn assign_env(
    field: &str,
    env: &str,
    target: &mut String,
    fallbacks: &mut BTreeMap<String, ResolvedSource>,
) {
    match std::env::var(env) {
        Ok(value) if !value.trim().is_empty() => *target = value,
        _ => {
            fallbacks.insert(field.into(), ResolvedSource::BundledManifest);
        }
    }
}

fn assign_config(
    field: &str,
    value: &Option<String>,
    target: &mut String,
    fallbacks: &mut BTreeMap<String, ResolvedSource>,
) {
    if let Some(value) = value {
        *target = value.clone();
    } else {
        fallbacks.insert(field.into(), ResolvedSource::BundledManifest);
    }
}

fn merge_partial(config: &mut SpoofConfig, partial: &CodexSpoofPartialConfig) {
    if let Some(value) = &partial.originator {
        config.originator = value.clone();
    }
    if let Some(value) = &partial.user_agent {
        config.user_agent = value.clone();
    }
    if let Some(value) = &partial.client_id {
        config.client_id = value.clone();
    }
    if let Some(value) = &partial.openai_beta {
        config.openai_beta = value.clone();
    }
    if !partial.extra_headers.is_empty() {
        config.extra_headers = partial.extra_headers.clone();
    }
}

fn disabled_by_env() -> bool {
    std::env::var("ARCHON_CODEX_DISABLED")
        .map(|v| matches!(v.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
        .unwrap_or(false)
}

fn mixed_resolution_enabled() -> bool {
    std::env::var("ARCHON_CODEX_SPOOF_ALLOW_MIXED")
        .map(|v| matches!(v.as_str(), "1" | "true" | "TRUE"))
        .unwrap_or(false)
}

fn any_env_set() -> bool {
    [
        "ARCHON_CODEX_ORIGINATOR",
        "ARCHON_CODEX_USER_AGENT",
        "ARCHON_CODEX_CLIENT_ID",
        "ARCHON_CODEX_BETA",
        "ARCHON_CODEX_FETCH_URL",
    ]
    .iter()
    .any(|key| std::env::var(key).is_ok())
}

fn validate_regex(field: &str, value: &str, pattern: &str) -> Result<(), SpoofError> {
    let re = Regex::new(pattern).map_err(|e| SpoofError::Validation(e.to_string()))?;
    if re.is_match(value) {
        Ok(())
    } else {
        Err(SpoofError::Validation(format!(
            "{field} does not match {pattern}"
        )))
    }
}

fn validate_len_no_crlf(
    field: &str,
    value: &str,
    min: usize,
    max: usize,
) -> Result<(), SpoofError> {
    if value.len() < min || value.len() > max || value.contains('\r') || value.contains('\n') {
        return Err(SpoofError::Validation(format!(
            "{field} length or characters invalid"
        )));
    }
    Ok(())
}

fn validate_extra_headers(headers: &BTreeMap<String, String>) -> Result<(), SpoofError> {
    for (key, value) in headers {
        let lower = key.to_lowercase();
        if RESERVED_HEADERS.contains(&lower.as_str()) {
            return Err(SpoofError::Validation(format!(
                "extra header `{key}` is reserved"
            )));
        }
        validate_len_no_crlf(key, value, 1, 1024)?;
        reject_impersonation(key, value)?;
    }
    Ok(())
}

pub fn reject_impersonation(field: &str, value: &str) -> Result<(), SpoofError> {
    let re = Regex::new(r"(?i)^(ChatGPT|OpenAI)[/_-]")
        .map_err(|e| SpoofError::Validation(e.to_string()))?;
    if re.is_match(value) {
        return Err(SpoofError::ImpersonationRejected {
            field: field.into(),
            value: value.into(),
            reason: "matches OpenAI product naming pattern; operator must not impersonate OpenAI's own products".into(),
        });
    }
    Ok(())
}

fn parse_manifest_date(value: &str) -> Option<DateTime<Utc>> {
    if let Ok(dt) = DateTime::parse_from_rfc3339(value) {
        return Some(dt.with_timezone(&Utc));
    }
    let date = NaiveDate::parse_from_str(value, "%Y-%m-%d").ok()?;
    Some(date.and_hms_opt(23, 59, 59)?.and_utc())
}

fn version_gt(left: &str, right: &str) -> bool {
    parse_version(left) > parse_version(right)
}

fn parse_version(value: &str) -> (u64, u64, u64) {
    let cleaned = value.trim_start_matches('v');
    let mut parts = cleaned.split(['.', '-']).filter_map(|p| p.parse().ok());
    (
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
    )
}
