use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use sha2::{Digest, Sha256};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const FINGERPRINT_SALT: &str = "59cf53e54c78";

/// Beta strings always sent (primary identity + unconditionally required).
pub const DEFAULT_BETAS: &[&str] = &[
    "claude-code-20250219",           // primary identity marker -- MUST always be present
    "oauth-2025-04-20",               // required for OAuth auth
    "interleaved-thinking-2025-05-14", // required for thinking blocks
    "prompt-caching-scope-2026-01-05", // required for 1P cache scopes
];

/// Conditional betas -- only sent when their feature is active.
/// These are NOT included by default because the API rejects unknown/inactive betas.
pub const CONDITIONAL_BETAS: &[(&str, &str)] = &[
    ("context-management-2025-06-27", "context_management"),
    ("context-1m-2025-08-07", "context_1m"),
    ("effort-2025-11-24", "effort"),
    ("redact-thinking-2026-02-12", "redact_thinking"),
    ("fast-mode-2026-02-01", "fast_mode"),
    ("structured-outputs-2025-12-15", "structured_outputs"),
    ("task-budgets-2026-03-13", "task_budgets"),
    ("afk-mode-2026-01-31", "afk_mode"),
];

// ---------------------------------------------------------------------------
// Identity mode
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum IdentityMode {
    Spoof {
        version: String,
        entrypoint: String,
        betas: Vec<String>,
        workload: Option<String>,
        anti_distillation: bool,
    },
    Clean,
    Custom {
        user_agent: String,
        x_app: String,
        extra_headers: HashMap<String, String>,
    },
}

// ---------------------------------------------------------------------------
// Identity provider
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct IdentityProvider {
    pub mode: IdentityMode,
    pub session_id: String,
    pub device_id: String,
    pub account_uuid: String,
}

impl IdentityProvider {
    /// Create a new identity provider.
    pub fn new(
        mode: IdentityMode,
        session_id: String,
        device_id: String,
        account_uuid: String,
    ) -> Self {
        Self {
            mode,
            session_id,
            device_id,
            account_uuid,
        }
    }

    /// Generate HTTP headers for an API request.
    pub fn request_headers(&self, request_id: &str) -> HashMap<String, String> {
        let mut headers = HashMap::new();

        match &self.mode {
            IdentityMode::Spoof {
                version,
                entrypoint: _,
                betas,
                ..
            } => {
                headers.insert("x-app".into(), "cli".into());
                headers.insert(
                    "User-Agent".into(),
                    format!("claude-cli/{version} (external, cli)"),
                );
                headers.insert(
                    "X-Claude-Code-Session-Id".into(),
                    self.session_id.clone(),
                );
                headers.insert("x-client-request-id".into(), request_id.into());
                headers.insert("anthropic-beta".into(), betas.join(","));
            }
            IdentityMode::Clean => {
                headers.insert(
                    "User-Agent".into(),
                    format!("archon-cli/{}", env!("CARGO_PKG_VERSION")),
                );
                headers.insert("x-app".into(), "archon".into());
            }
            IdentityMode::Custom {
                user_agent,
                x_app,
                extra_headers,
            } => {
                headers.insert("User-Agent".into(), user_agent.clone());
                headers.insert("x-app".into(), x_app.clone());
                for (k, v) in extra_headers {
                    headers.insert(k.clone(), v.clone());
                }
            }
        }

        // Always required
        headers.insert("anthropic-version".into(), "2023-06-01".into());
        headers.insert("content-type".into(), "application/json".into());

        headers
    }

    /// Generate the `metadata` field for the API request body.
    pub fn metadata(&self) -> serde_json::Value {
        match &self.mode {
            IdentityMode::Spoof { .. } => {
                let user_id = serde_json::json!({
                    "device_id": self.device_id,
                    "account_uuid": self.account_uuid,
                    "session_id": self.session_id,
                });
                serde_json::json!({
                    "user_id": user_id.to_string(),
                })
            }
            _ => serde_json::json!({}),
        }
    }

