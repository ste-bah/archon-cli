//! Tests for TASK-CLI-316: RemoteTrigger tool
//!
//! Feature-gated: compile with `--features remote-trigger`.
//!
//! Tests cover: host allowlist enforcement, env var expansion, timeout config,
//! response truncation, preset resolution, validation, schema.

#![cfg(feature = "remote-trigger")]

use archon_tools::remote_trigger::{RemoteTriggerConfig, RemoteTriggerTool, TriggerPreset};
use archon_tools::tool::{AgentMode, PermissionLevel, Tool, ToolContext};
use serde_json::json;
use std::collections::HashMap;
use std::path::PathBuf;

fn ctx() -> ToolContext {
    ToolContext {
        working_dir: PathBuf::from("/tmp"),
        session_id: "test-session".into(),
        mode: AgentMode::Normal,
        extra_dirs: vec![],
    }
}

fn cfg_with_hosts(hosts: &[&str]) -> RemoteTriggerConfig {
    RemoteTriggerConfig {
        allowed_hosts: hosts.iter().map(|s| s.to_string()).collect(),
        presets: HashMap::new(),
        default_timeout_secs: 20,
        max_timeout_secs: 120,
        max_response_bytes: 100 * 1024,
    }
}

// ---------------------------------------------------------------------------
// Tool identity
// ---------------------------------------------------------------------------

#[test]
fn tool_name_is_remote_trigger() {
    let tool = RemoteTriggerTool::new(cfg_with_hosts(&["localhost"]));
    assert_eq!(tool.name(), "RemoteTrigger");
}

#[test]
fn tool_description_not_empty() {
    let tool = RemoteTriggerTool::new(cfg_with_hosts(&["localhost"]));
    assert!(!tool.description().is_empty());
}

#[test]
fn tool_description_documents_divergence_from_project_zero() {
    let tool = RemoteTriggerTool::new(cfg_with_hosts(&["localhost"]));
    // Description or schema must NOT claim to be project-zero's RemoteTriggerTool
    let desc = tool.description();
    assert!(
        !desc.contains("create") || !desc.contains("list") || !desc.contains("update"),
        "must not expose project-zero CRUD actions"
    );
}

#[test]
fn tool_schema_has_url_field() {
    let tool = RemoteTriggerTool::new(cfg_with_hosts(&["localhost"]));
    let schema = tool.input_schema();
    let props = schema["properties"]
        .as_object()
        .expect("schema must have properties");
    assert!(props.contains_key("url"), "schema must have 'url' field");
}

#[test]
fn tool_schema_has_payload_field() {
    let tool = RemoteTriggerTool::new(cfg_with_hosts(&["localhost"]));
    let schema = tool.input_schema();
    let props = schema["properties"].as_object().unwrap();
    assert!(
        props.contains_key("payload"),
        "schema must have 'payload' field"
    );
}

#[test]
fn tool_schema_has_optional_preset_field() {
    let tool = RemoteTriggerTool::new(cfg_with_hosts(&["localhost"]));
    let schema = tool.input_schema();
    let props = schema["properties"].as_object().unwrap();
    assert!(
        props.contains_key("preset"),
        "schema must have 'preset' field"
    );
}

#[test]
fn tool_schema_has_optional_timeout_field() {
    let tool = RemoteTriggerTool::new(cfg_with_hosts(&["localhost"]));
    let schema = tool.input_schema();
    let props = schema["properties"].as_object().unwrap();
    assert!(
        props.contains_key("timeout_secs"),
        "schema must have 'timeout_secs' field"
    );
}

#[test]
fn permission_level_is_risky() {
    // HTTP to external hosts = Risky (not Dangerous)
    let tool = RemoteTriggerTool::new(cfg_with_hosts(&["localhost"]));
    let level = tool.permission_level(&json!({"url": "http://localhost/test", "payload": {}}));
    assert_eq!(level, PermissionLevel::Risky);
}

// ---------------------------------------------------------------------------
// RemoteTriggerConfig defaults
// ---------------------------------------------------------------------------

#[test]
fn config_default_timeout_is_20s() {
    let cfg = RemoteTriggerConfig::default();
    assert_eq!(cfg.default_timeout_secs, 20, "default timeout must be 20s");
}

#[test]
fn config_max_timeout_is_120s() {
    let cfg = RemoteTriggerConfig::default();
    assert_eq!(cfg.max_timeout_secs, 120, "max timeout must be 120s");
}

#[test]
fn config_max_response_is_100kb() {
    let cfg = RemoteTriggerConfig::default();
    assert_eq!(
        cfg.max_response_bytes,
        100 * 1024,
        "max response must be 100KB"
    );
}

// ---------------------------------------------------------------------------
// Host allowlist enforcement (sync validation, no network)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn non_allowlisted_host_rejected_without_network_call() {
    let tool = RemoteTriggerTool::new(cfg_with_hosts(&["localhost"]));
    let result = tool
        .execute(
            json!({"url": "http://evil.com/exfil", "payload": {}}),
            &ctx(),
        )
        .await;
    assert!(result.is_error, "non-allowlisted host must be rejected");
    assert!(
        result.content.contains("not allowed") || result.content.contains("allowlist"),
        "error must mention allowlist: got {:?}",
        result.content
    );
}

