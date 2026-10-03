use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use archon_core::dispatch::ToolRegistry;
use archon_mcp::types::{McpToolRisk, ServerConfig};
use archon_permissions::rules::{RuleSet, ToolRule};
use archon_tools::tool::Tool;

pub(crate) async fn install_project_tools(
    project_root: &Path,
    registry: &mut ToolRegistry,
    rules: &mut RuleSet,
) -> Result<()> {
    let root = archon_mcp::config::nearest_config_root(project_root);
    let configs = archon_mcp::config::load_merged_configs(&root)
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
    // Only a server the project itself declares is required. The merged list
    // also holds the user's own global servers; one of those that is broken on
    // this machine is a personal tool, not project authority, and stays a
    // warning as it is in the interactive session.
    let project_config = root.join(".mcp.json");
    let required = archon_mcp::config::load_config_file(&project_config)
        .with_context(|| format!("loading {}", project_config.display()))?
        .into_iter()
        .map(|config| config.name)
        .collect::<BTreeSet<_>>();
    let policies = policy_by_server(&configs);
    let manager = archon_mcp::lifecycle::McpServerManager::new();
    if let Err(error) = start_servers(&manager, configs, &required).await {
        let _ = manager.shutdown_all().await;
        return Err(error);
    }
    let tools = manager.build_mcp_tools().await;
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

async fn start_servers(
    manager: &archon_mcp::lifecycle::McpServerManager,
    configs: Vec<ServerConfig>,
    required: &BTreeSet<String>,
) -> Result<()> {
    for config in configs.into_iter().filter(|config| !config.disabled) {
        let name = config.name.clone();
        let command = config.command.clone();
        let recovery = if command.is_empty() {
            "check the configured transport, endpoint and credentials and retry".to_string()
        } else {
            format!(
                "make {command} resolvable on the PATH archon is started with and check the server configuration, then retry"
            )
        };
        let failure = match tokio::time::timeout(
            Duration::from_secs(15),
            manager.start_all(vec![config]),
        )
        .await
        {
            Err(_) => format!(
                "workflow MCP server '{name}' (executable '{command}') startup timed out after 15s; {recovery}"
            ),
            Ok(errors) if errors.is_empty() => continue,
            Ok(errors) => format!(
                "workflow MCP server '{name}' (executable '{command}') failed to start: {}; {recovery}",
                errors
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join("; ")
            ),
        };
        if required.contains(&name) {
            return Err(anyhow!(failure));
        }
        tracing::warn!(
            error = %failure,
            "optional user MCP server unavailable; workflow subagents run without its tools"
        );
    }
    Ok(())
}

fn policy_by_server(
    configs: &[ServerConfig],
) -> BTreeMap<String, archon_mcp::types::McpToolBridgePolicy> {
    configs
        .iter()
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
mod tests {
    use super::*;

    #[tokio::test]
    async fn configured_mcp_start_failure_is_operational() {
        let manager = archon_mcp::lifecycle::McpServerManager::new();
        let config: ServerConfig = serde_json::from_value(serde_json::json!({
            "name": "required-project-tools", "command": "archon-274-missing-executable"
        }))
        .unwrap();
        let required = BTreeSet::from(["required-project-tools".to_string()]);
        let error = start_servers(&manager, vec![config], &required)
            .await
            .expect_err("configured project tools must not disappear after startup failure")
            .to_string();
        assert!(error.contains("required-project-tools"), "{error}");
        assert!(error.contains("archon-274-missing-executable"), "{error}");
        assert!(error.contains("PATH archon is started with"), "{error}");
    }

    #[tokio::test]
    async fn user_global_mcp_start_failure_is_not_fatal() {
        // A broken server only the user's global config declares must not
        // stop a workflow; the project declares a different server.
        let manager = archon_mcp::lifecycle::McpServerManager::new();
        let config: ServerConfig = serde_json::from_value(serde_json::json!({
            "name": "personal-tools", "command": "archon-274-missing-executable"
        }))
        .unwrap();
        let required = BTreeSet::from(["required-project-tools".to_string()]);
        start_servers(&manager, vec![config], &required)
            .await
            .expect("an optional user server that cannot start is a warning");
        assert!(manager.build_mcp_tools().await.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn only_project_declared_mcp_servers_are_required() {
        // The global config lives under HOME, so the child gets its own.
        let home = tempfile::tempdir().unwrap();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--ignored",
                "--exact",
                "command::workflow_mcp::tests::project_and_global_mcp_child",
                "--nocapture",
            ])
            .env("HOME", home.path())
            .env("XDG_CONFIG_HOME", home.path().join(".config"))
            .status()
            .unwrap();
        assert!(status.success());
    }

    #[cfg(unix)]
    #[tokio::test]
    #[ignore = "isolated HOME"]
    async fn project_and_global_mcp_child() {
        let servers = |name: &str| {
            serde_json::json!({"mcpServers": {name: {"command": format!("archon-274-missing-{name}")}}})
                .to_string()
        };
        let global = dirs::config_dir().unwrap().join("archon");
        std::fs::create_dir_all(&global).unwrap();
        std::fs::write(global.join(".mcp.json"), servers("personal-tools")).unwrap();
        let install = |root: std::path::PathBuf| async move {
            let mut registry = ToolRegistry::new();
            let mut rules = RuleSet::default();
            install_project_tools(&root, &mut registry, &mut rules).await
        };

        let without = tempfile::tempdir().unwrap();
        install(without.path().to_path_buf())
            .await
            .expect("a broken user-global server alone is not fatal");

        let with = tempfile::tempdir().unwrap();
        std::fs::write(with.path().join(".mcp.json"), servers("project-tools")).unwrap();
        let error = install(with.path().to_path_buf())
            .await
            .expect_err("a project server that cannot start fails the call")
            .to_string();
        assert!(error.contains("'project-tools'"), "{error}");
        assert!(!error.contains("personal-tools"), "{error}");
    }

    #[test]
    fn explicit_policy_allows_safe_and_risky_but_denies_unknown_and_dangerous() {
        let mut policy = archon_mcp::types::McpToolBridgePolicy::default();
        policy
            .tool_permissions
            .insert("read".into(), McpToolRisk::Safe);
        policy
            .tool_permissions
            .insert("compile".into(), McpToolRisk::Risky);
        policy
            .tool_permissions
            .insert("delete".into(), McpToolRisk::Dangerous);
        let policies = BTreeMap::from([("tv".to_string(), policy)]);
        let names = vec![
            "mcp__tv__read".to_string(),
            "mcp__tv__compile".to_string(),
            "mcp__tv__delete".to_string(),
            "mcp__tv__unknown".to_string(),
        ];
        let mut rules = RuleSet::empty();
        apply_explicit_policy(&mut rules, &names, &policies);
        assert_eq!(rules.always_allow.len(), 2);
        assert_eq!(rules.always_deny.len(), 2);
    }
}