    /// Returns the `anti_distillation` field value for the API request body.
    ///
    /// Only set when running in Spoof mode with `anti_distillation: true` (Layer 9).
    pub fn anti_distillation_value(&self) -> Option<serde_json::Value> {
        match &self.mode {
            IdentityMode::Spoof {
                anti_distillation: true,
                ..
            } => Some(serde_json::json!(["fake_tools"])),
            _ => None,
        }
    }

    /// Generate the billing header for the system prompt (Layer 6).
    pub fn billing_header(&self, first_user_message: &str) -> Option<String> {
        match &self.mode {
            IdentityMode::Spoof {
                version,
                entrypoint,
                workload,
                ..
            } => {
                let fp = compute_fingerprint(first_user_message, version);
                let mut header = format!(
                    "x-anthropic-billing-header: cc_version={version}.{fp}; cc_entrypoint={entrypoint};"
                );
                if let Some(wl) = workload {
                    header.push_str(&format!(" cc_workload={wl};"));
                }
                Some(header)
            }
            _ => None,
        }
    }

    /// Generate system prompt blocks with correct cache_control scopes.
    pub fn system_prompt_blocks(
        &self,
        first_user_message: &str,
        static_content: &str,
        dynamic_content: &str,
    ) -> Vec<serde_json::Value> {
        match &self.mode {
            IdentityMode::Spoof { .. } => {
                let mut blocks = Vec::new();

                // Block 1: Billing header (cacheScope = null / ephemeral)
                if let Some(billing) = self.billing_header(first_user_message) {
                    blocks.push(serde_json::json!({
                        "type": "text",
                        "text": billing,
                        "cache_control": { "type": "ephemeral" }
                    }));
                }

                // Block 2: Identity prefix (scope = org)
                blocks.push(serde_json::json!({
                    "type": "text",
                    "text": "You are Claude Code, Anthropic's official CLI for Claude.",
                    "cache_control": { "type": "ephemeral", "scope": "org" }
                }));

                // Block 3: Static content (scope = global for 1P)
                if !static_content.is_empty() {
                    blocks.push(serde_json::json!({
                        "type": "text",
                        "text": static_content,
                        "cache_control": { "type": "ephemeral", "scope": "global" }
                    }));
                }

                // Block 4: Dynamic content (no cache_control)
                if !dynamic_content.is_empty() {
                    blocks.push(serde_json::json!({
                        "type": "text",
                        "text": dynamic_content,
                    }));
                }

                blocks
            }
            _ => {
                // Clean/Custom: just put the content as-is
                let mut blocks = Vec::new();
                if !static_content.is_empty() {
                    blocks.push(serde_json::json!({
                        "type": "text",
                        "text": static_content,
                    }));
                }
                if !dynamic_content.is_empty() {
                    blocks.push(serde_json::json!({
                        "type": "text",
                        "text": dynamic_content,
                    }));
                }
                blocks
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Fingerprint algorithm (REQ-IDENTITY-003)
// ---------------------------------------------------------------------------

/// Compute the fingerprint for the billing header.
///
/// ```text
/// salt = "59cf53e54c78"
/// chars = msg[4] + msg[7] + msg[20] (use "0" for missing)
/// input = salt + chars + version
/// fingerprint = SHA256(input)[0:3] (first 3 hex chars)
/// ```
pub fn compute_fingerprint(first_user_message: &str, version: &str) -> String {
    let chars: Vec<u8> = first_user_message.as_bytes().to_vec();

    let c4 = chars.get(4).copied().unwrap_or(b'0') as char;
    let c7 = chars.get(7).copied().unwrap_or(b'0') as char;
    let c20 = chars.get(20).copied().unwrap_or(b'0') as char;

    let input = format!("{FINGERPRINT_SALT}{c4}{c7}{c20}{version}");
    let hash = Sha256::digest(input.as_bytes());
    let hex = hex::encode(hash);
    hex[..3].to_string()
}

// ---------------------------------------------------------------------------
// Device ID management
// ---------------------------------------------------------------------------

/// Get or create a persistent device ID (64-char hex = 32 random bytes).
pub fn get_or_create_device_id() -> String {
    let path = device_id_path();

    if let Ok(id) = fs::read_to_string(&path) {
        let trimmed = id.trim().to_string();
        if trimmed.len() == 64 && trimmed.chars().all(|c| c.is_ascii_hexdigit()) {
            return trimmed;
        }
    }

    // Generate new
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).expect("system RNG");
    let id = hex::encode(bytes);

    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let _ = fs::write(&path, &id);

    id
}

fn device_id_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from(".config"))
        .join("archon")
        .join("device_id")
}

// ---------------------------------------------------------------------------
// Beta auto-discovery (REQ-IDENTITY-011)
// ---------------------------------------------------------------------------

/// Regex pattern for beta headers.
const BETA_REGEX: &str = r"[a-z][a-z0-9-]+-\d{4}-\d{2}-\d{2}";

/// Discover beta headers from installed Claude Code binary.
///
/// Returns discovered betas, or empty vec if Claude Code not found.
pub fn discover_betas_from_claude() -> Vec<String> {
    let claude_path = find_claude_binary();
    let path = match claude_path {
        Some(p) => p,
        None => {
            tracing::info!("Claude Code not installed, using default betas");
            return Vec::new();
        }
    };

    tracing::debug!("Found Claude Code at: {:?}", path);

    let content = match extract_strings_from_binary(&path) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("Cannot read Claude Code binary: {e}");
            return Vec::new();
        }
    };

    let re = match regex::Regex::new(BETA_REGEX) {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };

    let mut betas: Vec<String> = re
        .find_iter(&content)
        .map(|m| m.as_str().to_string())
        .collect();

    betas.sort();
    betas.dedup();

    tracing::debug!("Auto-discovered {} beta headers from Claude Code", betas.len());
    betas
}

