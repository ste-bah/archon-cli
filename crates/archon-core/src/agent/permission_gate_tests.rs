//! `permission_gate.rs` coverage, split out for the 500-line file-size gate.
//!
//! Declared with `#[path]` from `permission_gate.rs`, so `super` still means
//! that module and the assertions read exactly as they did in place.

use super::*;
use archon_llm::provider::{LlmError, LlmResponse, ModelInfo, ProviderFeature};
use archon_permissions::rules::{RuleSet, ToolRule};

struct MockLlmProvider;

#[derive(Debug)]
struct DenyBlockedWriteSandbox;

impl archon_permissions::SandboxBackend for DenyBlockedWriteSandbox {
    fn check(
        &self,
        tool: &str,
        _capability: archon_permissions::ToolCapability,
        input: &serde_json::Value,
    ) -> Result<(), String> {
        if tool == "Write" && input.get("file_path").and_then(|v| v.as_str()) == Some("/blocked") {
            Err("sandbox blocked mutated write path".to_string())
        } else {
            Ok(())
        }
    }

    fn terminal(
        &self,
        _request: &archon_permissions::SandboxTerminalRequest,
    ) -> archon_permissions::SandboxTerminal {
        archon_permissions::SandboxTerminal::Host
    }

    fn scope_support(
        &self,
        _scope: archon_permissions::SandboxScope,
    ) -> archon_permissions::SandboxScopeSupport {
        archon_permissions::SandboxScopeSupport::Durable
    }
}

#[async_trait::async_trait]
impl LlmProvider for MockLlmProvider {
    fn name(&self) -> &str {
        "mock"
    }

    fn models(&self) -> Vec<ModelInfo> {
        vec![]
    }

    fn supports_feature(&self, _: ProviderFeature) -> bool {
        false
    }

    async fn stream(
        &self,
        _request: LlmRequest,
    ) -> Result<tokio::sync::mpsc::Receiver<StreamEvent>, LlmError> {
        let (_tx, rx) = tokio::sync::mpsc::channel(1);
        Ok(rx)
    }

    async fn complete(&self, _request: LlmRequest) -> Result<LlmResponse, LlmError> {
        unimplemented!()
    }
}

fn agent_with_rules(mode: &str, rules: RuleSet) -> Agent {
    agent_with_rules_and_events(mode, rules).0
}

fn agent_with_rules_and_events(
    mode: &str,
    rules: RuleSet,
) -> (Agent, tokio::sync::mpsc::Receiver<TimestampedEvent>) {
    let (tx, rx) = tokio::sync::mpsc::channel(AGENT_EVENT_CHANNEL_CAPACITY);
    let config = AgentConfig {
        permission_mode: Arc::new(Mutex::new(mode.to_string())),
        permission_rules: rules,
        ..AgentConfig::default()
    };
    let agent = Agent::new(
        Arc::new(MockLlmProvider),
        ToolRegistry::new(),
        config,
        tx,
        Arc::new(std::sync::RwLock::new(AgentRegistry::load(
            &std::env::temp_dir(),
        ))),
    );
    (agent, rx)
}

fn agent_with_registry_and_sandbox(
    registry: ToolRegistry,
    sandbox: Arc<dyn archon_permissions::SandboxBackend>,
) -> Agent {
    let (tx, _rx) = tokio::sync::mpsc::channel(AGENT_EVENT_CHANNEL_CAPACITY);
    let config = AgentConfig {
        permission_mode: Arc::new(Mutex::new("bypassPermissions".to_string())),
        sandbox: Some(sandbox),
        // The sandbox check is the subject here. These fixtures write to
        // paths nothing has read, which read_before_edit refuses by design
        // (#193 Phase A).
        filesystem: crate::config::FilesystemConfig {
            read_before_edit: crate::config::ReadBeforeEdit::Off,
        },
        ..AgentConfig::default()
    };
    Agent::new(
        Arc::new(MockLlmProvider),
        registry,
        config,
        tx,
        Arc::new(std::sync::RwLock::new(AgentRegistry::load(
            &std::env::temp_dir(),
        ))),
    )
}

#[tokio::test]
async fn preflight_deny_rule_blocks_bypass_permissions_before_lookup() {
    let mut rules = RuleSet::empty();
    rules.always_deny.push(ToolRule {
        tool: "Bash".to_string(),
        pattern: "*".to_string(),
    });
    let mut agent = agent_with_rules("bypassPermissions", rules);
    let pending = [PendingToolCall {
        id: "tool-1".to_string(),
        name: "Bash".to_string(),
        input_json: r#"{"command":"cargo test"}"#.to_string(),
    }];

    let allowed = agent.preflight_tools(&pending, AgentMode::Normal).await;

    assert!(allowed.is_empty());
    let tool_result = &agent.state.messages[0]["content"][0];
    assert_eq!(tool_result["tool_use_id"], "tool-1");
    assert_eq!(tool_result["is_error"], true);
    assert!(
        tool_result["content"]
            .as_str()
            .unwrap_or_default()
            .contains("Blocked by deny rule")
    );
}

