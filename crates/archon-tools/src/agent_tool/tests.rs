use super::*;
use crate::tool::{PermissionLevel, Tool, ToolContext};
use serde_json::json;

fn make_ctx() -> ToolContext {
    ToolContext {
        working_dir: std::env::temp_dir(),
        session_id: "test-session".into(),
        mode: crate::tool::AgentMode::Normal,
        extra_dirs: vec![],
        ..Default::default()
    }
}

#[tokio::test]
async fn valid_input_returns_subagent_request() {
    // TASK-AGS-104: execute() now returns {agent_id,status}; validate
    // SubagentRequest shape directly via validate_and_build.
    let tool = AgentTool::new();
    let input = json!({
        "prompt": "Summarize the codebase",
        "model": "claude-sonnet-4-6",
        "allowed_tools": ["Read", "Glob"],
        "max_turns": 5
    });

    let request = tool.validate_and_build(&input).expect("valid input");
    assert_eq!(request.prompt, "Summarize the codebase");
    assert_eq!(request.model.as_deref(), Some("claude-sonnet-4-6"));
    assert_eq!(request.allowed_tools, vec!["Read", "Glob"]);
    assert_eq!(request.max_turns, 5);
    assert_eq!(request.timeout_secs, SubagentRequest::DEFAULT_TIMEOUT_SECS);
    assert!(!request.run_in_background);
    assert!(request.cwd.is_none());
}

#[tokio::test]
async fn missing_prompt_returns_error() {
    let tool = AgentTool::new();
    let input = json!({ "model": "claude-sonnet-4-6" });

    let result = tool.execute(input, &make_ctx()).await;
    assert!(result.is_error);
    assert!(
        result.content.contains("prompt"),
        "error should mention 'prompt': {}",
        result.content
    );
}

#[tokio::test]
async fn empty_prompt_returns_error() {
    let tool = AgentTool::new();
    let input = json!({ "prompt": "   " });

    let result = tool.execute(input, &make_ctx()).await;
    assert!(result.is_error);
    assert!(result.content.contains("prompt"));
}

#[tokio::test]
async fn default_max_turns_applied() {
    let tool = AgentTool::new();
    let input = json!({ "prompt": "Do something" });

    let request = tool.validate_and_build(&input).expect("valid input");
    assert_eq!(request.max_turns, SubagentRequest::DEFAULT_MAX_TURNS);
    assert_eq!(request.timeout_secs, SubagentRequest::DEFAULT_TIMEOUT_SECS);
    assert!(!request.run_in_background);
    assert!(request.cwd.is_none());
}

#[tokio::test]
async fn model_omitting_max_turns_uses_default() {
    let tool = AgentTool::new();
    let request = tool
        .validate_and_build(&json!({"prompt": "x"}))
        .expect("default applies");
    assert_eq!(request.max_turns, SubagentRequest::DEFAULT_MAX_TURNS);
}

#[tokio::test]
async fn blank_model_inherits_parent() {
    let tool = AgentTool::new();
    let request = tool
        .validate_and_build(&json!({"prompt": "x", "model": "   "}))
        .expect("blank model is ignored");

    assert!(request.model.is_none());
}

#[tokio::test]
async fn allowed_tools_parsed_from_array() {
    let tool = AgentTool::new();
    let input = json!({
        "prompt": "Refactor module",
        "allowed_tools": ["Read", "Write", "Edit"]
    });

    let request = tool.validate_and_build(&input).expect("valid input");
    assert_eq!(request.allowed_tools, vec!["Read", "Write", "Edit"]);
}

#[tokio::test]
async fn no_allowed_tools_gives_empty_vec() {
    let tool = AgentTool::new();
    let input = json!({ "prompt": "Analyze code" });

    let request = tool.validate_and_build(&input).expect("valid input");
    assert!(request.allowed_tools.is_empty());
}

#[tokio::test]
async fn invalid_max_turns_returns_error() {
    let tool = AgentTool::new();

    // Zero
    let result = tool
        .execute(json!({"prompt": "x", "max_turns": 0}), &make_ctx())
        .await;
    assert!(result.is_error);

    // Over MAX_TURNS_HARD_CAP (100_000)
    let result = tool
        .execute(json!({"prompt": "x", "max_turns": 100_001}), &make_ctx())
        .await;
    assert!(result.is_error);
}

#[test]
fn permission_level_is_risky() {
    let tool = AgentTool::new();
    assert_eq!(tool.permission_level(&json!({})), PermissionLevel::Risky);
}

#[tokio::test]
async fn subagent_type_parsed_when_present() {
    let tool = AgentTool::new();
    let input = json!({
        "prompt": "Review code",
        "subagent_type": "code-reviewer"
    });

    let request = tool.validate_and_build(&input).expect("valid input");
    assert_eq!(request.subagent_type.as_deref(), Some("code-reviewer"));
}

