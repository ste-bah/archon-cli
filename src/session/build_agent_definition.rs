use crate::cli_args::Cli;
use archon_core::agent::AgentConfig;
use archon_core::agents::AgentRegistry;
use archon_core::agents::permissions_overlay::{
    PermissionOverlayReason, resolve_permission_overlay,
};

use super::agent_catalog;

pub(in crate::session) fn register_agent_listing(
    registry: &mut archon_core::dispatch::ToolRegistry,
    agent_registry: &AgentRegistry,
) {
    let agents: Vec<(String, String)> = agent_registry
        .list()
        .iter()
        .map(|agent| (agent.agent_type.clone(), agent.description.clone()))
        .collect();
    let common_agents = agent_catalog::common_inline_agents(&agents);
    registry.register(Box::new(
        archon_tools::agent_tool::AgentTool::with_agent_listing(&common_agents),
    ));
    registry.register(Box::new(archon_tools::agent_tool::AgentCatalogTool::new(
        agents,
    )));
}

pub(in crate::session) async fn resolve_agent_definition(
    config: &archon_core::config::ArchonConfig,
    resolved_flags: &archon_core::cli_flags::ResolvedFlags,
    agent_registry: &AgentRegistry,
) -> Result<Option<archon_core::agents::definition::CustomAgentDefinition>, i32> {
    let Some(agent_name) = resolved_flags.agent.as_ref() else {
        return Ok(None);
    };
    match agent_registry.resolve(agent_name) {
        Some(definition) => {
            tracing::info!(agent = agent_name, "resolved custom agent");
            let mut definition = definition.clone();
            if let Err(error) = crate::runtime::agent_profile_overlay::apply_active_profile_overlay_if_enabled_async(
                config,
                &mut definition,
            )
            .await
            {
                tracing::warn!(agent = agent_name, %error, "agent profile overlay skipped");
            }
            Ok(Some(definition))
        }
        None => {
            eprintln!(
                "Unknown agent '{}'. Available: {}",
                agent_name,
                agent_registry.available_agent_names().join(", ")
            );
            Err(1)
        }
    }
}

pub(in crate::session) fn apply_agent_tool_filters(
    registry: &mut archon_core::dispatch::ToolRegistry,
    agent_def: Option<&archon_core::agents::definition::CustomAgentDefinition>,
) {
    if let Some(definition) = agent_def {
        if let Some(allowed) = &definition.allowed_tools {
            let allowed_refs: Vec<&str> = allowed.iter().map(String::as_str).collect();
            registry.filter_whitelist(&allowed_refs);
        }
        if let Some(denied) = &definition.disallowed_tools {
            let denied_refs: Vec<&str> = denied.iter().map(String::as_str).collect();
            registry.filter_blacklist(&denied_refs);
        }
    }
}

pub(in crate::session) fn validate_required_mcp_servers(
    registry: &archon_core::dispatch::ToolRegistry,
    agent_def: Option<&archon_core::agents::definition::CustomAgentDefinition>,
) -> Result<(), i32> {
    if let Some(definition) = agent_def {
        let available_mcp: Vec<String> = registry
            .tool_names()
            .into_iter()
            .filter(|name| name.starts_with("mcp__"))
            .map(str::to_string)
            .collect();
        if !definition.has_required_mcp_servers(&available_mcp) {
            eprintln!(
                "Agent '{}' requires MCP servers {:?} but they are not available.",
                definition.agent_type, definition.required_mcp_servers,
            );
            return Err(1);
        }
    }
    Ok(())
}

pub(in crate::session) async fn apply_agent_execution_overrides(
    agent_config: &mut AgentConfig,
    agent_def: Option<&archon_core::agents::definition::CustomAgentDefinition>,
    cli: &Cli,
) {
    let Some(definition) = agent_def else {
        return;
    };
    if let Some(model) = &definition.model
        && model != "inherit"
    {
        agent_config.model = model.clone();
        *agent_config.model_override.lock().await = model.clone();
    }
    if let Some(effort) = &definition.effort {
        if let Ok(level) = effort.parse::<archon_llm::effort::EffortLevel>() {
            *agent_config.effort_level.lock().await = level;
        } else {
            tracing::warn!(agent = %definition.agent_type, %effort, "invalid effort level in agent definition, using default");
        }
    }
    apply_permission_override(agent_config, definition, cli).await;
    if definition.max_turns.is_some() {
        agent_config.max_turns = definition.max_turns;
    }
}

async fn apply_permission_override(
    agent_config: &mut AgentConfig,
    definition: &archon_core::agents::definition::CustomAgentDefinition,
    cli: &Cli,
) {
    let Some(permission_mode) = &definition.permission_mode else {
        return;
    };
    let parent_mode = agent_config.permission_mode.lock().await.clone();
    let decision = resolve_permission_overlay(
        &parent_mode,
        Some(permission_mode),
        cli.dangerously_skip_permissions,
    );
    match decision.reason {
        PermissionOverlayReason::Applied => {
            *agent_config.permission_mode.lock().await =
                decision.effective_mode.as_str().to_string();
        }
        PermissionOverlayReason::ParentModeLocked => {
            tracing::debug!(agent = %definition.agent_type, parent_mode = %decision.parent_mode, requested_mode = %decision.requested_mode.expect("requested mode exists"), "agent permission_mode skipped because parent mode has priority");
        }
        PermissionOverlayReason::BlockedDangerousBypass => {
            tracing::warn!(agent = %definition.agent_type, raw_mode = %permission_mode, "agent requests bypassPermissions but --dangerously-skip-permissions not passed; ignoring");
        }
        PermissionOverlayReason::BlockedExpansion => {
            tracing::warn!(agent = %definition.agent_type, parent_mode = %decision.parent_mode, requested_mode = %decision.requested_mode.expect("requested mode exists"), "agent permission_mode would widen parent mode; keeping parent mode");
        }
        PermissionOverlayReason::NoRequest => {}
    }
}