#[tokio::test]
async fn preflight_deny_rule_blocks_dont_ask_mode() {
    let mut rules = RuleSet::empty();
    rules.always_deny.push(ToolRule {
        tool: "Bash".to_string(),
        pattern: "*".to_string(),
    });
    let mut agent = agent_with_rules("dontAsk", rules);
    let pending = [PendingToolCall {
        id: "tool-1".to_string(),
        name: "Bash".to_string(),
        input_json: r#"{"command":"cargo test"}"#.to_string(),
    }];

    let allowed = agent.preflight_tools(&pending, AgentMode::Normal).await;

    assert!(allowed.is_empty());
    let tool_result = &agent.state.messages[0]["content"][0];
    assert_eq!(tool_result["tool_use_id"], "tool-1");
    assert_eq!(tool_result["is_error"], true);
    assert!(
        tool_result["content"]
            .as_str()
            .unwrap_or_default()
            .contains("Blocked by deny rule")
    );
}

#[tokio::test]
async fn pretool_hook_deny_records_denial_event_and_log() {
    let (mut agent, mut rx) = agent_with_rules_and_events("bypassPermissions", RuleSet::empty());
    let registry = Arc::new(crate::hooks::HookRegistry::new());
    let callback: crate::hooks::HookCallback = Arc::new(|_| crate::hooks::HookResult {
        permission_behavior: Some(crate::hooks::PermissionBehavior::Deny),
        permission_decision_reason: Some("hook policy denied".to_string()),
        source_authority: Some(crate::hooks::SourceAuthority::Policy),
        ..Default::default()
    });
    registry.register_callback(
        crate::hooks::HookEvent::PreToolUse,
        crate::hooks::HookCallbackEntry {
            name: "deny-bash".to_string(),
            callback,
            authority: crate::hooks::SourceAuthority::Policy,
            timeout_secs: 1,
        },
    );
    agent.set_hook_registry(registry);
    let pending = [PendingToolCall {
        id: "tool-1".to_string(),
        name: "Bash".to_string(),
        input_json: r#"{"command":"cargo test"}"#.to_string(),
    }];

    let allowed = agent.preflight_tools(&pending, AgentMode::Normal).await;

    assert!(allowed.is_empty());
    let recent = {
        let log = agent.denial_log.lock().await;
        log.recent(1).to_vec()
    };
    assert_eq!(recent.len(), 1);
    assert_eq!(recent[0].tool_name, "Bash");
    assert_eq!(recent[0].reason, "hook policy denied");

    let mut events = Vec::new();
    while let Ok(event) = rx.try_recv() {
        events.push(event.inner);
    }
    assert!(events.iter().any(|event| matches!(
        event,
        AgentEvent::PermissionDenied { tool, reason }
            if tool == "Bash" && reason.as_deref() == Some("hook policy denied")
    )));
    assert!(events.iter().any(|event| matches!(
        event,
        AgentEvent::ToolCallComplete { name, result, .. }
            if name == "Bash" && result.is_error
    )));
}

#[tokio::test]
async fn preflight_rejects_hook_mutated_input_that_violates_tool_schema() {
    let mut tools = ToolRegistry::new();
    tools.register(Box::new(archon_tools::bash::BashTool::default()));
    let (tx, mut rx) = tokio::sync::mpsc::channel(AGENT_EVENT_CHANNEL_CAPACITY);
    let config = AgentConfig {
        permission_mode: Arc::new(Mutex::new("bypassPermissions".to_string())),
        ..AgentConfig::default()
    };
    let mut agent = Agent::new(
        Arc::new(MockLlmProvider),
        tools,
        config,
        tx,
        Arc::new(std::sync::RwLock::new(AgentRegistry::load(
            &std::env::temp_dir(),
        ))),
    );
    let hooks = Arc::new(crate::hooks::HookRegistry::new());
    hooks.register_callback(
        crate::hooks::HookEvent::PreToolUse,
        crate::hooks::HookCallbackEntry {
            name: "remove-required-command".to_string(),
            callback: Arc::new(|_| crate::hooks::HookResult {
                updated_input: Some(serde_json::json!({ "timeout": 1 })),
                ..Default::default()
            }),
            authority: crate::hooks::SourceAuthority::Policy,
            timeout_secs: 1,
        },
    );
    agent.set_hook_registry(hooks);
    let pending = [PendingToolCall {
        id: "tool-invalid-hook-input".to_string(),
        name: "Bash".to_string(),
        input_json: r#"{"command":"cargo test"}"#.to_string(),
    }];

    let allowed = agent.preflight_tools(&pending, AgentMode::Normal).await;

    assert!(allowed.is_empty());
    let tool_result = &agent.state.messages[0]["content"][0];
    assert_eq!(tool_result["tool_use_id"], "tool-invalid-hook-input");
    assert_eq!(tool_result["is_error"], true);
    assert!(
        tool_result["content"]
            .as_str()
            .unwrap_or_default()
            .contains("effective input failed schema validation")
    );
    let mut saw_error_without_summary = false;
    while let Ok(event) = rx.try_recv() {
        if matches!(
            event.inner,
            AgentEvent::ToolCallComplete {
                result,
                transcript_summary: None,
                ..
            } if result.is_error
        ) {
            saw_error_without_summary = true;
        }
    }
    assert!(saw_error_without_summary);
}

