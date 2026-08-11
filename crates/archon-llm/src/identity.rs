use std::fs;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

mod fingerprint;
mod mode;
mod provider;

pub use fingerprint::compute_fingerprint;
pub use mode::{
    CONDITIONAL_BETAS, CustomIdentityConfigView, DEFAULT_BETAS, IdentityConfigView, IdentityMode,
    resolve_identity_mode,
};
pub use provider::IdentityProvider;

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

    tracing::debug!(
        "Auto-discovered {} beta headers from Claude Code",
        betas.len()
    );
    betas
}

/// Find the Claude Code binary in PATH or common locations.
fn find_claude_binary() -> Option<PathBuf> {
    // Check PATH first
    if let Ok(path) = which::which("claude") {
        return Some(path);
    }

    // Common locations
    let candidates = ["/usr/local/bin/claude", "/usr/bin/claude"];

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

/// Read the version field from the npm package.json that ships with the
/// installed Claude Code binary.
///
/// Resolution: takes the path returned by `find_claude_binary()`, follows
/// symlinks to the real file, walks up two levels (bin/claude.exe →
/// ../package.json), parses the JSON, returns `version` as String.
///
/// Returns None if:
/// - find_claude_binary() returns None (not installed)
/// - canonicalize fails
/// - package.json doesn't exist or isn't readable
/// - JSON is malformed or has no string `version` field
pub fn version_from_package_json() -> Option<String> {
    let claude_path = find_claude_binary()?;
    version_from_package_json_at(&claude_path)
}

/// Same as `version_from_package_json` but takes an explicit binary path
/// (testable in isolation without relying on PATH state).
pub fn version_from_package_json_at(claude_path: &std::path::Path) -> Option<String> {
    let real = std::fs::canonicalize(claude_path).ok()?;
    // bin/claude.exe → ../package.json
    let pkg_json = real.parent()?.parent()?.join("package.json");
    let content = std::fs::read_to_string(&pkg_json).ok()?;
    let v: serde_json::Value = serde_json::from_str(&content).ok()?;
    v.get("version").and_then(|x| x.as_str()).map(String::from)
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
        if (0x20..0x7f).contains(&byte) {
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

const BETA_CACHE_VERSION: u32 = 1;
const BETA_CACHE_MAX_AGE_SECONDS: i64 = 86_400;

#[derive(serde::Deserialize)]
struct BetaCache {
    version: u32,
    betas: Vec<String>,
    timestamp: i64,
    integrity: String,
}

const DISCOVERED_BETAS_FILE: &str = "discovered_betas.json";
const VALIDATED_BETAS_FILE: &str = "validated_betas.json";

/// Real on-disk home of the beta caches: `<user config dir>/archon`.
///
/// This is the only place the user-level directory is named. Every read, write
/// and delete below is expressed against a root the caller supplies, so a test
/// can point at its own `TempDir` and share nothing with any other test. That
/// is deliberate: these caches used to be reachable only through their real
/// path, which meant the suite both raced against itself and overwrote the
/// developer's actual `validated_betas.json` on every run.
fn beta_cache_root() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from(".config"))
        .join("archon")
}

/// Load cached betas from disk, or None if cache is stale/missing.
pub fn load_cached_betas() -> Option<Vec<String>> {
    load_beta_cache_file(&beta_cache_root().join(DISCOVERED_BETAS_FILE))
}

/// Save discovered betas to cache.
pub fn save_betas_cache(betas: &[String]) {
    let _ = save_beta_cache_file(&beta_cache_root().join(DISCOVERED_BETAS_FILE), betas);
}

// ---------------------------------------------------------------------------
// Validated beta cache (separate from raw discovered betas)
// ---------------------------------------------------------------------------

/// Load the previously validated+cached beta list, or None if missing/stale.
pub fn load_cached_validated_betas() -> Option<Vec<String>> {
    load_validated_betas_in(&beta_cache_root())
}

/// Save the validated beta list to cache.
pub fn save_validated_betas_cache(betas: &[String]) {
    save_validated_betas_in(&beta_cache_root(), betas);
}

/// [`load_cached_validated_betas`], against an explicit cache root.
pub(crate) fn load_validated_betas_in(root: &Path) -> Option<Vec<String>> {
    load_beta_cache_file(&root.join(VALIDATED_BETAS_FILE))
}

/// [`save_validated_betas_cache`], against an explicit cache root.
pub(crate) fn save_validated_betas_in(root: &Path, betas: &[String]) {
    let _ = save_beta_cache_file(&root.join(VALIDATED_BETAS_FILE), betas);
}

fn save_beta_cache_file(path: &Path, betas: &[String]) -> std::io::Result<()> {
    let timestamp = chrono::Utc::now().timestamp();
    let integrity = beta_cache_integrity(BETA_CACHE_VERSION, timestamp, betas);
    let cache = serde_json::json!({
        "version": BETA_CACHE_VERSION,
        "betas": betas,
        "timestamp": timestamp,
        "integrity": integrity,
    });
    let content = serde_json::to_string_pretty(&cache).unwrap_or_default();
    write_private_json_file(path, &content)
}

fn load_beta_cache_file(path: &Path) -> Option<Vec<String>> {
    let content = fs::read_to_string(path).ok()?;
    let cache: BetaCache = serde_json::from_str(&content).ok()?;
    if cache.version != BETA_CACHE_VERSION {
        return None;
    }
    if cache.integrity != beta_cache_integrity(cache.version, cache.timestamp, &cache.betas) {
        return None;
    }
    let age = chrono::Utc::now().timestamp() - cache.timestamp;
    if age > BETA_CACHE_MAX_AGE_SECONDS {
        return None;
    }
    Some(cache.betas)
}

fn beta_cache_integrity(version: u32, timestamp: i64, betas: &[String]) -> String {
    let payload = serde_json::json!([version, timestamp, betas]);
    let bytes = serde_json::to_vec(&payload).unwrap_or_default();
    format!("sha256:{}", hex::encode(Sha256::digest(&bytes)))
}

fn write_private_json_file(path: &Path, content: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

        let mut file = fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .mode(0o600)
            .open(path)?;
        file.write_all(content.as_bytes())?;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
        Ok(())
    }
    #[cfg(not(unix))]
    {
        fs::write(path, content)
    }
}