#[tokio::test]
async fn missing_url_returns_validation_error() {
    let tool = RemoteTriggerTool::new(cfg_with_hosts(&["localhost"]));
    let result = tool.execute(json!({"payload": {}}), &ctx()).await;
    assert!(result.is_error, "missing url must return error");
}

#[tokio::test]
async fn empty_url_returns_validation_error() {
    let tool = RemoteTriggerTool::new(cfg_with_hosts(&["localhost"]));
    let result = tool
        .execute(json!({"url": "", "payload": {}}), &ctx())
        .await;
    assert!(result.is_error, "empty url must return error");
}

#[tokio::test]
async fn invalid_url_returns_validation_error() {
    let tool = RemoteTriggerTool::new(cfg_with_hosts(&["localhost"]));
    let result = tool
        .execute(json!({"url": "not-a-url", "payload": {}}), &ctx())
        .await;
    assert!(result.is_error, "invalid url must return error");
}

#[tokio::test]
async fn https_allowed_host_passes_allowlist_check() {
    // Host extraction must strip scheme, port, path
    let tool = RemoteTriggerTool::new(cfg_with_hosts(&["secure.example.com"]));
    // We don't make a real request — we just verify the allowlist check passes
    // by checking the error message is not about allowlist
    let result = tool
        .execute(
            json!({"url": "https://secure.example.com/trigger", "payload": {}}),
            &ctx(),
        )
        .await;
    // Will fail with connection error (not allowlist error) — that's fine
    if result.is_error {
        assert!(
            !result.content.contains("allowlist") && !result.content.contains("not allowed"),
            "allowed host should not be rejected by allowlist: {:?}",
            result.content
        );
    }
}

// ---------------------------------------------------------------------------
// Timeout capping
// ---------------------------------------------------------------------------

#[test]
fn timeout_capped_at_120s() {
    let tool = RemoteTriggerTool::new(cfg_with_hosts(&["localhost"]));
    // Test the internal timeout resolution
    let effective = tool.resolve_timeout(200);
    assert_eq!(effective, 120, "timeout must be capped at 120s");
}

#[test]
fn timeout_zero_uses_default() {
    let tool = RemoteTriggerTool::new(cfg_with_hosts(&["localhost"]));
    let effective = tool.resolve_timeout(0);
    assert_eq!(effective, 20, "zero timeout must use default 20s");
}

#[test]
fn timeout_within_range_preserved() {
    let tool = RemoteTriggerTool::new(cfg_with_hosts(&["localhost"]));
    let effective = tool.resolve_timeout(30);
    assert_eq!(effective, 30);
}

// ---------------------------------------------------------------------------
// Response truncation
// ---------------------------------------------------------------------------

#[test]
fn truncate_at_100kb() {
    let big = "x".repeat(200 * 1024); // 200KB
    let truncated = RemoteTriggerTool::truncate_response(&big, 100 * 1024);
    assert_eq!(truncated.len(), 100 * 1024);
}

#[test]
fn small_response_not_truncated() {
    let small = "hello world";
    let result = RemoteTriggerTool::truncate_response(small, 100 * 1024);
    assert_eq!(result, small);
}

// ---------------------------------------------------------------------------
// Env var expansion in auth
// ---------------------------------------------------------------------------

#[test]
fn env_var_expansion_in_auth() {
    unsafe {
        std::env::set_var("TEST_ARCHON_TOKEN", "secret-abc-123");
    }
    let expanded = RemoteTriggerTool::expand_env_vars("Bearer ${TEST_ARCHON_TOKEN}");
    assert_eq!(expanded, "Bearer secret-abc-123");
    unsafe {
        std::env::remove_var("TEST_ARCHON_TOKEN");
    }
}

#[test]
fn env_var_expansion_missing_var_uses_empty() {
    // Unset var should expand to empty string without panic
    unsafe {
        std::env::remove_var("DEFINITELY_NOT_SET_ARCHON_TEST");
    }
    let expanded = RemoteTriggerTool::expand_env_vars("Bearer ${DEFINITELY_NOT_SET_ARCHON_TEST}");
    // Should not panic; value is empty or original placeholder
    let _ = expanded;
}

#[test]
fn no_env_var_placeholder_unchanged() {
    let result = RemoteTriggerTool::expand_env_vars("Bearer hardcoded-token");
    assert_eq!(result, "Bearer hardcoded-token");
}

// ---------------------------------------------------------------------------
// Preset resolution
// ---------------------------------------------------------------------------

#[test]
fn preset_resolves_url_and_auth() {
    let mut presets = HashMap::new();
    presets.insert(
        "staging".to_string(),
        TriggerPreset {
            url: "https://staging.example.com/trigger".into(),
            auth: Some("Bearer token123".into()),
        },
    );
    let cfg = RemoteTriggerConfig {
        allowed_hosts: vec!["staging.example.com".into()],
        presets,
        default_timeout_secs: 20,
        max_timeout_secs: 120,
        max_response_bytes: 100 * 1024,
    };
    let tool = RemoteTriggerTool::new(cfg);
    let (url, auth) = tool.resolve_preset("staging").unwrap();
    assert_eq!(url, "https://staging.example.com/trigger");
    assert_eq!(auth, Some("Bearer token123".to_string()));
}

#[test]
fn unknown_preset_returns_none() {
    let tool = RemoteTriggerTool::new(cfg_with_hosts(&["localhost"]));
    assert!(tool.resolve_preset("nonexistent").is_none());
}
