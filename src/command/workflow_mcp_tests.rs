use super::*;
use std::path::Path;

const DEADLINE: Duration = Duration::from_secs(5);

fn missing(name: &str) -> ServerConfig {
    serde_json::from_value(serde_json::json!({
        "name": name, "command": "archon-274-missing-executable"
    }))
    .unwrap()
}

#[tokio::test]
async fn configured_mcp_start_failure_is_operational() {
    let manager = archon_mcp::lifecycle::McpServerManager::new();
    let error = start_servers(&manager, vec![(missing("project-tools"), true)], DEADLINE)
        .await
        .map(|_| ())
        .expect_err("configured project tools must not disappear after startup failure")
        .to_string();
    assert!(error.contains("project-tools"), "{error}");
    assert!(error.contains("archon-274-missing-executable"), "{error}");
    assert!(error.contains("PATH archon is started with"), "{error}");
}

#[tokio::test]
async fn user_global_mcp_start_failure_is_not_fatal() {
    // A broken server only the user's global config declares must not stop
    // a workflow.
    let manager = archon_mcp::lifecycle::McpServerManager::new();
    let tools = start_servers(&manager, vec![(missing("personal-tools"), false)], DEADLINE)
        .await
        .expect("an optional user server that cannot start is a warning");
    assert!(tools.is_empty());
}

/// A stdio MCP server in POSIX sh, run as an argument of the system shell. It
/// answers `initialize`; `MODE` decides `tools/list` (or fails `initialize`).
#[cfg(unix)]
const FIXTURE_SERVER: &str = r#"
field() { printf '%s' "$1" | grep -o "\"$2\":[^,}]*" | head -n 1 | cut -d: -f2-; }
while IFS= read -r line; do
  id=$(field "$line" id)
  case "$line" in
    *'"method":"initialize"'*)
      if [ "$MODE" = init-error ]; then
        printf '{"jsonrpc":"2.0","id":%s,"error":{"code":-32000,"message":"rejected credential %s"}}\n' "$id" "$SECRET"
      else
        printf '{"jsonrpc":"2.0","id":%s,"result":{"protocolVersion":%s,"capabilities":{"tools":{}},"serverInfo":{"name":"fixture","version":"0"}}}\n' "$id" "$(field "$line" protocolVersion)"
      fi ;;
    *'"method":"tools/list"'*)
      case "$MODE" in
        hang) ;;
        list-error) printf '{"jsonrpc":"2.0","id":%s,"error":{"code":-32000,"message":"listing broke"}}\n' "$id" ;;
        empty) printf '{"jsonrpc":"2.0","id":%s,"result":{"tools":[]}}\n' "$id" ;;
        *) printf '{"jsonrpc":"2.0","id":%s,"result":{"tools":[{"name":"probe","inputSchema":{"type":"object"}}]}}\n' "$id" ;;
      esac ;;
  esac
done
"#;

#[cfg(unix)]
fn fixture(dir: &Path, mode: &str) -> ServerConfig {
    let script = dir.join("server.sh");
    std::fs::write(&script, FIXTURE_SERVER).unwrap();
    serde_json::from_value(serde_json::json!({
        "name": "project-tools", "command": "/bin/sh", "args": [script],
        "env": {"MODE": mode, "SECRET": "canary-secret-value-274"}
    }))
    .unwrap()
}

