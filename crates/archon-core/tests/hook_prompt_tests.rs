/// TASK-HOOK-019: Prompt Hook Executor tests
///
/// Prompt hooks differ from Command hooks:
/// - Stdout is treated as plain text (NOT JSON-parsed)
/// - The captured text becomes `additional_context` in the HookResult
/// - Exit code 2 → Blocking (stderr as reason)
/// - Empty/whitespace stdout → no additional_context (None)
/// - Any other exit code → HookResult::allow() with additional_context from stdout
use archon_core::hooks::{HookCommandType, HookConfig, HookEvent, HookMatcher, HookRegistry};

// ---------------------------------------------------------------------------
// Helper: build a HookRegistry with a single Prompt-type hook
// ---------------------------------------------------------------------------

fn make_prompt_registry(event: HookEvent, cmd: &str) -> HookRegistry {
    let mut registry = HookRegistry::new();
    registry.register_matchers(
        event,
        vec![HookMatcher {
            matcher: None,
            hooks: vec![HookConfig {
                hook_type: HookCommandType::Prompt,
                command: cmd.to_string(),
                if_condition: None,
                timeout: Some(10),
                once: None,
                r#async: None,
                async_rewake: None,
                status_message: None,
                headers: Default::default(),
                allowed_env_vars: Default::default(),
                enabled: true,
            }],
        }],
        None,
    );
    registry
}

async fn fire_prompt_hook(
    registry: &HookRegistry,
    event: HookEvent,
) -> archon_core::hooks::AggregatedHookResult {
    let cwd = std::env::current_dir().unwrap_or_default();
    let input = serde_json::json!({
        "hook_event": event.to_string(),
        "tool_name": "Bash",
        "tool_input": {"command": "echo test"}
    });
    registry
        .execute_hooks(event, input, &cwd, "test-session")
        .await
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Prompt hook stdout is captured as additional_context (plain text, not JSON-parsed).
#[tokio::test(flavor = "multi_thread")]
async fn test_prompt_hook_stdout_as_context() {
    let registry = make_prompt_registry(
        HookEvent::PreToolUse,
        "echo 'You should be careful with this tool'",
    );
    let result = fire_prompt_hook(&registry, HookEvent::PreToolUse).await;

    assert!(!result.is_blocked());
    assert_eq!(
        result.additional_contexts,
        vec!["You should be careful with this tool"],
        "stdout should appear as additional_context plain text"
    );
}

/// Prompt hook with exit 0 and no stdout → no additional_context.
#[tokio::test(flavor = "multi_thread")]
async fn test_prompt_hook_empty_stdout_no_context() {
    let registry = make_prompt_registry(HookEvent::PreToolUse, "exit 0");
    let result = fire_prompt_hook(&registry, HookEvent::PreToolUse).await;

    assert!(!result.is_blocked());
    assert!(
        result.additional_contexts.is_empty(),
        "empty stdout should produce no additional_context"
    );
}

/// Prompt hook exit 2 → Blocking with stderr as reason.
#[tokio::test(flavor = "multi_thread")]
async fn test_prompt_hook_exit_2_blocks() {
    let registry = make_prompt_registry(
        HookEvent::PreToolUse,
        "echo 'some stdout' ; echo 'policy violation' >&2; exit 2",
    );
    let result = fire_prompt_hook(&registry, HookEvent::PreToolUse).await;

    assert!(result.is_blocked(), "exit 2 must block");
    let reason = result.block_reason().expect("should have block reason");
    assert!(
        reason.contains("policy violation"),
        "block reason should contain stderr, got: '{reason}'"
    );
    // When blocked, stdout should NOT appear as additional_context
    assert!(
        result.additional_contexts.is_empty(),
        "blocked prompt hook should not produce additional_context"
    );
}

/// Prompt hook with whitespace-only stdout → no additional_context.
#[tokio::test(flavor = "multi_thread")]
async fn test_prompt_hook_whitespace_only_no_context() {
    let registry = make_prompt_registry(HookEvent::PreToolUse, "printf '   \\n  \\n  '");
    let result = fire_prompt_hook(&registry, HookEvent::PreToolUse).await;

    assert!(!result.is_blocked());
    assert!(
        result.additional_contexts.is_empty(),
        "whitespace-only stdout should produce no additional_context"
    );
}

/// Prompt hook with multiline stdout → all lines captured as single context string.
#[tokio::test(flavor = "multi_thread")]
async fn test_prompt_hook_multiline_stdout() {
    let registry = make_prompt_registry(
        HookEvent::PreToolUse,
        "printf 'line one\\nline two\\nline three'",
    );
    let result = fire_prompt_hook(&registry, HookEvent::PreToolUse).await;

    assert!(!result.is_blocked());
    assert_eq!(
        result.additional_contexts.len(),
        1,
        "should be one context entry"
    );
    let ctx = &result.additional_contexts[0];
    assert!(ctx.contains("line one"), "should contain first line");
    assert!(ctx.contains("line two"), "should contain second line");
    assert!(ctx.contains("line three"), "should contain third line");
}

/// Prompt hook stdout that happens to be valid JSON must NOT be parsed as HookResult.
/// It should appear verbatim as additional_context plain text.
#[tokio::test(flavor = "multi_thread")]
async fn test_prompt_hook_not_json_parsed() {
    // This is valid HookResult JSON — a Command hook would parse it and extract fields.
    // A Prompt hook must treat it as plain text.
    let json_str = r#"{"outcome":"blocking","reason":"should not be parsed"}"#;
    let cmd = format!("printf '{json_str}'");
    let registry = make_prompt_registry(HookEvent::PreToolUse, &cmd);
    let result = fire_prompt_hook(&registry, HookEvent::PreToolUse).await;

    // Must NOT be blocked — the JSON is plain text, not interpreted
    assert!(
        !result.is_blocked(),
        "prompt hook must not parse stdout as JSON HookResult"
    );
    // The raw JSON string should appear as additional_context
    assert_eq!(
        result.additional_contexts.len(),
        1,
        "should have one additional_context entry"
    );
    assert_eq!(
        result.additional_contexts[0], json_str,
        "stdout JSON should appear verbatim as plain text context"
    );
}
