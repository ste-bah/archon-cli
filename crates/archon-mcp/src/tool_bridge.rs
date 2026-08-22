//! Bridge between MCP tool definitions and the Archon `Tool` trait.
//!
//! `McpTool` wraps a remote MCP tool so the LLM can discover and invoke it
//! through the same interface as built-in tools. All MCP tools are prefixed
//! `mcp__{server_name}__{tool_name}`. Unknown tools default to
//! [`PermissionLevel::Risky`]; explicit config can override, and server
//! annotations / metadata are honored only when the server is configured as
//! trusted.

use std::sync::Arc;
use std::time::Duration;

use archon_tools::tool::{PermissionLevel, Tool, ToolCapability, ToolContext, ToolResult};

use crate::client::McpClient;
use crate::types::{McpToolBridgePolicy, McpToolDef, McpToolRisk, ToolContent};

const MCP_MODEL_DESCRIPTION_MAX_BYTES: usize = 1_024;
const MCP_DESCRIPTION_TRUNCATION_MARKER: &str = "... [truncated]";

/// Whole-call budget reported to the dispatcher through [`Tool::timeout`].
///
/// `McpClient::call_tool` already bounds the wait for a response at 120s. This
/// sits just above it so the specific "timed out after 120s" error still wins
/// on an ordinary slow server, and only the failures that timer cannot see —
/// a transport whose send never completes, an event loop that has stopped
/// draining — reach the dispatcher's generic message. Every MCP tool on every
/// server shares this bound; per-server budgets would need config plumbing
/// that #200 Phase 1 does not introduce.
const CALL_BUDGET: Duration = Duration::from_secs(130);

/// Build the fully qualified tool name for an MCP tool.
pub fn qualified_tool_name(server_name: &str, tool_name: &str) -> String {
    format!("mcp__{server_name}__{tool_name}")
}

/// An MCP-backed tool that proxies execution to a remote MCP server.
pub struct McpTool {
    /// Fully qualified name: `mcp__{server}__{tool}`.
    qualified_name: String,
    /// The original tool definition from the server.
    tool_def: McpToolDef,
    /// Shared client used for RPC calls.
    client: Arc<McpClient>,
    /// Description exposed to the model-facing tool schema.
    model_description: String,
    /// Permission level computed from config and trusted metadata.
    permission_level: PermissionLevel,
}

impl McpTool {
    /// Create a new bridge tool.
    ///
    /// The tool name is constructed as `mcp__{server_name}__{tool_name}`.
    pub fn new(server_name: &str, tool_def: McpToolDef, client: Arc<McpClient>) -> Self {
        Self::with_policy(
            server_name,
            tool_def,
            client,
            McpToolBridgePolicy::default(),
        )
    }

    /// Create a new bridge tool with an explicit per-server permission policy.
    pub fn with_policy(
        server_name: &str,
        tool_def: McpToolDef,
        client: Arc<McpClient>,
        policy: McpToolBridgePolicy,
    ) -> Self {
        let qualified_name = qualified_tool_name(server_name, &tool_def.name);
        let permission_level = classify_mcp_tool_permission(server_name, &tool_def, &policy);
        let model_description = bounded_model_description(tool_def.description.as_deref());
        Self {
            qualified_name,
            tool_def,
            client,
            model_description,
            permission_level,
        }
    }
}

/// Classify an MCP tool into Archon's tool-permission levels.
///
/// Precedence:
/// 1. Explicit operator config by raw or qualified tool name.
/// 2. Trusted server `_meta.archon.permissionLevel` / `_meta.permissionLevel`.
/// 3. Trusted MCP annotations and conservative capability-name hints.
/// 4. Risky by default.
pub fn classify_mcp_tool_permission(
    server_name: &str,
    tool_def: &McpToolDef,
    policy: &McpToolBridgePolicy,
) -> PermissionLevel {
    let qualified_name = qualified_tool_name(server_name, &tool_def.name);
    if let Some(level) = policy
        .tool_permissions
        .get(&qualified_name)
        .or_else(|| policy.tool_permissions.get(&tool_def.name))
    {
        return level.as_permission_level();
    }

    if !policy.trust_server_hints {
        return PermissionLevel::Risky;
    }

    if let Some(level) = tool_def.meta.as_ref().and_then(meta_permission_level) {
        return level.as_permission_level();
    }

    if let Some(annotations) = &tool_def.annotations {
        if annotations.read_only_hint == Some(true) && annotations.destructive_hint != Some(true) {
            return PermissionLevel::Safe;
        }
        if annotations.destructive_hint == Some(true) {
            return PermissionLevel::Risky;
        }
    }

    classify_trusted_name_hint(&tool_def.name, tool_def.description.as_deref())
        .unwrap_or(PermissionLevel::Risky)
}

