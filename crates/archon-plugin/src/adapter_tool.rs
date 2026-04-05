//! PluginToolAdapter — wraps a WASM plugin tool registration into Box<dyn Tool>.

use std::sync::{Arc, Mutex};

use archon_tools::tool::{PermissionLevel, Tool, ToolContext, ToolResult};

use crate::host::WasmPluginHost;

// ── PluginToolAdapter ─────────────────────────────────────────────────────────

/// Adapts a WASM plugin's tool registration to the `Tool` trait.
///
/// The namespaced name is `plugin_id:tool_name` (colon separator).
/// When a live WASM host is attached (via `new_with_host`), `execute()` dispatches
/// the call into the WASM guest via `spawn_blocking`. Without a host, it returns
/// an error result without panicking.
pub struct PluginToolAdapter {
    namespaced_name: String,
    raw_tool_name: String,
    description: String,
    schema: serde_json::Value,
    /// Live WASM host for dispatch. `None` → error on execute.
    host: Option<Arc<Mutex<WasmPluginHost>>>,
}

impl PluginToolAdapter {
    /// Create an adapter without a live WASM host.
    ///
    /// `execute()` returns an error result. Use this only when a host is not available
    /// (e.g., static registration without WASM bytes).
    ///
    /// `schema_json` must be valid JSON; falls back to `{}` on parse error.
    pub fn new(
        plugin_id: String,
        tool_name: String,
        description: String,
        schema_json: String,
    ) -> Self {
        let schema = serde_json::from_str(&schema_json).unwrap_or_else(|_| serde_json::json!({}));
        Self {
            namespaced_name: format!("{plugin_id}:{tool_name}"),
            raw_tool_name: tool_name,
            description,
            schema,
            host: None,
        }
    }

    /// Create an adapter backed by a live WASM host.
    ///
    /// `execute()` dispatches the call into the WASM guest via `spawn_blocking`.
    pub fn new_with_host(
        plugin_id: String,
        tool_name: String,
        description: String,
        schema_json: String,
        host: Arc<Mutex<WasmPluginHost>>,
    ) -> Self {
        let schema = serde_json::from_str(&schema_json).unwrap_or_else(|_| serde_json::json!({}));
        Self {
            namespaced_name: format!("{plugin_id}:{tool_name}"),
            raw_tool_name: tool_name,
            description,
            schema,
            host: Some(host),
        }
    }
}

#[async_trait::async_trait]
impl Tool for PluginToolAdapter {
    fn name(&self) -> &str {
        &self.namespaced_name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn input_schema(&self) -> serde_json::Value {
        self.schema.clone()
    }

    /// Execute the tool by dispatching into the live WASM instance.
    ///
    /// Uses `tokio::task::spawn_blocking` so the sync `Mutex` lock does not
    /// block the async runtime thread.
    ///
    /// Returns an error result when no host is attached or on any WASM error.
    async fn execute(&self, input: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        let Some(host) = &self.host else {
            return ToolResult::error(format!(
                "plugin tool '{}' has no live WASM instance attached",
                self.namespaced_name
            ));
        };

        let host = Arc::clone(host);
        let tool_name = self.raw_tool_name.clone();
        let args_json = input.to_string();

        let result = tokio::task::spawn_blocking(move || {
            match host.lock() {
                Ok(mut guard) => guard.dispatch_tool(&tool_name, &args_json),
                Err(e) => {
                    serde_json::json!({"error": format!("WASM host lock poisoned: {e}")})
                        .to_string()
                }
            }
        })
        .await
        .unwrap_or_else(|e| {
            serde_json::json!({"error": format!("spawn_blocking panicked: {e}")}).to_string()
        });

        ToolResult::success(result)
    }

    /// Plugin tools are classified as `Risky` since they execute arbitrary WASM code.
    fn permission_level(&self, _input: &serde_json::Value) -> PermissionLevel {
        PermissionLevel::Risky
    }
}
