//! Tests for tool calls from inside a workflow script (#189 Phase 4).

use super::*;

/// A rule matching every use of one tool. `"*"` is the wildcard the rule
/// matcher recognises; an empty pattern matches nothing.
fn rule(tool: &str) -> archon_permissions::rules::ToolRule {
    archon_permissions::rules::ToolRule {
        tool: tool.to_string(),
        pattern: "*".to_string(),
    }
}

fn envelope(name: &str, input: serde_json::Value) -> String {
    serde_json::json!({
        "id": format!("{name}#1"),
        "options": { "name": name, "input": input }
    })
    .to_string()
}

fn host() -> Arc<ScriptToolHost> {
    Arc::new(
        ScriptToolHost::new(std::env::temp_dir(), "script-tool-tests".to_string())
            .expect("the tool host builds from config"),
    )
}

fn budget() -> Arc<std::sync::Mutex<ToolCallBudget>> {
    Arc::new(std::sync::Mutex::new(ToolCallBudget::default()))
}

/// The point of the phase: a script reads a file without a model round-trip.
#[tokio::test]
async fn a_script_can_read_a_file_directly() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("hello.txt");
    std::fs::write(&path, "read-without-a-model").expect("write");

    let json = execute_run_tool(
        &host(),
        &budget(),
        &envelope(
            "Read",
            serde_json::json!({"file_path": path.to_string_lossy()}),
        ),
    )
    .await
    .expect("the call is answered");

    let response: serde_json::Value = serde_json::from_str(&json).expect("json");
    assert_eq!(response["is_error"], serde_json::json!(false), "{response}");
    assert!(
        response["content"]
            .as_str()
            .unwrap_or_default()
            .contains("read-without-a-model"),
        "{response}"
    );
}

/// The same, for the other half of the criterion.
#[tokio::test]
async fn a_script_can_grep_files_directly() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("a.txt"), "needle-in-here").expect("write");

    let json = execute_run_tool(
        &host(),
        &budget(),
        &envelope(
            "Grep",
            serde_json::json!({"pattern": "needle", "path": dir.path().to_string_lossy()}),
        ),
    )
    .await
    .expect("the call is answered");

    let response: serde_json::Value = serde_json::from_str(&json).expect("json");
    assert!(
        response["content"]
            .as_str()
            .unwrap_or_default()
            .contains("a.txt"),
        "{response}"
    );
}

/// A name nothing serves has to say so, not return an empty success that a
/// script would read as "nothing matched".
#[tokio::test]
async fn an_unknown_tool_comes_back_as_a_failed_result() {
    let json = execute_run_tool(
        &host(),
        &budget(),
        &envelope("NoSuchTool", serde_json::json!({})),
    )
    .await
    .expect("an unknown tool is the script's problem, not the run's");

    let response: serde_json::Value = serde_json::from_str(&json).expect("json");
    assert_eq!(response["is_error"], serde_json::json!(true), "{response}");
    assert!(
        response["content"]
            .as_str()
            .unwrap_or_default()
            .contains("NoSuchTool"),
        "{response}"
    );
}

#[tokio::test]
async fn a_call_without_a_tool_name_fails_the_run() {
    let payload = serde_json::json!({"id": "x", "options": {"input": {}}}).to_string();

    let error = execute_run_tool(&host(), &budget(), &payload)
        .await
        .expect_err("a nameless call is a malformed script, not a failed tool");

    assert!(
        format!("{error}").contains("without a tool name"),
        "{error}"
    );
}

#[tokio::test]
async fn an_unreadable_payload_fails_the_run() {
    let error = execute_run_tool(&host(), &budget(), "{ not json")
        .await
        .expect_err("malformed payload");

    assert!(format!("{error}").contains("unreadable"), "{error}");
}

/// A script is a loop with no model in it to get bored, so the call count is
/// bounded. Exceeding it is fatal — unlike a refused call, there is no state in
/// which continuing produces a smaller total.
#[test]
fn the_call_budget_stops_a_runaway_loop() {
    let mut budget = ToolCallBudget::default();
    for _ in 0..MAX_TOOL_CALLS {
        budget.admit(1).expect("within the cap");
    }

    let error = budget.admit(1).expect_err("the cap must hold");

    assert!(error.contains(&MAX_TOOL_CALLS.to_string()), "{error}");
    assert_eq!(
        budget.calls, MAX_TOOL_CALLS,
        "a refused call is not counted"
    );
}

/// The failure worth preventing is a thousand small reads, not one large one,
/// so the byte cap is on the sum.
#[test]
fn the_byte_budget_bounds_the_total_rather_than_each_call() {
    let mut budget = ToolCallBudget::default();
    budget.admit(MAX_TOTAL_BYTES - 10).expect("within the cap");
    budget.admit(10).expect("exactly at the cap is allowed");

    let error = budget.admit(1).expect_err("one byte past is not");

    assert!(error.contains("limit for one run"), "{error}");
}

#[test]
fn a_budget_starts_empty() {
    let budget = ToolCallBudget::default();
    assert_eq!(budget.calls, 0);
    assert_eq!(budget.bytes, 0);
}

/// The gate is the same one a model-issued call passes. This asserts the wiring
/// exists at all: a checker that allowed everything would make the phase a
/// permission-escalation path.
#[tokio::test]
async fn a_denied_tool_is_refused_with_the_reason() {
    let checker = PermissionChecker::new(
        archon_permissions::mode::PermissionMode::Default,
        archon_permissions::rules::RuleSet {
            always_allow: vec![],
            always_deny: vec![rule("Bash")],
            always_ask: vec![],
        },
    );
    let host = ScriptToolHost {
        registry: archon_core::dispatch::create_default_registry(std::env::temp_dir(), None),
        checker,
        working_dir: std::env::temp_dir(),
        session_id: "denied-tool-test".to_string(),
    };

    let refusal = host
        .run(&RunToolRequest {
            name: "Bash".to_string(),
            input: serde_json::json!({"command": "echo hi"}),
        })
        .await
        .expect_err("an always_deny rule must refuse");

    assert!(refusal.contains("Bash"), "{refusal}");
    assert!(refusal.contains("denied"), "{refusal}");
}

/// A script runs unattended. A decision meaning "confirm with the user" cannot
/// be answered, so it must refuse and say why rather than hang or allow.
#[tokio::test]
async fn a_tool_needing_confirmation_is_refused_because_nobody_can_answer() {
    let host = ScriptToolHost {
        registry: archon_core::dispatch::create_default_registry(std::env::temp_dir(), None),
        checker: PermissionChecker::new(
            archon_permissions::mode::PermissionMode::Default,
            archon_permissions::rules::RuleSet {
                always_allow: vec![],
                always_deny: vec![],
                always_ask: vec![rule("Write")],
            },
        ),
        working_dir: std::env::temp_dir(),
        session_id: "ask-tool-test".to_string(),
    };

    let refusal = host
        .run(&RunToolRequest {
            name: "Write".to_string(),
            input: serde_json::json!({"file_path": "x", "content": "y"}),
        })
        .await
        .expect_err("an always_ask rule cannot be answered by a script");

    assert!(refusal.contains("nobody"), "{refusal}");
    assert!(
        refusal.contains("always_allow"),
        "the message must say how to permit it: {refusal}"
    );
}
