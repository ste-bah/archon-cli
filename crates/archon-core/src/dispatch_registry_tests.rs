use super::*;
use archon_tools::tool::AgentMode;

#[test]
fn clone_filtered_does_not_mutate_original() {
    let registry = create_default_registry(std::env::temp_dir(), None);
    let original_count = registry.tool_names().len();
    let _filtered = registry.clone_filtered(&["Read"]);
    assert_eq!(registry.tool_names().len(), original_count);
}

#[test]
fn clone_filtered_tool_definitions_match() {
    let registry = create_default_registry(std::env::temp_dir(), None);
    let filtered = registry.clone_filtered(&["Read", "Glob"]);
    let defs = filtered.tool_definitions();
    assert_eq!(defs.len(), 2);
    let def_names: Vec<&str> = defs.iter().map(|d| d["name"].as_str().unwrap()).collect();
    assert!(def_names.contains(&"Read"));
    assert!(def_names.contains(&"Glob"));
}

#[tokio::test]
async fn dispatch_blocked_in_plan_mode_writes_session_scoped_audit() {
    let tmp = tempfile::tempdir().unwrap();
    let registry = create_default_registry(tmp.path().to_path_buf(), None);
    let first = ToolContext {
        working_dir: tmp.path().to_path_buf(),
        session_id: "session-one".into(),
        mode: AgentMode::Plan,
        extra_dirs: vec![],
        ..Default::default()
    };
    let second = ToolContext {
        working_dir: tmp.path().to_path_buf(),
        session_id: "session-two".into(),
        mode: AgentMode::Plan,
        extra_dirs: vec![],
        ..Default::default()
    };

    let first_result = registry
        .dispatch(
            "Write",
            serde_json::json!({"file_path": "/tmp/one"}),
            &first,
        )
        .await;
    let second_result = registry
        .dispatch("Bash", serde_json::json!({"command": "true"}), &second)
        .await;

    assert!(first_result.is_error && second_result.is_error);
    let first_audit = crate::plan_file::plan_audit_path(tmp.path(), "session-one").unwrap();
    let second_audit = crate::plan_file::plan_audit_path(tmp.path(), "session-two").unwrap();
    assert!(
        std::fs::read_to_string(first_audit)
            .unwrap()
            .contains("Write (intercepted in Plan Mode)")
    );
    assert!(
        std::fs::read_to_string(second_audit)
            .unwrap()
            .contains("Bash (intercepted in Plan Mode)")
    );
}

#[tokio::test]
async fn dispatch_rejects_unsafe_session_id_without_writing_outside_audit_root() {
    let tmp = tempfile::tempdir().unwrap();
    let registry = create_default_registry(tmp.path().to_path_buf(), None);
    let context = ToolContext {
        working_dir: tmp.path().to_path_buf(),
        session_id: "../../escape".into(),
        mode: AgentMode::Plan,
        extra_dirs: vec![],
        ..Default::default()
    };

    let result = registry
        .dispatch(
            "Write",
            serde_json::json!({"file_path": "/tmp/one"}),
            &context,
        )
        .await;
    assert!(result.is_error);
    assert!(result.content.contains("not available in plan mode"));
    assert!(!tmp.path().join("escape.md").exists());
    assert!(!tmp.path().join(".archon/plan-audit").exists());
}
