//! Parser for `.mcp.json` configuration files.
//!
//! Loads server definitions from both project-local and global config,
//! merges them (project overrides global), filters disabled servers,
//! and expands environment variable references in env values.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::types::{McpError, McpToolBridgePolicy, ServerConfig};

/// Raw on-disk shape of a single server entry inside `mcpServers`.
#[derive(Debug, Deserialize)]
struct RawServerEntry {
    #[serde(default)]
    command: String,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    env: HashMap<String, String>,
    #[serde(default)]
    disabled: bool,
    #[serde(default = "default_transport")]
    transport: String,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    headers: Option<HashMap<String, String>>,
    #[serde(default, rename = "allowInsecureWs", alias = "allow_insecure_ws")]
    allow_insecure_ws: bool,
    #[serde(default, rename = "toolPolicy", alias = "tool_policy")]
    tool_policy: McpToolBridgePolicy,
}

fn default_transport() -> String {
    "stdio".into()
}

/// Top-level shape of a `.mcp.json` file.
#[derive(Debug, Deserialize)]
struct RawMcpConfig {
    #[serde(default, rename = "mcpServers")]
    mcp_servers: HashMap<String, RawServerEntry>,
}

/// The nearest ancestor of `start` holding a `.mcp.json`, else `start` itself.
///
/// `load_merged_configs` joins `.mcp.json` to exactly one directory. That held
/// while agents ran in the project root, and broke when workflow write
/// branches moved into git worktrees: a worktree is a checkout of the
/// REPOSITORY, `.mcp.json` is untracked PROJECT configuration, so the file is
/// absent there and can never appear. Every worktree agent silently received
/// an empty tool registry — while the worktrees sit under the very project
/// root that holds the file.
///
/// Walking up recovers it wherever an agent is placed. A directory with its
/// own config still wins, and a path under no project is returned unchanged.
pub fn nearest_config_root(start: &Path) -> PathBuf {
    start
        .ancestors()
        .find(|ancestor| ancestor.join(".mcp.json").is_file())
        .unwrap_or(start)
        .to_path_buf()
}

/// Load and merge MCP server configs from project and global paths.
///
/// Project-local entries override global entries by server name.
/// Disabled servers are excluded from the returned list.
/// Missing files are silently ignored; malformed files return an error.
pub fn load_merged_configs(project_root: &Path) -> Result<Vec<ServerConfig>, McpError> {
    let global_path = global_config_path();
    let project_path = project_root.join(".mcp.json");

    let mut merged: HashMap<String, ServerConfig> = HashMap::new();

    // Global first (lower priority)
    if let Some(path) = global_path {
        for cfg in load_config_file(&path)? {
            merged.insert(cfg.name.clone(), cfg);
        }
    }

    // Project-local overrides global
    for cfg in load_config_file(&project_path)? {
        merged.insert(cfg.name.clone(), cfg);
    }

    // Filter out disabled servers
    let configs: Vec<ServerConfig> = merged.into_values().filter(|c| !c.disabled).collect();

    Ok(configs)
}

/// Load server configs from a single `.mcp.json` file.
///
/// Returns an empty vec if the file does not exist.
pub fn load_config_file(path: &Path) -> Result<Vec<ServerConfig>, McpError> {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(McpError::ConfigIo(e)),
    };

    parse_mcp_json(&content, path)
}

/// Parse the contents of a `.mcp.json` string into server configs.
fn parse_mcp_json(content: &str, source: &Path) -> Result<Vec<ServerConfig>, McpError> {
    let raw: RawMcpConfig = serde_json::from_str(content)
        .map_err(|e| McpError::ConfigParse(format!("{}: {}", source.display(), e)))?;

    let configs = raw
        .mcp_servers
        .into_iter()
        .map(|(name, entry)| {
            let env = expand_env_vars(&entry.env);
            ServerConfig {
                name,
                command: entry.command,
                args: entry.args,
                env,
                disabled: entry.disabled,
                transport: entry.transport,
                url: entry.url,
                headers: entry.headers,
                allow_insecure_ws: entry.allow_insecure_ws,
                tool_policy: entry.tool_policy,
            }
        })
        .collect();

    Ok(configs)
}

/// Expand `${VAR}` references in environment variable values.
///
/// Unresolved variables are replaced with an empty string.
fn expand_env_vars(env: &HashMap<String, String>) -> HashMap<String, String> {
    let re = regex::Regex::new(r"\$\{([^}]+)\}").expect("valid regex");
    env.iter()
        .map(|(k, v)| {
            let expanded = re.replace_all(v, |caps: &regex::Captures| {
                let var_name = &caps[1];
                std::env::var(var_name).unwrap_or_default()
            });
            (k.clone(), expanded.into_owned())
        })
        .collect()
}