#[tokio::test]
async fn subagent_type_none_when_absent() {
    let tool = AgentTool::new();
    let input = json!({ "prompt": "Do something" });

    let request = tool.validate_and_build(&input).expect("valid input");
    assert!(request.subagent_type.is_none());
}

#[test]
fn subagent_type_backward_compatible_deserialization() {
    // JSON without subagent_type should deserialize fine (serde default)
    let json = r#"{
        "prompt": "test",
        "allowed_tools": [],
        "max_turns": 10,
        "timeout_secs": 300
    }"#;
    let request: SubagentRequest = serde_json::from_str(json).unwrap();
    assert!(request.subagent_type.is_none());
}

#[test]
fn subagent_type_serializes_to_json() {
    let request = SubagentRequest {
        prompt: "test".into(),
        model: None,
        allowed_tools: vec![],
        max_turns: 10,
        timeout_secs: 300,
        subagent_type: Some("code-reviewer".into()),
        run_in_background: false,
        cwd: None,
        isolation: None,
        provider_env: None,
    };
    let json = serde_json::to_value(&request).unwrap();
    assert_eq!(json["subagent_type"], "code-reviewer");
}

#[test]
fn provider_env_policy_is_internal_only() {
    let request = SubagentRequest {
        prompt: "test".into(),
        model: None,
        allowed_tools: vec![],
        max_turns: 10,
        timeout_secs: 300,
        subagent_type: None,
        run_in_background: false,
        cwd: None,
        isolation: None,
        provider_env: Some(crate::provider_env::ProviderEnvPolicy::new(vec![
            "POLYGON_API_KEY".to_string(),
        ])),
    };
    let json = serde_json::to_value(&request).unwrap();
    assert!(json.get("provider_env").is_none());

    let injected: SubagentRequest = serde_json::from_value(serde_json::json!({
        "prompt": "test",
        "allowed_tools": [],
        "max_turns": 10,
        "timeout_secs": 300,
        "provider_env": {"required_keys": ["POLYGON_API_KEY"]}
    }))
    .unwrap();
    assert!(injected.provider_env.is_none());
}

#[test]
fn schema_includes_subagent_type() {
    let tool = AgentTool::new();
    let schema = tool.input_schema();
    let props = schema["properties"].as_object().unwrap();
    assert!(props.contains_key("subagent_type"));
    assert_eq!(props["subagent_type"]["type"], "string");
}

#[test]
fn agent_tool_schema_does_not_expose_max_turns() {
    let tool = AgentTool::new();
    let schema = tool.input_schema();
    let props = schema["properties"].as_object().expect("properties");
    assert!(
        !props.contains_key("max_turns"),
        "AgentTool schema must not advertise max_turns"
    );
}

#[tokio::test]
async fn run_in_background_parsed_when_present() {
    let tool = AgentTool::new();
    let input = json!({
        "prompt": "Review code",
        "run_in_background": true
    });

    let request = tool.validate_and_build(&input).expect("valid input");
    assert!(request.run_in_background);
}

#[test]
fn run_in_background_defaults_to_false() {
    let json = r#"{
        "prompt": "test",
        "allowed_tools": [],
        "max_turns": 10,
        "timeout_secs": 300
    }"#;
    let request: SubagentRequest = serde_json::from_str(json).unwrap();
    assert!(!request.run_in_background);
}

#[test]
fn run_in_background_serializes_to_json() {
    let request = SubagentRequest {
        prompt: "test".into(),
        model: None,
        allowed_tools: vec![],
        max_turns: 10,
        timeout_secs: 300,
        subagent_type: None,
        run_in_background: true,
        cwd: None,
        isolation: None,
        provider_env: None,
    };
    let json = serde_json::to_value(&request).unwrap();
    assert_eq!(json["run_in_background"], true);
}

#[tokio::test]
async fn cwd_parsed_when_present() {
    let tool = AgentTool::new();
    let input = json!({
        "prompt": "Review code",
        "cwd": "/tmp"
    });

    let request = tool.validate_and_build(&input).expect("valid input");
    assert_eq!(request.cwd.as_deref(), Some("/tmp"));
}

#[test]
fn cwd_defaults_to_none() {
    let json = r#"{
        "prompt": "test",
        "allowed_tools": [],
        "max_turns": 10,
        "timeout_secs": 300
    }"#;
    let request: SubagentRequest = serde_json::from_str(json).unwrap();
    assert!(request.cwd.is_none());
}

#[test]
fn cwd_serializes_to_json() {
    let request = SubagentRequest {
        prompt: "test".into(),
        model: None,
        allowed_tools: vec![],
        max_turns: 10,
        timeout_secs: 300,
        subagent_type: None,
        run_in_background: false,
        cwd: Some("/tmp".into()),
        isolation: None,
        provider_env: None,
    };
    let json = serde_json::to_value(&request).unwrap();
    assert_eq!(json["cwd"], "/tmp");
}