fn meta_permission_level(meta: &serde_json::Value) -> Option<McpToolRisk> {
    let raw = meta
        .pointer("/archon/permissionLevel")
        .or_else(|| meta.pointer("/archon/permission_level"))
        .or_else(|| meta.pointer("/permissionLevel"))
        .or_else(|| meta.pointer("/permission_level"))
        .or_else(|| meta.pointer("/risk"))?
        .as_str()?;
    parse_tool_risk(raw)
}

fn parse_tool_risk(raw: &str) -> Option<McpToolRisk> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "safe" | "read" | "read_only" | "readonly" => Some(McpToolRisk::Safe),
        "risky" | "write" | "medium" => Some(McpToolRisk::Risky),
        "dangerous" | "danger" | "destructive" | "high" => Some(McpToolRisk::Dangerous),
        _ => None,
    }
}

fn classify_trusted_name_hint(name: &str, description: Option<&str>) -> Option<PermissionLevel> {
    let joined = format!(
        "{} {}",
        name.to_ascii_lowercase(),
        description.unwrap_or("").to_ascii_lowercase()
    );
    const DANGEROUS: &[&str] = &[
        "delete", "remove", "destroy", "drop", "truncate", "erase", "purge", "kill",
    ];
    const MUTATING: &[&str] = &[
        "write", "edit", "update", "create", "insert", "upsert", "patch", "apply", "execute",
        "run", "send", "publish", "deploy", "commit", "push", "merge", "move", "rename",
    ];
    const READ_ONLY: &[&str] = &[
        "read", "list", "search", "find", "get", "fetch", "inspect", "status", "describe", "query",
        "lookup", "show",
    ];

    if DANGEROUS.iter().any(|word| joined.contains(word)) {
        return Some(PermissionLevel::Dangerous);
    }
    if MUTATING.iter().any(|word| joined.contains(word)) {
        return Some(PermissionLevel::Risky);
    }
    if READ_ONLY.iter().any(|word| joined.contains(word)) {
        return Some(PermissionLevel::Safe);
    }
    None
}

fn bounded_model_description(description: Option<&str>) -> String {
    let description = description.unwrap_or("MCP tool (no description)");
    if description.len() <= MCP_MODEL_DESCRIPTION_MAX_BYTES {
        return description.to_string();
    }

    let content_budget = MCP_MODEL_DESCRIPTION_MAX_BYTES - MCP_DESCRIPTION_TRUNCATION_MARKER.len();
    let mut end = content_budget;
    while !description.is_char_boundary(end) {
        end -= 1;
    }
    format!(
        "{}{}",
        &description[..end],
        MCP_DESCRIPTION_TRUNCATION_MARKER
    )
}