/// Clear cached Claude/Anthropic beta-header discovery files.
///
/// This backs the `/refresh-identity` skill. The next Anthropic request will
/// re-discover and re-validate the accepted beta headers.
pub fn clear_beta_caches() -> std::io::Result<Vec<PathBuf>> {
    let root = beta_cache_root();
    let mut removed = Vec::new();
    for path in [
        root.join(DISCOVERED_BETAS_FILE),
        root.join(VALIDATED_BETAS_FILE),
    ] {
        match fs::remove_file(&path) {
            Ok(()) => removed.push(path),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => return Err(err),
        }
    }
    Ok(removed)
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
    resolve_and_validate_betas_in(&beta_cache_root(), client, config_betas).await
}

/// [`resolve_and_validate_betas`], against an explicit cache root.
pub(crate) async fn resolve_and_validate_betas_in(
    cache_root: &Path,
    client: &crate::anthropic::AnthropicClient,
    config_betas: Option<&[String]>,
) -> Vec<String> {
    // Priority 1: explicit config override — user knows best, no validation needed
    if let Some(betas) = config_betas
        && !betas.is_empty()
    {
        return betas.to_vec();
    }

    // Priority 2: valid validated cache
    if let Some(cached) = load_validated_betas_in(cache_root)
        && !cached.is_empty()
    {
        tracing::debug!("Using {} validated betas from cache", cached.len());
        return cached;
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
    save_validated_betas_in(cache_root, &result);
    tracing::info!(
        "Beta validation complete: {} betas validated and cached",
        result.len()
    );

    result
}

/// Resolve beta list: config override > discovered/cached > hardcoded defaults.
pub fn resolve_betas(config_betas: Option<&[String]>) -> Vec<String> {
    // Priority 1: explicit config override
    if let Some(betas) = config_betas
        && !betas.is_empty()
    {
        return betas.to_vec();
    }

    // Priority 2: cached discovery
    if let Some(cached) = load_cached_betas()
        && !cached.is_empty()
    {
        return cached;
    }

    // Priority 3: hardcoded defaults
    DEFAULT_BETAS.iter().map(|s| s.to_string()).collect()
}

#[cfg(test)]
mod tests;
