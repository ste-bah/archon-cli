//! RemoteTrigger tool — HTTP POST to user-configured endpoints.
//!
//! # Divergence from project-zero's RemoteTriggerTool
//!
//! project-zero's `RemoteTriggerTool` is a first-party claude.ai cloud-service
//! integration.  It manages cloud-hosted remote sessions with `list`, `get`,
//! `create`, `update`, and `run` CRUD actions and requires OAuth.
//!
//! Archon's `RemoteTrigger` is intentionally different:
//! - General-purpose HTTP POST to ANY user-controlled endpoint
//! - No OAuth dependency — uses a config-based host allowlist instead
//! - No cloud backend — works with any HTTP server the user controls
//! - No CRUD actions — single POST-and-return operation
//!
//! This is an Archon-native extension, not a compatibility shim.

use std::collections::HashMap;

use serde_json::json;

use crate::tool::{PermissionLevel, Tool, ToolContext, ToolResult};

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Named preset for a remote trigger endpoint.
#[derive(Debug, Clone)]
pub struct TriggerPreset {
    /// Fully-qualified URL for this preset.
    pub url: String,
    /// Optional `Authorization` header value.  May contain `${ENV_VAR}`.
    pub auth: Option<String>,
}

/// Runtime configuration for `RemoteTriggerTool`.
#[derive(Debug, Clone)]
pub struct RemoteTriggerConfig {
    /// Only send requests to these hostnames.  Scheme and port are stripped
    /// during comparison so `"localhost"` matches `http://localhost:8080/`.
    pub allowed_hosts: Vec<String>,
    /// Named presets (name → url + optional auth).
    pub presets: HashMap<String, TriggerPreset>,
    /// Default timeout in seconds.  Must be 20.
    pub default_timeout_secs: u32,
    /// Maximum timeout the caller may request.
    pub max_timeout_secs: u32,
    /// Maximum response body size in bytes (100 KB = 102_400).
    pub max_response_bytes: usize,
}

impl Default for RemoteTriggerConfig {
    fn default() -> Self {
        Self {
            allowed_hosts: Vec::new(),
            presets: HashMap::new(),
            default_timeout_secs: 20,
            max_timeout_secs: 120,
            max_response_bytes: 100 * 1024,
        }
    }
}

// ---------------------------------------------------------------------------
// Tool
// ---------------------------------------------------------------------------

/// HTTP POST tool for triggering remote agent workflows.
///
/// NOT a port of project-zero's `RemoteTriggerTool`.  See module-level docs.
pub struct RemoteTriggerTool {
    config: RemoteTriggerConfig,
}

impl RemoteTriggerTool {
    /// Create a new tool with the given configuration.
    pub fn new(config: RemoteTriggerConfig) -> Self {
        Self { config }
    }

    // ── Internal helpers ────────────────────────────────────────────────────

    /// Extract the hostname from a URL string.  Returns `None` for invalid URLs.
    fn extract_host(url: &str) -> Option<String> {
        // Minimal URL parsing: strip scheme, extract host before first /
        let without_scheme = if let Some(pos) = url.find("://") {
            &url[pos + 3..]
        } else {
            return None;
        };
        // Strip path and query
        let host_and_port = without_scheme.split('/').next().unwrap_or(without_scheme);
        // Strip port
        let host = if let Some(colon) = host_and_port.rfind(':') {
            // Only strip port if it's numeric (avoid stripping IPv6 addresses)
            let maybe_port = &host_and_port[colon + 1..];
            if maybe_port.chars().all(|c| c.is_ascii_digit()) {
                &host_and_port[..colon]
            } else {
                host_and_port
            }
        } else {
            host_and_port
        };
        Some(host.to_lowercase())
    }

    /// Check whether `url` is allowed by the host allowlist.
    fn is_host_allowed(&self, url: &str) -> bool {
        let Some(host) = Self::extract_host(url) else {
            return false;
        };
        self.config
            .allowed_hosts
            .iter()
            .any(|allowed| allowed.to_lowercase() == host)
    }

    /// Resolve an effective timeout: clamp to `[1, max_timeout_secs]`,
    /// using `default_timeout_secs` when the input is 0.
    pub fn resolve_timeout(&self, requested: u32) -> u32 {
        if requested == 0 {
            self.config.default_timeout_secs
        } else {
            requested.min(self.config.max_timeout_secs).max(1)
        }
    }

    /// Truncate `body` to at most `max_bytes` bytes (character boundary aware).
    pub fn truncate_response(body: &str, max_bytes: usize) -> &str {
        if body.len() <= max_bytes {
            return body;
        }
        // Walk back from max_bytes to a valid UTF-8 char boundary
        let mut end = max_bytes;
        while !body.is_char_boundary(end) {
            end -= 1;
        }
        &body[..end]
    }