/// Flatten MCP tool content into a single text string.
fn flatten_content(content: &[ToolContent]) -> String {
    content
        .iter()
        .filter_map(|c| match c {
            ToolContent::Text { text } => Some(text.as_str()),
            ToolContent::Resource { text, .. } => text.as_deref(),
            ToolContent::Image { .. } => Some("[image content]"),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[async_trait::async_trait]
impl Tool for McpTool {
    fn name(&self) -> &str {
        &self.qualified_name
    }

    fn capability(&self) -> ToolCapability {
        ToolCapability::Egress
    }

    fn description(&self) -> &str {
        &self.model_description
    }

    fn input_schema(&self) -> serde_json::Value {
        self.tool_def.input_schema.clone()
    }

    async fn execute(&self, input: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        // Convert Value to a Map for call_tool.
        let arguments = match input {
            serde_json::Value::Object(map) => Some(map),
            serde_json::Value::Null => None,
            other => {
                return ToolResult::error(format!(
                    "MCP tool '{}' expects an object input, got: {}",
                    self.qualified_name, other
                ));
            }
        };

        match self.client.call_tool(&self.tool_def.name, arguments).await {
            Ok(mcp_result) => {
                let text = flatten_content(&mcp_result.content);
                if mcp_result.is_error {
                    ToolResult::error(text)
                } else {
                    ToolResult::success(text)
                }
            }
            Err(e) => {
                ToolResult::error(format!("MCP call to '{}' failed: {e}", self.qualified_name))
            }
        }
    }

    fn permission_level(&self, _input: &serde_json::Value) -> PermissionLevel {
        self.permission_level
    }

    /// Safe to cancel only because [`crate::client::McpClient::call_tool`] was
    /// made cancel-aware for this. The single await sends `tools/call` and then
    /// waits on the response channel; dropping that wait fires the guard in
    /// `call_cancellation`, which sends `notifications/cancelled` for the
    /// request id so the server stops working on an answer nobody will read.
    /// Without that guard this budget would leak work on every server it timed
    /// out against — see `tests/mcp_call_cancellation_tests.rs`.
    fn timeout(&self) -> Option<Duration> {
        Some(CALL_BUDGET)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::McpToolResult;

    fn tool_def(name: &str) -> McpToolDef {
        McpToolDef {
            name: name.into(),
            description: None,
            input_schema: serde_json::json!({"type": "object"}),
            annotations: None,
            meta: None,
            server_name: "server".into(),
        }
    }

    #[test]
    fn qualified_name_construction() {
        assert_eq!(
            qualified_tool_name("memorygraph", "store_memory"),
            "mcp__memorygraph__store_memory"
        );
        assert_eq!(
            qualified_tool_name("serena", "find_symbol"),
            "mcp__serena__find_symbol"
        );
    }

    #[test]
    fn qualified_name_with_special_chars() {
        assert_eq!(
            qualified_tool_name("my-server", "do_thing"),
            "mcp__my-server__do_thing"
        );
    }

    #[test]
    fn flatten_content_text_only() {
        let content = vec![
            ToolContent::Text {
                text: "line 1".into(),
            },
            ToolContent::Text {
                text: "line 2".into(),
            },
        ];
        assert_eq!(flatten_content(&content), "line 1\nline 2");
    }

    #[test]
    fn flatten_content_mixed() {
        let content = vec![
            ToolContent::Text {
                text: "hello".into(),
            },
            ToolContent::Image {
                data: "base64data".into(),
                mime_type: "image/png".into(),
            },
            ToolContent::Resource {
                uri: "file:///test".into(),
                text: Some("resource text".into()),
            },
        ];
        assert_eq!(
            flatten_content(&content),
            "hello\n[image content]\nresource text"
        );
    }

    #[test]
    fn flatten_content_empty() {
        assert_eq!(flatten_content(&[]), "");
    }

    #[test]
    fn flatten_content_resource_no_text() {
        let content = vec![ToolContent::Resource {
            uri: "file:///bin".into(),
            text: None,
        }];
        // Resource with no text is filtered out.
        assert_eq!(flatten_content(&content), "");
    }

    #[test]
    fn flatten_error_result() {
        let result = McpToolResult {
            content: vec![ToolContent::Text {
                text: "something broke".into(),
            }],
            is_error: true,
        };
        let text = flatten_content(&result.content);
        assert_eq!(text, "something broke");
        assert!(result.is_error);
    }

    #[test]
    fn model_description_caps_long_ascii_text() {
        let description = "x".repeat(MCP_MODEL_DESCRIPTION_MAX_BYTES + 1);
        let bounded = bounded_model_description(Some(&description));

        assert!(bounded.len() <= MCP_MODEL_DESCRIPTION_MAX_BYTES);
        assert!(bounded.ends_with(MCP_DESCRIPTION_TRUNCATION_MARKER));
    }

    #[test]
    fn model_description_keeps_exact_cap() {
        let description = "x".repeat(MCP_MODEL_DESCRIPTION_MAX_BYTES);
        assert_eq!(bounded_model_description(Some(&description)), description);
    }

    #[test]
    fn model_description_cap_preserves_utf8() {
        for scalar in ['é', '界', '🦀'] {
            let description = scalar.to_string().repeat(MCP_MODEL_DESCRIPTION_MAX_BYTES);
            let bounded = bounded_model_description(Some(&description));

            assert!(bounded.len() <= MCP_MODEL_DESCRIPTION_MAX_BYTES);
            assert!(bounded.is_char_boundary(bounded.len()));
            assert!(bounded.ends_with(MCP_DESCRIPTION_TRUNCATION_MARKER));
        }
    }

    #[test]
    fn model_description_keeps_short_text_and_default() {
        assert_eq!(bounded_model_description(Some("read data")), "read data");
        assert_eq!(bounded_model_description(None), "MCP tool (no description)");
    }

    #[test]
    fn permission_classification_uses_full_description_beyond_model_cap() {
        let mut tool = tool_def("opaque");
        tool.description = Some(format!(
            "{} delete records",
            "x".repeat(MCP_MODEL_DESCRIPTION_MAX_BYTES)
        ));
        let policy = McpToolBridgePolicy {
            trust_server_hints: true,
            tool_permissions: Default::default(),
        };

        assert_eq!(
            classify_mcp_tool_permission("server", &tool, &policy),
            PermissionLevel::Dangerous
        );
    }

    #[test]
    fn unknown_tool_defaults_risky() {
        let tool = tool_def("mystery");
        let policy = McpToolBridgePolicy::default();
        assert_eq!(
            classify_mcp_tool_permission("server", &tool, &policy),
            PermissionLevel::Risky
        );
    }

    #[test]
    fn untrusted_read_only_annotation_stays_risky() {
        let mut tool = tool_def("read_file");
        tool.annotations = Some(rmcp::model::ToolAnnotations::new().read_only(true));
        assert_eq!(
            classify_mcp_tool_permission("server", &tool, &McpToolBridgePolicy::default()),
            PermissionLevel::Risky
        );
    }

    #[test]
    fn trusted_read_only_annotation_can_be_safe() {
        let mut tool = tool_def("read_file");
        tool.annotations = Some(rmcp::model::ToolAnnotations::new().read_only(true));
        let policy = McpToolBridgePolicy {
            trust_server_hints: true,
            tool_permissions: Default::default(),
        };
        assert_eq!(
            classify_mcp_tool_permission("server", &tool, &policy),
            PermissionLevel::Safe
        );
    }

    #[test]
    fn explicit_policy_overrides_metadata_and_names() {
        let mut tool = tool_def("read_file");
        tool.meta = Some(serde_json::json!({"archon": {"permissionLevel": "safe"}}));
        let mut policy = McpToolBridgePolicy {
            trust_server_hints: true,
            tool_permissions: Default::default(),
        };
        policy
            .tool_permissions
            .insert("read_file".into(), McpToolRisk::Dangerous);
        assert_eq!(
            classify_mcp_tool_permission("server", &tool, &policy),
            PermissionLevel::Dangerous
        );
    }

    #[test]
    fn trusted_meta_permission_hint_is_honored() {
        let mut tool = tool_def("opaque");
        tool.meta = Some(serde_json::json!({"archon": {"permissionLevel": "safe"}}));
        let policy = McpToolBridgePolicy {
            trust_server_hints: true,
            tool_permissions: Default::default(),
        };
        assert_eq!(
            classify_mcp_tool_permission("server", &tool, &policy),
            PermissionLevel::Safe
        );
    }

    // NOTE: McpTool::new() and the Tool trait methods (name, description,
    // input_schema, permission_level, execute) require an Arc<McpClient>,
    // which needs a live MCP server connection. Those are covered by
    // integration tests. The unit tests above verify the helper functions
    // that McpTool delegates to.
}