include!("permission_gate_observer_test.rs");

// The prompt text is built next door, in the preflight gate that raises it.
use crate::agent::tool_preflight_gates::{INTENT_EXCERPT_LIMIT, describe_tool_intent};

/// The prompt has to say what is being approved. "Tool 'Bash' wants to:
/// use Bash" asked the user to authorise a shell command without showing
/// them the command, which is a rubber stamp rather than a decision.
#[test]
fn a_permission_prompt_names_the_command_it_is_asking_about() {
    let described = describe_tool_intent("Bash", r#"{"command":"rm -rf /tmp/x"}"#);

    // The checker renders this as "Tool 'Bash' wants to: {described}", so
    // the phrasing has to complete that sentence.
    assert_eq!(described, "run `rm -rf /tmp/x`");
}

#[test]
fn a_permission_prompt_names_the_file_a_write_would_touch() {
    let described = describe_tool_intent("Write", r#"{"file_path":"/etc/hosts","content":"x"}"#);

    assert!(described.contains("/etc/hosts"), "{described}");
}

#[test]
fn an_unrecognised_tool_falls_back_to_its_name_rather_than_a_json_blob() {
    let described = describe_tool_intent("Mystery", r#"{"secret":"value"}"#);

    assert_eq!(described, "use Mystery");
}

#[test]
fn a_long_multiline_command_is_flattened_and_bounded() {
    let long = "a".repeat(INTENT_EXCERPT_LIMIT + 50);
    let input = serde_json::json!({ "command": format!("echo one\necho {long}") }).to_string();

    let described = describe_tool_intent("Bash", &input);

    assert!(!described.contains('\n'), "{described}");
    assert!(
        described.chars().count() < INTENT_EXCERPT_LIMIT + 40,
        "{described}"
    );
}

#[test]
fn unparseable_permission_input_still_produces_a_prompt() {
    let described = describe_tool_intent("Bash", "{not json");

    assert_eq!(described, "use Bash");
}

#[tokio::test]
async fn preflight_sandbox_check_uses_hook_mutated_input() {
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(archon_tools::file_write::WriteTool));
    let mut agent = agent_with_registry_and_sandbox(registry, Arc::new(DenyBlockedWriteSandbox));
    let hooks = Arc::new(crate::hooks::HookRegistry::new());
    let callback: crate::hooks::HookCallback = Arc::new(|_| crate::hooks::HookResult {
        updated_input: Some(serde_json::json!({
            "file_path": "/blocked",
            "content": "must not be dispatched"
        })),
        ..Default::default()
    });
    hooks.register_callback(
        crate::hooks::HookEvent::PreToolUse,
        crate::hooks::HookCallbackEntry {
            name: "rewrite-write-path".to_string(),
            callback,
            authority: crate::hooks::SourceAuthority::Policy,
            timeout_secs: 1,
        },
    );
    agent.set_hook_registry(hooks);
    let pending = [PendingToolCall {
        id: "tool-1".to_string(),
        name: "Write".to_string(),
        input_json: r#"{"file_path":"/allowed","content":"before hook"}"#.to_string(),
    }];

    let allowed = agent.preflight_tools(&pending, AgentMode::Normal).await;
    assert_eq!(allowed.len(), 1);
    let ctx = agent.build_tool_context(AgentMode::Normal, "mock").await;
    let results = agent.dispatch_allowed_tools(&allowed, &ctx).await;

    assert_eq!(results.len(), 1);
    assert!(results[0].is_error);
    assert!(
        results[0]
            .content
            .contains("sandbox blocked mutated write path")
    );
}
