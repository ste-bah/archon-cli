use std::collections::HashMap;

use super::{IdentityMode, compute_fingerprint};

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
                headers.insert("X-Claude-Code-Session-Id".into(), self.session_id.clone());
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
