use std::path::Path;

use super::super::context::HookContext;
use super::super::function::FunctionRegistry;
use super::super::types::{HookConfig, HookEvent, HookResult};

static FUNCTION_REGISTRY: std::sync::LazyLock<FunctionRegistry> =
    std::sync::LazyLock::new(FunctionRegistry::new);

pub(super) fn execute_function_hook(
    config: &HookConfig,
    input: &serde_json::Value,
    cwd: &Path,
    session_id: &str,
    event_name: &str,
) -> HookResult {
    let hook_event: HookEvent =
        serde_json::from_value(serde_json::Value::String(event_name.to_string()))
            .unwrap_or(HookEvent::PreToolUse);

    let tool_name = input
        .get("tool_name")
        .and_then(|value| value.as_str())
        .map(str::to_string);

    let mut builder = HookContext::builder(hook_event)
        .session_id(session_id.to_string())
        .cwd(cwd.to_string_lossy().to_string());

    if let Some(name) = tool_name {
        builder = builder.tool_name(name);
    }
    if let Some(tool_input) = input.get("tool_input") {
        builder = builder.tool_input(tool_input.clone());
    }

    FUNCTION_REGISTRY.execute(&config.command, &builder.build())
}