/// Return the global config path: `~/.config/archon/.mcp.json`.
fn global_config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("archon").join(".mcp.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_empty_object() {
        let configs = parse_mcp_json("{}", Path::new("test.json")).expect("parse");
        assert!(configs.is_empty());
    }

    /// The live regression: a workflow write branch runs inside
    /// `<project>/.archon/workflows/<run>/v2/worktrees/<branch>/<item>`, a
    /// checkout of the REPOSITORY. `.mcp.json` is untracked project config, so
    /// it is absent there and always will be. Joining `.mcp.json` to that
    /// directory alone found nothing, and every worktree agent silently got no
    /// MCP tools at all.
    #[test]
    fn a_worktree_finds_the_project_config_above_it() {
        let project = tempfile::tempdir().expect("project");
        let root = project.path();
        std::fs::write(root.join(".mcp.json"), r#"{"mcpServers":{}}"#).expect("config");
        let worktree = root.join(".archon/workflows/wf-1/v2/worktrees/impl-wave-1/item-abc");
        std::fs::create_dir_all(&worktree).expect("worktree");

        assert_eq!(nearest_config_root(&worktree), root);
    }

    #[test]
    fn the_nearest_config_wins_over_an_ancestor() {
        let outer = tempfile::tempdir().expect("outer");
        std::fs::write(outer.path().join(".mcp.json"), "{}").expect("outer");
        let inner = outer.path().join("nested/project");
        std::fs::create_dir_all(&inner).expect("inner dir");
        std::fs::write(inner.join(".mcp.json"), "{}").expect("inner");

        assert_eq!(nearest_config_root(&inner), inner);
    }

    /// No config anywhere above: the caller's own path is returned, so a run
    /// outside any project behaves exactly as it did before.
    #[test]
    fn a_path_under_no_project_is_returned_unchanged() {
        let bare = tempfile::tempdir().expect("bare");
        let nested = bare.path().join("a/b");
        std::fs::create_dir_all(&nested).expect("nested");

        assert_eq!(nearest_config_root(&nested), nested);
    }

    #[test]
    fn parse_empty_servers() {
        let json = r#"{"mcpServers": {}}"#;
        let configs = parse_mcp_json(json, Path::new("test.json")).expect("parse");
        assert!(configs.is_empty());
    }

    #[test]
    fn parse_single_server() {
        let json = r#"{
            "mcpServers": {
                "filesystem": {
                    "command": "npx",
                    "args": ["-y", "@anthropic/mcp-filesystem"],
                    "env": {"HOME": "/home/user"}
                }
            }
        }"#;
        let configs = parse_mcp_json(json, Path::new("test.json")).expect("parse");
        assert_eq!(configs.len(), 1);
        let cfg = &configs[0];
        assert_eq!(cfg.name, "filesystem");
        assert_eq!(cfg.command, "npx");
        assert_eq!(cfg.args, vec!["-y", "@anthropic/mcp-filesystem"]);
        assert!(!cfg.disabled);
    }

    #[test]
    fn parse_disabled_server() {
        let json = r#"{
            "mcpServers": {
                "disabled-server": {
                    "command": "node",
                    "args": ["server.js"],
                    "disabled": true
                }
            }
        }"#;
        let configs = parse_mcp_json(json, Path::new("test.json")).expect("parse");
        assert_eq!(configs.len(), 1);
        assert!(configs[0].disabled);
    }

    #[test]
    fn parse_invalid_json_returns_error() {
        let result = parse_mcp_json("not json", Path::new("bad.json"));
        assert!(result.is_err());
        let err = result.unwrap_err();
        match err {
            McpError::ConfigParse(msg) => assert!(msg.contains("bad.json")),
            other => panic!("expected ConfigParse, got {other:?}"),
        }
    }

    #[test]
    fn env_var_expansion() {
        // Set a known env var for testing
        // SAFETY: This test runs single-threaded and the var is cleaned up.
        unsafe { std::env::set_var("ARCHON_TEST_VAR", "expanded_value") };
        let mut env = HashMap::new();
        env.insert("MY_VAR".into(), "${ARCHON_TEST_VAR}/subpath".into());
        env.insert("PLAIN".into(), "no_expansion".into());

        let result = expand_env_vars(&env);
        assert_eq!(result.get("MY_VAR").unwrap(), "expanded_value/subpath");
        assert_eq!(result.get("PLAIN").unwrap(), "no_expansion");

        // SAFETY: cleanup from above set_var
        unsafe { std::env::remove_var("ARCHON_TEST_VAR") };
    }

    #[test]
    fn env_var_expansion_missing_var() {
        let mut env = HashMap::new();
        env.insert("KEY".into(), "${NONEXISTENT_VAR_12345}".into());
        let result = expand_env_vars(&env);
        assert_eq!(result.get("KEY").unwrap(), "");
    }

    #[test]
    fn load_missing_file_returns_empty() {
        let configs =
            load_config_file(Path::new("/nonexistent/.mcp.json")).expect("should succeed");
        assert!(configs.is_empty());
    }

    #[test]
    fn merged_config_project_overrides_global() {
        let dir = tempfile::tempdir().expect("tmpdir");

        // Create a project .mcp.json with one server
        let project_json = r#"{
            "mcpServers": {
                "server-a": {
                    "command": "project-cmd",
                    "args": ["--project"]
                }
            }
        }"#;
        std::fs::write(dir.path().join(".mcp.json"), project_json).expect("write");

        let configs = load_merged_configs(dir.path()).expect("load");
        // Should have at least the project server (global may or may not exist)
        let server_a = configs.iter().find(|c| c.name == "server-a");
        assert!(server_a.is_some());
        assert_eq!(server_a.unwrap().command, "project-cmd");
    }

    #[test]
    fn merged_config_filters_disabled() {
        let dir = tempfile::tempdir().expect("tmpdir");

        let json = r#"{
            "mcpServers": {
                "enabled": {"command": "echo", "args": []},
                "disabled": {"command": "echo", "args": [], "disabled": true}
            }
        }"#;
        std::fs::write(dir.path().join(".mcp.json"), json).expect("write");

        let configs = load_merged_configs(dir.path()).expect("load");
        assert!(configs.iter().any(|c| c.name == "enabled"));
        assert!(!configs.iter().any(|c| c.name == "disabled"));
    }

    #[test]
    fn merged_config_empty_dir() {
        let dir = tempfile::tempdir().expect("tmpdir");
        let configs = load_merged_configs(dir.path()).expect("load");
        // May include global configs, but should not error
        // At minimum, no crash on missing .mcp.json
        let _ = configs;
    }
}