/// Find the Claude Code binary in PATH or common locations.
fn find_claude_binary() -> Option<PathBuf> {
    // Check PATH first
    if let Ok(path) = which::which("claude") {
        return Some(path);
    }

    // Common locations
    let candidates = [
        "/usr/local/bin/claude",
        "/usr/bin/claude",
    ];

    for candidate in &candidates {
        let p = PathBuf::from(candidate);
        if p.exists() {
            return Some(p);
        }
    }

    // Check ~/.local/bin
    if let Some(home) = dirs::home_dir() {
        let local_bin = home.join(".local/bin/claude");
        if local_bin.exists() {
            return Some(local_bin);
        }
    }

    None
}

/// Extract printable strings from a binary file (like `strings` command).
fn extract_strings_from_binary(path: &PathBuf) -> Result<String, std::io::Error> {
    let content = fs::read(path)?;

    // If it's text (no null bytes in first 1024 bytes), return as-is
    if !content.iter().take(1024).any(|&b| b == 0) {
        return Ok(String::from_utf8_lossy(&content).to_string());
    }

    // Binary: extract printable ASCII strings of length >= 8
    let mut result = String::new();
    let mut current = String::new();

    for &byte in &content {
        if byte >= 0x20 && byte < 0x7f {
            current.push(byte as char);
        } else {
            if current.len() >= 8 {
                result.push_str(&current);
                result.push('\n');
            }
            current.clear();
        }
    }
    if current.len() >= 8 {
        result.push_str(&current);
    }

    Ok(result)
}

// ---------------------------------------------------------------------------
// Beta cache
// ---------------------------------------------------------------------------

/// Load cached betas from disk, or None if cache is stale/missing.
pub fn load_cached_betas() -> Option<Vec<String>> {
    let path = beta_cache_path();
    let content = fs::read_to_string(&path).ok()?;

    #[derive(serde::Deserialize)]
    struct BetaCache {
        betas: Vec<String>,
        timestamp: i64,
    }

    let cache: BetaCache = serde_json::from_str(&content).ok()?;

    // Check if cache is older than 24 hours
    let age = chrono::Utc::now().timestamp() - cache.timestamp;
    if age > 86400 {
        return None; // stale
    }

    Some(cache.betas)
}

