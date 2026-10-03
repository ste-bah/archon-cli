use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use archon_core::dispatch::ToolRegistry;
use archon_mcp::types::{McpToolRisk, ServerConfig};
use archon_permissions::rules::{RuleSet, ToolRule};
use archon_tools::tool::Tool;

/// How long one server gets to start and list its tools.
const SERVER_STARTUP: Duration = Duration::from_secs(15);

pub(crate) async fn install_project_tools(
    project_root: &Path,
    registry: &mut ToolRegistry,
    rules: &mut RuleSet,
) -> Result<()> {
    let root = archon_mcp::config::nearest_config_root(project_root);
    let configs = archon_mcp::config::load_merged_configs_with_origin(&root)
        .with_context(|| format!("loading workflow MCP configuration from {}", root.display()))?;
    if configs.is_empty() {
        // Say so. This returning silently is how project MCP tools vanished
        // from workflow subagents without a single error: the agents simply
        // had no tradingview tools, and every task requiring one failed for
        // "never exercised" instead of "no config found".
        tracing::warn!(
            searched = %root.display(),
            from = %project_root.display(),
            "no project MCP servers configured; subagents get no MCP tools"
        );
        return Ok(());
    }
    let policies = policy_by_server(configs.iter().map(|(config, _)| config));
    let manager = archon_mcp::lifecycle::McpServerManager::new();
    let tools = match start_servers(&manager, configs, SERVER_STARTUP).await {
        Ok(tools) => tools,
        Err(error) => {
            let _ = manager.shutdown_all().await;
            return Err(error);
        }
    };
    let names = tools
        .iter()
        .map(|tool| tool.name().to_string())
        .collect::<Vec<_>>();
    apply_explicit_policy(rules, &names, &policies);
    for tool in tools {
        registry.register(Box::new(tool));
    }
    tracing::info!(
        count = names.len(),
        "registered project MCP tools for workflow subagents"
    );
    Ok(())
}

/// Starts each server and lists its tools within `deadline`. A server the
/// project's own `.mcp.json` declares (`true` beside it) is required: if it
/// cannot start or cannot list its tools, the call fails; one that lists no
/// tools is healthy and only warned about. A server
/// only the user's global configuration declares is a personal tool, not
/// project authority, and stays a warning as it is in the interactive session.
async fn start_servers(
    manager: &archon_mcp::lifecycle::McpServerManager,
    configs: Vec<(ServerConfig, bool)>,
    deadline: Duration,
) -> Result<Vec<archon_mcp::tool_bridge::McpTool>> {
    let mut tools = Vec::new();
    for (config, required) in configs.into_iter().filter(|(config, _)| !config.disabled) {
        let name = config.name.clone();
        let command = config.command.clone();
        let secrets = configured_values(&config);
        let recovery = if command.is_empty() {
            "check the configured transport, endpoint and credentials and retry".to_string()
        } else {
            format!(
                "make {command} resolvable on the PATH archon is started with and check the server configuration, then retry"
            )
        };
        let outcome = tokio::time::timeout(deadline, async {
            let errors = manager.start_all(vec![config]).await;
            if !errors.is_empty() {
                return Err(format!(
                    "failed to start: {}",
                    errors
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join("; ")
                ));
            }
            match manager.tools_for(&name).await {
                Err(error) => Err(format!("started but listing its tools failed: {error}")),
                Ok(listed) => {
                    // Healthy, just tool-less: a resources-only server exists.
                    if listed.is_empty() {
                        tracing::warn!(server = %name, "MCP server started but offers no tools");
                    }
                    Ok(listed)
                }
            }
        })
        .await
        .unwrap_or_else(|_| {
            Err(format!(
                "did not start and list its tools within {}s",
                deadline.as_secs_f32()
            ))
        });
        let reason = match outcome {
            Ok(listed) => {
                tools.extend(listed);
                continue;
            }
            Err(reason) => reason,
        };
        let failure = redact(
            &format!("workflow MCP server '{name}' (executable '{command}') {reason}; {recovery}"),
            &secrets,
        );
        if required {
            return Err(anyhow!(failure));
        }
        tracing::warn!(
            error = %failure,
            "optional user MCP server unavailable; workflow subagents run without its tools"
        );
    }
    Ok(tools)
}

/// Values the server configuration supplies to the server: environment and
/// header values are where its credentials live, and a start error can echo
/// them. Very short values are left alone, since masking them would only
/// garble the message.
fn configured_values(config: &ServerConfig) -> Vec<String> {
    let mut values = config
        .env
        .values()
        .chain(config.headers.iter().flat_map(|headers| headers.values()))
        .filter(|value| value.len() >= 4)
        .cloned()
        .collect::<Vec<_>>();
    values.sort_by_key(|value| std::cmp::Reverse(value.len()));
    values
}

fn redact(text: &str, secrets: &[String]) -> String {
    secrets.iter().fold(text.to_string(), |text, secret| {
        text.replace(secret.as_str(), "[REDACTED]")
    })
}

fn policy_by_server<'a>(
    configs: impl Iterator<Item = &'a ServerConfig>,
) -> BTreeMap<String, archon_mcp::types::McpToolBridgePolicy> {
    configs
        .map(|config| (config.name.clone(), config.tool_policy.clone()))
        .collect()
}

fn apply_explicit_policy(
    rules: &mut RuleSet,
    names: &[String],
    policies: &BTreeMap<String, archon_mcp::types::McpToolBridgePolicy>,
) {
    for name in names {
        let risk = configured_risk(name, policies);
        let target = match risk {
            Some(McpToolRisk::Safe | McpToolRisk::Risky) => &mut rules.always_allow,
            Some(McpToolRisk::Dangerous) | None => &mut rules.always_deny,
        };
        target.push(ToolRule {
            tool: name.clone(),
            pattern: "*".to_string(),
        });
    }
}

fn configured_risk(
    qualified: &str,
    policies: &BTreeMap<String, archon_mcp::types::McpToolBridgePolicy>,
) -> Option<McpToolRisk> {
    let (server, raw) = split_qualified(qualified)?;
    let policy = policies.get(server)?;
    policy
        .tool_permissions
        .get(qualified)
        .or_else(|| policy.tool_permissions.get(raw))
        .copied()
}

fn split_qualified(name: &str) -> Option<(&str, &str)> {
    let suffix = name.strip_prefix("mcp__")?;
    suffix.split_once("__")
}

pub(crate) fn explicitly_permitted_tools(project_root: &Path) -> BTreeSet<String> {
    let configs = archon_mcp::config::load_merged_configs(project_root).unwrap_or_default();
    let mut tools = BTreeSet::new();
    for config in configs {
        for (name, risk) in &config.tool_policy.tool_permissions {
            if *risk == McpToolRisk::Dangerous {
                continue;
            }
            let qualified = if name.starts_with("mcp__") {
                name.clone()
            } else {
                archon_mcp::tool_bridge::qualified_tool_name(&config.name, name)
            };
            tools.insert(qualified);
        }
    }
    tools
}

#[cfg(test)]
#[path = "workflow_mcp_tests.rs"]
mod tests;