    /// Expand `${ENV_VAR}` placeholders in a string using current env.
    /// Missing variables are replaced with an empty string.
    pub fn expand_env_vars(s: &str) -> String {
        let mut result = s.to_owned();
        // Find all ${...} patterns
        while let Some(start) = result.find("${") {
            let after_start = start + 2;
            if let Some(end) = result[after_start..].find('}') {
                let var_name = &result[after_start..after_start + end].to_owned();
                let value = std::env::var(var_name).unwrap_or_default();
                let placeholder = format!("${{{var_name}}}");
                result = result.replacen(&placeholder, &value, 1);
            } else {
                break; // malformed placeholder — stop processing
            }
        }
        result
    }

    /// Resolve a named preset.  Returns `(url, Option<auth>)` or `None` if not found.
    pub fn resolve_preset(&self, name: &str) -> Option<(String, Option<String>)> {
        self.config.presets.get(name).map(|p| {
            let auth = p.auth.as_deref().map(Self::expand_env_vars);
            (p.url.clone(), auth)
        })
    }
}

#[async_trait::async_trait]
impl Tool for RemoteTriggerTool {
    fn name(&self) -> &str {
        "RemoteTrigger"
    }

    fn description(&self) -> &str {
        "Send an HTTP POST request to a configured remote endpoint and return the \
         response. Only hosts listed in `remote_triggers.allowed_hosts` are accepted. \
         Archon-specific tool — NOT project-zero's RemoteTriggerTool (no cloud backend, \
         no CRUD actions, no OAuth)."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "Full URL to POST to. Must be on the allowed_hosts list."
                },
                "payload": {
                    "type": "object",
                    "description": "JSON payload sent as the request body."
                },
                "preset": {
                    "type": "string",
                    "description": "Named preset from config.toml [remote_triggers.presets]. Overrides 'url'."
                },
                "timeout_secs": {
                    "type": "integer",
                    "description": "Request timeout in seconds (1-120). Default: 20.",
                    "minimum": 1,
                    "maximum": 120
                }
            },
            "required": []
        })
    }

    async fn execute(&self, input: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        // Resolve URL: preset > explicit url
        let (url, auth_override) =
            if let Some(preset_name) = input.get("preset").and_then(|v| v.as_str()) {
                match self.resolve_preset(preset_name) {
                    Some((u, a)) => (u, a),
                    None => {
                        return ToolResult::error(format!(
                            "RemoteTrigger: unknown preset '{preset_name}'"
                        ));
                    }
                }
            } else {
                let url = match input.get("url").and_then(|v| v.as_str()) {
                    Some(u) if !u.is_empty() => u.to_owned(),
                    _ => return ToolResult::error("RemoteTrigger: 'url' is required"),
                };
                (url, None)
            };

        // Validate URL structure
        if !url.contains("://") {
            return ToolResult::error(format!(
                "RemoteTrigger: invalid URL '{url}' — must include scheme (http:// or https://)"
            ));
        }

        // Allowlist check — done BEFORE any network I/O
        if !self.is_host_allowed(&url) {
            let host = Self::extract_host(&url).unwrap_or_else(|| url.clone());
            return ToolResult::error(format!(
                "RemoteTrigger: host '{host}' is not on the allowlist. \
                 Add it to [remote_triggers] allowed_hosts in config.toml."
            ));
        }

        // Payload
        let payload = input.get("payload").cloned().unwrap_or(json!({}));

        // Timeout
        let timeout_secs = input
            .get("timeout_secs")
            .and_then(|v| v.as_u64())
            .map(|s| s as u32)
            .unwrap_or(0);
        let timeout = self.resolve_timeout(timeout_secs);

        // Build request
        let client = match reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(timeout as u64))
            .build()
        {
            Ok(c) => c,
            Err(e) => {
                return ToolResult::error(format!(
                    "RemoteTrigger: failed to build HTTP client: {e}"
                ));
            }
        };

        let mut req = client.post(&url).json(&payload);

        // Auth header (preset auth overrides nothing; explicit is separate)
        if let Some(auth) = auth_override {
            req = req.header("Authorization", auth);
        }

        // Execute
        let response = match req.send().await {
            Ok(r) => r,
            Err(e) => return ToolResult::error(format!("RemoteTrigger: request failed: {e}")),
        };

        let status = response.status();
        let body = match response.text().await {
            Ok(b) => b,
            Err(e) => {
                return ToolResult::error(format!(
                    "RemoteTrigger: failed to read response body: {e}"
                ));
            }
        };

        let truncated = Self::truncate_response(&body, self.config.max_response_bytes);
        let truncated_note = if body.len() > self.config.max_response_bytes {
            format!(
                " [truncated at {}KB]",
                self.config.max_response_bytes / 1024
            )
        } else {
            String::new()
        };

        let output = format!(
            "HTTP {} {}\n{}{}",
            status.as_u16(),
            status.canonical_reason().unwrap_or(""),
            truncated,
            truncated_note
        );

        if status.is_success() {
            ToolResult::success(output)
        } else {
            ToolResult::error(output)
        }
    }

    fn permission_level(&self, _input: &serde_json::Value) -> PermissionLevel {
        PermissionLevel::Risky
    }
}