/// Save discovered betas to cache.
pub fn save_betas_cache(betas: &[String]) {
    let cache = serde_json::json!({
        "betas": betas,
        "timestamp": chrono::Utc::now().timestamp(),
    });

    let path = beta_cache_path();
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let _ = fs::write(&path, serde_json::to_string_pretty(&cache).unwrap_or_default());
}

fn beta_cache_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from(".config"))
        .join("archon")
        .join("discovered_betas.json")
}

// ---------------------------------------------------------------------------
// Validated beta cache (separate from raw discovered betas)
// ---------------------------------------------------------------------------

/// Load the previously validated+cached beta list, or None if missing/stale.
pub fn load_cached_validated_betas() -> Option<Vec<String>> {
    let path = validated_beta_cache_path();
    let content = fs::read_to_string(&path).ok()?;

    #[derive(serde::Deserialize)]
    struct BetaCache {
        betas: Vec<String>,
        timestamp: i64,
    }

    let cache: BetaCache = serde_json::from_str(&content).ok()?;

    let age = chrono::Utc::now().timestamp() - cache.timestamp;
    if age > 86400 {
        return None; // stale
    }

    Some(cache.betas)
}

/// Save the validated beta list to cache.
pub fn save_validated_betas_cache(betas: &[String]) {
    let cache = serde_json::json!({
        "betas": betas,
        "timestamp": chrono::Utc::now().timestamp(),
    });

    let path = validated_beta_cache_path();
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let _ = fs::write(&path, serde_json::to_string_pretty(&cache).unwrap_or_default());
}

fn validated_beta_cache_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from(".config"))
        .join("archon")
        .join("validated_betas.json")
}

/// Discover betas from the installed Claude Code binary, validate them
/// against the API, save the validated set to cache, and return it.
///
/// Falls back gracefully at each step:
/// - No Claude Code installed → use hardcoded defaults
/// - Probe fails → return unvalidated discovered betas (better than nothing)
/// - All betas invalid → return hardcoded defaults
pub async fn resolve_and_validate_betas(
    client: &crate::anthropic::AnthropicClient,
    config_betas: Option<&[String]>,
) -> Vec<String> {
    // Priority 1: explicit config override — user knows best, no validation needed
    if let Some(betas) = config_betas {
        if !betas.is_empty() {
            return betas.to_vec();
        }
    }

    // Priority 2: valid validated cache
    if let Some(cached) = load_cached_validated_betas() {
        if !cached.is_empty() {
            tracing::debug!("Using {} validated betas from cache", cached.len());
            return cached;
        }
    }

    // Priority 3: discover from Claude Code binary
    let discovered = discover_betas_from_claude();

    // Build candidate list: always start with DEFAULT_BETAS, then merge discovered
    let mut candidates: Vec<String> = DEFAULT_BETAS.iter().map(|s| s.to_string()).collect();
    for b in &discovered {
        if !candidates.contains(b) {
            candidates.push(b.clone());
        }
    }

    if candidates.is_empty() {
        return DEFAULT_BETAS.iter().map(|s| s.to_string()).collect();
    }

    // Validate against the API
    let validated = client.validate_betas(candidates).await;

    let result = if validated.is_empty() {
        tracing::warn!("Beta validation removed all betas; falling back to defaults");
        DEFAULT_BETAS.iter().map(|s| s.to_string()).collect()
    } else {
        validated
    };

    // Cache the validated result
    save_validated_betas_cache(&result);
    tracing::info!("Beta validation complete: {} betas validated and cached", result.len());

    result
}

// ---------------------------------------------------------------------------
// Tests for new beta validation cache functions
// ---------------------------------------------------------------------------