#[cfg(unix)]
#[tokio::test]
async fn required_server_must_list_tools_within_the_deadline() {
    let temp = tempfile::tempdir().unwrap();
    let manager = archon_mcp::lifecycle::McpServerManager::new();
    let tools = start_servers(
        &manager,
        vec![(fixture(temp.path(), "one"), true)],
        DEADLINE,
    )
    .await
    .expect("a working project server registers its tools");
    assert_eq!(tools.len(), 1);
    let _ = manager.shutdown_all().await;

    // Started and listed, just with no tools (a resources-only server): a
    // warning naming it, and the call proceeds.
    let manager = archon_mcp::lifecycle::McpServerManager::new();
    let tools = start_servers(
        &manager,
        vec![(fixture(temp.path(), "empty"), true)],
        DEADLINE,
    )
    .await
    .expect("a healthy server without tools does not fail the call");
    assert!(tools.is_empty());
    let _ = manager.shutdown_all().await;

    for (mode, expected) in [
        ("hang", "within"),
        ("list-error", "listing its tools failed"),
    ] {
        let manager = archon_mcp::lifecycle::McpServerManager::new();
        let started = std::time::Instant::now();
        let error = start_servers(
            &manager,
            vec![(fixture(temp.path(), mode), true)],
            Duration::from_secs(2),
        )
        .await
        .map(|_| ())
        .expect_err("a required server that cannot list its tools fails the call")
        .to_string();
        let _ = manager.shutdown_all().await;
        assert!(error.contains(expected), "{mode}: {error}");
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "{mode} waited unbounded"
        );
    }
}

#[cfg(unix)]
#[tokio::test]
async fn start_errors_do_not_carry_configured_values() {
    let temp = tempfile::tempdir().unwrap();
    let manager = archon_mcp::lifecycle::McpServerManager::new();
    let error = start_servers(
        &manager,
        vec![(fixture(temp.path(), "init-error"), true)],
        DEADLINE,
    )
    .await
    .map(|_| ())
    .expect_err("a rejected initialize fails the call")
    .to_string();
    let _ = manager.shutdown_all().await;
    assert!(!error.contains("canary-secret-value-274"), "{error}");
    assert!(error.contains("[REDACTED]"), "{error}");
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
        .map(|_| ())
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

#[cfg(unix)]
#[tokio::test]
async fn workflow_discovery_progress_has_no_total_time_limit() {
    let temp = tempfile::tempdir().unwrap();
    let script = temp.path().join("progress.sh");
    std::fs::write(&script, r#"
field() { printf '%s' "$1" | grep -o "\"$2\":[^,}]*" | head -n 1 | cut -d: -f2-; }
while IFS= read -r line; do
  id=$(field "$line" id)
  case "$line" in
    *'"method":"initialize"'*)
      printf '{"jsonrpc":"2.0","id":%s,"result":{"protocolVersion":%s,"capabilities":{"tools":{}},"serverInfo":{"name":"fixture","version":"0"}}}\n' "$id" "$(field "$line" protocolVersion)" ;;
    *'"method":"tools/list"'*)
      token=$(field "$line" progressToken)
      for step in 1 2 3 4; do
        sleep 0.1
        printf '{"jsonrpc":"2.0","method":"notifications/progress","params":{"progressToken":%s,"progress":%s}}\n' "$token" "$step"
      done
      printf '{"jsonrpc":"2.0","id":%s,"result":{"tools":[]}}\n' "$id" ;;
  esac
done
"#).unwrap();
    let config = serde_json::from_value(serde_json::json!({
        "name": "progress", "command": "/bin/sh", "args": [script]
    }))
    .unwrap();
    let manager = archon_mcp::lifecycle::McpServerManager::new();
    let result = start_servers(&manager, vec![(config, true)], Duration::from_millis(300)).await;
    let _ = manager.shutdown_all().await;
    assert!(
        result.is_ok(),
        "progressing discovery hit a total-time deadline: {:?}",
        result.err()
    );
}

#[tokio::test]
async fn round2_workflow_recovery_hint_preserves_ordinary_words() {
    let mut cfg = missing("credentials-service");
    cfg.command.clear();
    cfg.transport = "http".into();
    let manager = archon_mcp::lifecycle::McpServerManager::new();
    let error = start_servers(&manager, vec![(cfg, true)], DEADLINE)
        .await
        .map(|_| ())
        .expect_err("missing endpoint")
        .to_string();
    assert!(
        error.contains("check the configured transport, endpoint and credentials and retry"),
        "{error}"
    );
}
