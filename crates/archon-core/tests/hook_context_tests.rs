/// TASK-HOOK-029: HookContext enrichment tests
///
/// Tests cover:
/// - HookContext serializes to JSON with all required fields
/// - HookContextBuilder sets all fields correctly
/// - Default values: conversation_turn=0, agent_id=None, previous_tool=None, permission_mode="normal"
/// - Timestamp is valid ISO 8601 format
/// - HookContext::to_json() produces valid JSON
/// - tool_input and tool_output are optional (can be None)
/// - session_id and cwd are required (set in builder)
/// - Clone works correctly
/// - Builder with all fields set
use archon_core::hooks::{HookContext, HookContextBuilder, HookEvent, SourceAuthority};

// ---------------------------------------------------------------------------
// Builder basics
// ---------------------------------------------------------------------------

#[test]
fn builder_creates_context_with_defaults() {
    let ctx = HookContext::builder(HookEvent::PreToolUse)
        .session_id("sess-123".to_string())
        .cwd("/tmp".to_string())
        .build();

    assert!(matches!(ctx.hook_event, HookEvent::PreToolUse));
    assert_eq!(ctx.session_id, "sess-123");
    assert_eq!(ctx.cwd, "/tmp");
    assert_eq!(ctx.conversation_turn, 0);
    assert_eq!(ctx.permission_mode, "normal");
    assert!(ctx.agent_id.is_none());
    assert!(ctx.previous_tool.is_none());
    assert!(ctx.tool_name.is_none());
    assert!(ctx.tool_input.is_none());
    assert!(ctx.tool_output.is_none());
    assert!(ctx.source_authority.is_none());
}

#[test]
fn builder_sets_all_fields() {
    let input = serde_json::json!({"file": "test.rs"});
    let output = serde_json::json!({"status": "ok"});

    let ctx = HookContext::builder(HookEvent::PostToolUse)
        .session_id("sess-456".to_string())
        .cwd("/home/user".to_string())
        .tool_name("Bash".to_string())
        .tool_input(input.clone())
        .tool_output(output.clone())
        .agent_id("agent-1".to_string())
        .permission_mode("plan".to_string())
        .previous_tool("Read".to_string())
        .conversation_turn(5)
        .source_authority(SourceAuthority::Policy)
        .timestamp("2026-04-08T12:00:00Z".to_string())
        .build();

    assert!(matches!(ctx.hook_event, HookEvent::PostToolUse));
    assert_eq!(ctx.session_id, "sess-456");
    assert_eq!(ctx.cwd, "/home/user");
    assert_eq!(ctx.tool_name, Some("Bash".to_string()));
    assert_eq!(ctx.tool_input, Some(input));
    assert_eq!(ctx.tool_output, Some(output));
    assert_eq!(ctx.agent_id, Some("agent-1".to_string()));
    assert_eq!(ctx.permission_mode, "plan");
    assert_eq!(ctx.previous_tool, Some("Read".to_string()));
    assert_eq!(ctx.conversation_turn, 5);
    assert_eq!(ctx.source_authority, Some(SourceAuthority::Policy));
    assert_eq!(ctx.timestamp, "2026-04-08T12:00:00Z");
}

// ---------------------------------------------------------------------------
// Timestamp
// ---------------------------------------------------------------------------

#[test]
fn timestamp_is_valid_iso8601() {
    let ctx = HookContext::builder(HookEvent::SessionStart)
        .session_id("sess-ts".to_string())
        .cwd("/tmp".to_string())
        .build();

    // ISO 8601 contains 'T' separator and either 'Z' or '+'/'-' offset
    assert!(
        ctx.timestamp.contains('T'),
        "timestamp should contain 'T': {}",
        ctx.timestamp
    );
    let has_tz = ctx.timestamp.contains('Z')
        || ctx.timestamp.contains('+')
        || ctx.timestamp.rfind('-').is_some_and(|i| i > 10);
    assert!(has_tz, "timestamp should have timezone: {}", ctx.timestamp);
}

// ---------------------------------------------------------------------------
// Serialization
// ---------------------------------------------------------------------------

