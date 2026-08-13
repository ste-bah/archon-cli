use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::time::Duration;

use archon_core::dispatch::ToolRegistry;
use archon_mcp::types::{McpToolRisk, ServerConfig};
use archon_permissions::rules::{RuleSet, ToolRule};
use archon_tools::tool::Tool;

pub(crate) async fn install_project_tools(
    project_root: &Path,
    registry: &mut ToolRegistry,
    rules: &mut RuleSet,
) {
    let root = mcp_config_root(project_root);
    let configs = match archon_mcp::config::load_merged_configs(&root) {
        Ok(configs) => configs,
        Err(error) => {
            tracing::warn!(%error, "workflow MCP config unavailable");
            return;
        }
    };
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
        return;
    }
    let policies = policy_by_server(&configs);
    let manager = archon_mcp::lifecycle::McpServerManager::new();
    start_servers(&manager, configs).await;
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
}

/// The nearest ancestor of `start` holding a `.mcp.json`, else `start`.
///
/// `load_merged_configs` joins `.mcp.json` to exactly one directory. That was
/// fine while subagents ran in the project root, and stopped being fine when
/// write branches moved into git worktrees: a worktree is a checkout of the
/// REPOSITORY, `.mcp.json` is untracked project configuration, so the file is
/// absent there and can never appear. Every worktree agent silently lost its
/// project MCP tools — while the worktrees themselves sit under the very
/// project root that holds the file.
///
/// Walking up recovers it wherever the agent is placed, and leaves a run in an
/// unrelated directory exactly as it was.
fn mcp_config_root(start: &Path) -> std::path::PathBuf {
    start
        .ancestors()
        .find(|ancestor| ancestor.join(".mcp.json").is_file())
        .unwrap_or(start)
        .to_path_buf()
}

async fn start_servers(
    manager: &archon_mcp::lifecycle::McpServerManager,
    configs: Vec<ServerConfig>,
) {
    match tokio::time::timeout(Duration::from_secs(15), manager.start_all(configs)).await {
        Ok(errors) => {
            for error in errors {
                tracing::warn!(%error, "workflow MCP server start failed");
            }
        }
        Err(_) => tracing::warn!("workflow MCP startup timed out after 15s"),
    }
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

    /// The live regression: a write branch runs inside
    /// `<project>/.archon/workflows/<run>/v2/worktrees/<branch>/<item>`, a
    /// checkout of the REPOSITORY. `.mcp.json` is untracked project config, so
    /// it is absent there and always will be — and the lookup joined
    /// `.mcp.json` to that directory alone, so every worktree agent silently
    /// got no MCP tools at all.
    #[test]
    fn a_worktree_agent_finds_the_project_mcp_config_above_it() {
        let project = tempfile::tempdir().expect("project");
        let project_root = project.path();
        std::fs::write(project_root.join(".mcp.json"), "{\"mcpServers\":{}}").expect("config");
        let worktree =
            project_root.join(".archon/workflows/wf-1/v2/worktrees/implementation-wave-1/item-abc");
        std::fs::create_dir_all(&worktree).expect("worktree");

        assert_eq!(
            mcp_config_root(&worktree),
            project_root,
            "the config above the worktree must be found"
        );
    }

    /// A directory with its own `.mcp.json` still wins over any ancestor.
    #[test]
    fn the_nearest_config_wins() {
        let outer = tempfile::tempdir().expect("outer");
        std::fs::write(outer.path().join(".mcp.json"), "{}").expect("outer config");
        let inner = outer.path().join("nested/project");
        std::fs::create_dir_all(&inner).expect("inner");
        std::fs::write(inner.join(".mcp.json"), "{}").expect("inner config");

        assert_eq!(mcp_config_root(&inner), inner);
    }

    /// No config anywhere: the caller's own root is returned unchanged, so a
    /// run outside any project behaves exactly as before.
    #[test]
    fn a_root_with_no_config_anywhere_is_returned_unchanged() {
        let bare = tempfile::tempdir().expect("bare");
        let nested = bare.path().join("a/b");
        std::fs::create_dir_all(&nested).expect("nested");
        assert_eq!(mcp_config_root(&nested), nested);
    }
}