// -----------------------------------------------------------------------
// Worktree isolation tests (AGT-017)
// -----------------------------------------------------------------------

#[tokio::test]
async fn isolation_worktree_parsed_when_present() {
    let tool = AgentTool::new();
    let input = json!({
        "prompt": "Review code",
        "isolation": "worktree"
    });

    let request = tool.validate_and_build(&input).expect("valid input");
    assert_eq!(request.isolation.as_deref(), Some("worktree"));
}

#[tokio::test]
async fn isolation_none_when_absent() {
    let tool = AgentTool::new();
    let input = json!({ "prompt": "Do something" });

    let request = tool.validate_and_build(&input).expect("valid input");
    assert!(request.isolation.is_none());
}

#[tokio::test]
async fn isolation_none_string_parsed_as_absent() {
    let tool = AgentTool::new();
    let input = json!({
        "prompt": "Read the code",
        "isolation": "none"
    });

    let request = tool.validate_and_build(&input).expect("valid input");
    assert!(request.isolation.is_none());
}

#[tokio::test]
async fn invalid_isolation_returns_error() {
    let tool = AgentTool::new();
    let result = tool
        .execute(
            json!({"prompt": "Read the code", "isolation": "inplace"}),
            &make_ctx(),
        )
        .await;

    assert!(result.is_error);
    assert!(result.content.contains("isolation must be"));
}

#[test]
fn isolation_backward_compatible_deserialization() {
    let json = r#"{
        "prompt": "test",
        "allowed_tools": [],
        "max_turns": 10,
        "timeout_secs": 300
    }"#;
    let request: SubagentRequest = serde_json::from_str(json).unwrap();
    assert!(request.isolation.is_none());
}

#[test]
fn isolation_serializes_to_json() {
    let request = SubagentRequest {
        prompt: "test".into(),
        model: None,
        allowed_tools: vec![],
        max_turns: 10,
        timeout_secs: 300,
        subagent_type: None,
        run_in_background: false,
        cwd: None,
        isolation: Some("worktree".into()),
        provider_env: None,
    };
    let json = serde_json::to_value(&request).unwrap();
    assert_eq!(json["isolation"], "worktree");
}

#[test]
fn schema_includes_isolation() {
    let tool = AgentTool::new();
    let schema = tool.input_schema();
    let props = schema["properties"].as_object().unwrap();
    assert!(props.contains_key("isolation"));
    assert_eq!(props["isolation"]["type"], "string");
    assert_eq!(props["isolation"]["enum"][0], "none");
    assert_eq!(props["isolation"]["enum"][1], "worktree");
}

#[test]
fn schema_includes_run_in_background_and_cwd() {
    let tool = AgentTool::new();
    let schema = tool.input_schema();
    let props = schema["properties"].as_object().unwrap();
    assert!(props.contains_key("run_in_background"));
    assert_eq!(props["run_in_background"]["type"], "boolean");
    assert!(props.contains_key("cwd"));
    assert_eq!(props["cwd"]["type"], "string");
}

#[test]
fn agent_listing_is_capped_and_description_bounded() {
    let agents: Vec<_> = (0..30)
        .map(|idx| {
            (
                format!("agent-{idx:02}"),
                "long description that should not leak the whole catalog".repeat(20),
            )
        })
        .collect();

    let tool = AgentTool::with_agent_listing(&agents);

    assert!(tool.description().len() <= AGENT_DESCRIPTION_LIMIT_BYTES);
    assert!(tool.description().contains("agent-00"));
    assert!(tool.description().contains("AgentCatalog"));
    assert!(!tool.description().contains("agent-29"));
}

#[test]
fn agent_catalog_lists_searches_and_infos_sorted_agents() {
    let tool = AgentCatalogTool::new(vec![
        ("zeta".into(), "last".into()),
        ("sherlock-holmes".into(), "forensic reviewer".into()),
        ("builder".into(), "implementation agent".into()),
    ]);

    let listed = tool.list(&json!({"action": "list", "limit": 2}));
    let listed_agents = listed["agents"].as_array().unwrap();
    assert_eq!(listed["total"], 3);
    assert_eq!(listed_agents[0]["name"], "builder");
    assert_eq!(listed_agents[1]["name"], "sherlock-holmes");

    let searched = tool.search(&json!({"action": "search", "query": "forensic"}));
    assert_eq!(searched["agents"][0]["name"], "sherlock-holmes");

    let info = tool.info(&json!({"name": "zeta"})).unwrap();
    assert_eq!(info["agent"]["description"], "last");
}

#[test]
fn tool_metadata() {
    let tool = AgentTool::new();
    assert_eq!(tool.name(), "Agent");
    assert!(!tool.description().is_empty());

    let schema = tool.input_schema();
    assert_eq!(schema["type"], "object");
    assert!(
        schema["required"]
            .as_array()
            .unwrap()
            .contains(&json!("prompt"))
    );
}
