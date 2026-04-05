//! Builder pattern tests for archon-sdk (TASK-CLI-306).

use std::sync::Arc;

use archon_sdk::{
    SdkError, SdkTool,
    builder::{AgentBuilder, AgentOptions, PermissionMode, SessionBuilder, ThinkingConfig},
    create_sdk_mcp_server,
};

// ── helpers ───────────────────────────────────────────────────────────────────

fn noop_tool(name: &str) -> SdkTool {
    SdkTool {
        name: name.into(),
        description: "noop".into(),
        schema: serde_json::json!({ "type": "object" }),
        handler: Arc::new(|_| Box::pin(async { Ok("ok".to_string()) })),
    }
}

// ── AgentBuilder defaults ─────────────────────────────────────────────────────

#[test]
fn agent_builder_default_model_is_sonnet() {
    // Build with just an API key to satisfy required field
    let query = AgentBuilder::new().api_key("sk-test").build().unwrap();
    assert_eq!(query.options().model, "claude-sonnet-4-6");
}

#[test]
fn agent_builder_default_max_tokens() {
    let query = AgentBuilder::new().api_key("sk-test").build().unwrap();
    assert_eq!(query.options().max_tokens, 8096);
}

#[test]
fn agent_builder_default_permission_mode_auto() {
    let query = AgentBuilder::new().api_key("sk-test").build().unwrap();
    assert_eq!(query.options().permission_mode, PermissionMode::Auto);
}

#[test]
fn agent_builder_default_thinking_disabled() {
    let query = AgentBuilder::new().api_key("sk-test").build().unwrap();
    assert_eq!(query.options().thinking, ThinkingConfig::Disabled);
}

// ── AgentBuilder field setting ────────────────────────────────────────────────

#[test]
fn agent_builder_set_model() {
    let query = AgentBuilder::new()
        .api_key("sk-test")
        .model("claude-opus-4-6")
        .build()
        .unwrap();
    assert_eq!(query.options().model, "claude-opus-4-6");
}

#[test]
fn agent_builder_set_max_tokens() {
    let query = AgentBuilder::new()
        .api_key("sk-test")
        .max_tokens(4096)
        .build()
        .unwrap();
    assert_eq!(query.options().max_tokens, 4096);
}

#[test]
fn agent_builder_set_system_prompt() {
    let query = AgentBuilder::new()
        .api_key("sk-test")
        .system_prompt("You are helpful.")
        .build()
        .unwrap();
    assert_eq!(
        query.options().system_prompt.as_deref(),
        Some("You are helpful.")
    );
}

#[test]
fn agent_builder_set_thinking_enabled() {
    let query = AgentBuilder::new()
        .api_key("sk-test")
        .thinking(ThinkingConfig::Enabled {
            budget_tokens: 8192,
        })
        .build()
        .unwrap();
    assert_eq!(
        query.options().thinking,
        ThinkingConfig::Enabled {
            budget_tokens: 8192
        }
    );
}

#[test]
fn agent_builder_set_thinking_auto() {
    let query = AgentBuilder::new()
        .api_key("sk-test")
        .thinking(ThinkingConfig::Auto)
        .build()
        .unwrap();
    assert_eq!(query.options().thinking, ThinkingConfig::Auto);
}

#[test]
fn agent_builder_set_thinking_disabled() {
    let query = AgentBuilder::new()
        .api_key("sk-test")
        .thinking(ThinkingConfig::Disabled)
        .build()
        .unwrap();
    assert_eq!(query.options().thinking, ThinkingConfig::Disabled);
}

#[test]
fn agent_builder_set_permission_mode() {
    let query = AgentBuilder::new()
        .api_key("sk-test")
        .permission_mode(PermissionMode::Auto)
        .build()
        .unwrap();
    assert_eq!(query.options().permission_mode, PermissionMode::Auto);
}

#[test]
fn agent_builder_set_tool() {
    let server = create_sdk_mcp_server("tools", vec![noop_tool("echo")]);
    let query = AgentBuilder::new()
        .api_key("sk-test")
        .tool(server)
        .build()
        .unwrap();
    assert!(query.options().mcp_server.is_some());
}

#[test]
fn agent_builder_bearer_token() {
    let query = AgentBuilder::new()
        .bearer_token("bearer-xyz")
        .build()
        .unwrap();
    assert!(query.options().model == "claude-sonnet-4-6");
}

// ── Validation: missing required fields ───────────────────────────────────────

#[test]
fn agent_builder_empty_build_returns_error() {
    let result = AgentBuilder::new().build();
    assert!(result.is_err(), "empty builder should fail");
}

#[test]
fn agent_builder_empty_build_is_missing_api_key_error() {
    let err = AgentBuilder::new().build().unwrap_err();
    matches!(err, SdkError::MissingApiKey);
    assert!(err.to_string().contains("API key") || err.to_string().contains("auth"));
}

#[test]
fn agent_builder_empty_api_key_returns_error() {
    let result = AgentBuilder::new().api_key("").build();
    assert!(result.is_err());
}

// ── AgentOptions is Clone ─────────────────────────────────────────────────────

#[test]
fn agent_options_is_clone() {
    let query = AgentBuilder::new().api_key("sk-test").build().unwrap();
    let opts: AgentOptions = query.options().clone();
    assert_eq!(opts.model, query.options().model);
}

// ── SessionBuilder ────────────────────────────────────────────────────────────

#[test]
fn session_builder_empty_returns_error() {
    let result = SessionBuilder::new().build();
    assert!(result.is_err());
}

#[test]
fn session_builder_with_api_key_succeeds() {
    let session = SessionBuilder::new().api_key("sk-test").build().unwrap();
    let _ = session; // type assertion: SessionHandle exists
}

#[test]
fn session_builder_set_model() {
    let session = SessionBuilder::new()
        .api_key("sk-test")
        .model("claude-haiku-4-5")
        .build()
        .unwrap();
    let _ = session;
}

#[test]
fn session_builder_set_cwd() {
    let session = SessionBuilder::new()
        .api_key("sk-test")
        .cwd("/tmp/project")
        .build()
        .unwrap();
    let _ = session;
}

// ── ThinkingConfig variants compile ──────────────────────────────────────────

#[test]
fn thinking_config_all_variants() {
    let _: Vec<ThinkingConfig> = vec![
        ThinkingConfig::Disabled,
        ThinkingConfig::Auto,
        ThinkingConfig::Enabled {
            budget_tokens: 1024,
        },
        ThinkingConfig::Enabled {
            budget_tokens: 16384,
        },
    ];
}

// ── Documented example from spec compiles ────────────────────────────────────

#[test]
fn spec_example_compiles() {
    let my_tool = noop_tool("my_tool");
    let _query = AgentBuilder::new()
        .model("claude-sonnet-4-6")
        .api_key("sk-ant-test")
        .system_prompt("You are a helpful assistant.")
        .tool(create_sdk_mcp_server("my-tools", vec![my_tool]))
        .max_tokens(4096)
        .thinking(ThinkingConfig::Enabled {
            budget_tokens: 8192,
        })
        .permission_mode(PermissionMode::Auto)
        .build()
        .unwrap();
}
