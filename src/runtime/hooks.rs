use std::path::Path;
use std::sync::Arc;

use archon_core::hooks::{AggregatedHookResult, HookEvent, HookRegistry};

pub(crate) fn load_runtime_hook_registry(working_dir: &Path) -> Arc<HookRegistry> {
    let home_dir = dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
    Arc::new(HookRegistry::load_all(working_dir, &home_dir))
}

pub(crate) fn register_agent_session_hooks(
    registry: &Arc<HookRegistry>,
    session_id: &str,
    agent_def: Option<&archon_core::agents::definition::CustomAgentDefinition>,
) {
    let Some(def) = agent_def else {
        return;
    };
    let Some(hooks_json) = def.hooks.as_ref() else {
        return;
    };
    match archon_core::agents::loader::parse_agent_hooks(hooks_json) {
        Ok(hook_pairs) => {
            for (event, config) in hook_pairs {
                registry.register_session_hook(session_id, event, config);
            }
            tracing::info!(agent = %def.agent_type, "registered agent session-scoped hooks");
        }
        Err(error) => {
            tracing::warn!(
                agent = %def.agent_type,
                %error,
                "failed to parse agent hooks"
            );
        }
    }
}

pub(crate) async fn fire_runtime_hook(
    registry: &Arc<HookRegistry>,
    event: HookEvent,
    working_dir: &Path,
    session_id: &str,
    payload: serde_json::Value,
) -> AggregatedHookResult {
    let result = registry
        .execute_hooks(event.clone(), payload, working_dir, session_id)
        .await;
    trace_ignored_runtime_hook_output(&event, &result);
    result
}

pub(crate) async fn fire_provider_resolve_hook(
    registry: &Arc<HookRegistry>,
    working_dir: &Path,
    session_id: &str,
    payload: ProviderResolveHookPayload<'_>,
) {
    let event = if payload.stage == "before_provider_resolve" {
        HookEvent::BeforeProviderResolve
    } else {
        HookEvent::AfterProviderResolve
    };
    fire_runtime_hook(
        registry,
        event,
        working_dir,
        session_id,
        serde_json::json!({
            "hook_event": payload.hook_event,
            "stage": payload.stage,
            "surface": payload.surface,
            "requested_provider": payload.requested_provider,
            "selected_provider": payload.selected_provider,
            "runtime_mode": payload.runtime_mode,
            "profile_id": payload.profile_id,
        }),
    )
    .await;
}

pub(crate) struct ProviderResolveHookPayload<'a> {
    pub(crate) hook_event: &'a str,
    pub(crate) stage: &'a str,
    pub(crate) surface: &'a str,
    pub(crate) requested_provider: &'a str,
    pub(crate) selected_provider: Option<&'a str>,
    pub(crate) runtime_mode: Option<&'a str>,
    pub(crate) profile_id: Option<&'a str>,
}

fn trace_ignored_runtime_hook_output(event: &HookEvent, result: &AggregatedHookResult) {
    if result.is_blocked()
        || result.updated_input.is_some()
        || result.updated_mcp_tool_output.is_some()
        || !result.updated_permissions.is_empty()
        || result.prevent_continuation
        || result.retry
    {
        tracing::warn!(
            hook_event = %event,
            "runtime lifecycle hook returned behaviour-changing output; ignored"
        );
    }
}