#[test]
fn to_json_produces_valid_json() {
    let ctx = HookContext::builder(HookEvent::PreToolUse)
        .session_id("sess-json".to_string())
        .cwd("/tmp".to_string())
        .tool_name("Write".to_string())
        .build();

    let json = ctx.to_json();
    assert!(json.is_object());
    let obj = json.as_object().unwrap();
    assert!(obj.contains_key("hook_event"));
    assert!(obj.contains_key("session_id"));
    assert!(obj.contains_key("cwd"));
    assert!(obj.contains_key("timestamp"));
    assert!(obj.contains_key("permission_mode"));
    assert!(obj.contains_key("conversation_turn"));
    assert!(obj.contains_key("tool_name"));
}

#[test]
fn serializes_to_json_string() {
    let ctx = HookContext::builder(HookEvent::Notification)
        .session_id("sess-ser".to_string())
        .cwd("/home".to_string())
        .build();

    let json_str = serde_json::to_string(&ctx).expect("should serialize");
    assert!(json_str.contains("\"session_id\":\"sess-ser\""));
    assert!(json_str.contains("\"cwd\":\"/home\""));
    assert!(json_str.contains("\"conversation_turn\":0"));
    assert!(json_str.contains("\"permission_mode\":\"normal\""));
}

#[test]
fn deserializes_from_json() {
    let ctx = HookContext::builder(HookEvent::PreToolUse)
        .session_id("sess-de".to_string())
        .cwd("/tmp".to_string())
        .tool_name("Bash".to_string())
        .conversation_turn(3)
        .build();

    let json_str = serde_json::to_string(&ctx).unwrap();
    let restored: HookContext = serde_json::from_str(&json_str).expect("should deserialize");

    assert_eq!(restored.session_id, "sess-de");
    assert_eq!(restored.cwd, "/tmp");
    assert_eq!(restored.tool_name, Some("Bash".to_string()));
    assert_eq!(restored.conversation_turn, 3);
}

// ---------------------------------------------------------------------------
// Optional fields
// ---------------------------------------------------------------------------

#[test]
fn tool_input_and_output_are_optional() {
    let ctx = HookContext::builder(HookEvent::UserPromptSubmit)
        .session_id("sess-opt".to_string())
        .cwd("/tmp".to_string())
        .build();

    assert!(ctx.tool_input.is_none());
    assert!(ctx.tool_output.is_none());

    let json = ctx.to_json();
    // Optional None fields may or may not appear depending on serde config
    // But to_json should still be valid
    assert!(json.is_object());
}

// ---------------------------------------------------------------------------
// Clone
// ---------------------------------------------------------------------------

#[test]
fn clone_produces_equal_context() {
    let ctx = HookContext::builder(HookEvent::PreToolUse)
        .session_id("sess-clone".to_string())
        .cwd("/tmp".to_string())
        .tool_name("Edit".to_string())
        .conversation_turn(7)
        .build();

    let cloned = ctx.clone();
    assert_eq!(cloned.session_id, ctx.session_id);
    assert_eq!(cloned.cwd, ctx.cwd);
    assert_eq!(cloned.tool_name, ctx.tool_name);
    assert_eq!(cloned.conversation_turn, ctx.conversation_turn);
    assert_eq!(cloned.timestamp, ctx.timestamp);
    assert_eq!(cloned.permission_mode, ctx.permission_mode);
}

// ---------------------------------------------------------------------------
// Session ID and CWD are required
// ---------------------------------------------------------------------------

#[test]
fn session_id_and_cwd_are_set_by_builder() {
    let ctx = HookContext::builder(HookEvent::SessionEnd)
        .session_id("required-sess".to_string())
        .cwd("/required/path".to_string())
        .build();

    assert_eq!(ctx.session_id, "required-sess");
    assert_eq!(ctx.cwd, "/required/path");
    // Verify they are non-empty after being set
    assert!(!ctx.session_id.is_empty());
    assert!(!ctx.cwd.is_empty());
}

// ---------------------------------------------------------------------------
// Builder returns HookContextBuilder type
// ---------------------------------------------------------------------------

#[test]
fn builder_returns_correct_type() {
    let _builder: HookContextBuilder = HookContext::builder(HookEvent::Stop);
    // Compiles = passes. HookContextBuilder is the correct type.
}