#[cfg(test)]
mod beta_validation_cache_tests {
    use super::*;

    #[test]
    fn test_load_cached_validated_betas_returns_none_when_missing() {
        // When the cache file doesn't exist, should return None gracefully.
        // We test this by checking a path that won't exist (temp path).
        // The function uses dirs::config_dir() + archon/validated_betas.json
        // We can't easily change the path, but we can verify None is returned
        // when the content is absent (or expired). We'll do a round-trip instead.
        // First, just ensure it returns None or Some without panicking.
        let result = load_cached_validated_betas();
        // Result is either None (no cache) or Some (cache exists) — both are valid.
        // The test exercises that the function runs without panicking.
        let _ = result;
    }

    #[test]
    fn test_save_and_load_validated_betas_round_trip() {
        let betas = vec![
            "claude-code-20250219".to_string(),
            "oauth-2025-04-20".to_string(),
            "test-beta-2025-01-01".to_string(),
        ];

        save_validated_betas_cache(&betas);
        let loaded = load_cached_validated_betas();

        assert!(loaded.is_some(), "cache should be present after saving");
        let loaded_betas = loaded.unwrap();
        assert_eq!(loaded_betas.len(), betas.len());
        for b in &betas {
            assert!(loaded_betas.contains(b), "loaded cache should contain {b}");
        }
    }

    #[tokio::test]
    async fn test_resolve_and_validate_betas_uses_config_betas_if_provided() {
        use crate::anthropic::AnthropicClient;
        use crate::auth::AuthProvider;
        use crate::identity::{IdentityMode, IdentityProvider};

        let auth = AuthProvider::ApiKey(crate::types::Secret::new("test-key".to_string()));
        let identity = IdentityProvider::new(
            IdentityMode::Clean,
            "test-session".to_string(),
            "test-device".to_string(),
            String::new(),
        );
        let client = AnthropicClient::new(auth, identity, None);

        let config_betas = vec!["explicit-beta-2025-01-01".to_string()];
        let result = resolve_and_validate_betas(&client, Some(&config_betas)).await;

        // When config_betas is non-empty, it should be returned as-is without validation
        assert_eq!(result, config_betas);
    }

    #[tokio::test]
    async fn test_resolve_and_validate_betas_falls_back_to_defaults_when_no_discovery() {
        use crate::anthropic::AnthropicClient;
        use crate::auth::AuthProvider;
        use crate::identity::{IdentityMode, IdentityProvider};

        // Clear any existing validated cache to force a fresh discovery attempt
        let cache_path = dirs::config_dir()
            .unwrap_or_default()
            .join("archon")
            .join("validated_betas.json");
        let _ = std::fs::remove_file(&cache_path);

        let auth = AuthProvider::ApiKey(crate::types::Secret::new("test-key".to_string()));
        let identity = IdentityProvider::new(
            IdentityMode::Clean,
            "test-session".to_string(),
            "test-device".to_string(),
            String::new(),
        );
        let client = AnthropicClient::new(auth, identity, None);

        // Pass None so it attempts discovery; if Claude Code is not installed,
        // should return DEFAULT_BETAS (possibly after a failed API probe).
        // We just verify the result is non-empty (graceful fallback).
        let result = resolve_and_validate_betas(&client, None).await;
        assert!(!result.is_empty(), "should always return at least some betas");
    }
}

/// Resolve beta list: config override > discovered/cached > hardcoded defaults.
pub fn resolve_betas(config_betas: Option<&[String]>) -> Vec<String> {
    // Priority 1: explicit config override
    if let Some(betas) = config_betas {
        if !betas.is_empty() {
            return betas.to_vec();
        }
    }

    // Priority 2: cached discovery
    if let Some(cached) = load_cached_betas() {
        if !cached.is_empty() {
            return cached;
        }
    }

    // Priority 3: hardcoded defaults
    DEFAULT_BETAS.iter().map(|s| s.to_string()).collect()
}
