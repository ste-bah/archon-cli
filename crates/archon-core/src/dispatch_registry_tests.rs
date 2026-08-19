use super::*;
use archon_tools::tool::{AgentMode, WorkingTreeEffect};

#[test]
fn production_tool_effects_match_the_registry_contract() {
    let mut actual = effects_by_name(create_default_registry(std::env::temp_dir(), None));
    actual.sort_by(|left, right| left.0.cmp(&right.0));

    let mut expected = PRODUCTION_TOOL_EFFECTS
        .iter()
        .map(|(name, effect)| ((*name).to_owned(), *effect))
        .collect::<Vec<_>>();
    expected.sort_by(|left, right| left.0.cmp(&right.0));
    assert_eq!(
        actual, expected,
        "registry changed without a reviewed effect"
    );
}

#[test]
fn arbitrary_effects_are_explicitly_reviewed() {
    let actual = effects_by_name(create_default_registry(std::env::temp_dir(), None));
    let actual_arbitrary = actual
        .into_iter()
        .filter_map(|(name, effect)| (effect == WorkingTreeEffect::Arbitrary).then_some(name))
        .collect::<std::collections::BTreeSet<_>>();
    let reviewed_arbitrary = approved_arbitrary_tools()
        .iter()
        .map(|name| (*name).to_owned())
        .collect::<std::collections::BTreeSet<_>>();

    assert_eq!(
        actual_arbitrary, reviewed_arbitrary,
        "Arbitrary tools must be reviewed in both directions"
    );
}

fn effects_by_name(registry: ToolRegistry) -> Vec<(String, WorkingTreeEffect)> {
    registry
        .tool_names()
        .into_iter()
        .map(|name| {
            let effect = registry
                .get(name)
                .expect("registered production tool")
                .working_tree_effect();
            (name.to_owned(), effect)
        })
        .collect()
}

fn approved_arbitrary_tools() -> &'static [&'static str] {
    &[
        "Agent",
        "Bash",
        "BehaviourApprove",
        "BehaviourProposals",
        "BehaviourRollback",
        "CronCreate",
        "CronDelete",
        "DocAnswer",
        "DocGet",
        "DocIngest",
        "DocInspect",
        "DocList",
        "DocModelStatus",
        "DocProvenance",
        "DocSearch",
        "DocStatus",
        "EnterWorktree",
        "ExitWorktree",
        "GameTheoryCallSpecialist",
        "GameTheoryClassify",
        "GameTheoryInspect",
        "GameTheoryReplay",
        "GameTheoryRun",
        "GameTheorySpecimens",
        "GameTheoryStatus",
        "JavaToolchain",
        "LargeEditAbort",
        "LargeEditBegin",
        "LargeEditCommit",
        "LargeEditDeleteSection",
        "LargeEditInsertAfter",
        "LargeEditReplaceSection",
        "LearningInspect",
        "LearningStatus",
        "Monitor",
        "PowerShell",
        "Skill",
        "TaskCreate",
        "TeamCreate",
        "TeamDelete",
        // #189 Phase 6. `TerminalCreate` runs the user's shell startup files
        // and `TerminalWrite` runs whatever is typed into a live shell, so both
        // reach as far as `Bash` does. `TerminalRead` and `TerminalClose` only
        // touch the terminal registry and are not Arbitrary.
        "TerminalCreate",
        "TerminalWrite",
        "lsp",
    ]
}

#[test]
fn optional_leann_tools_are_read_only() {
    use std::sync::Arc;

    let database = cozo::DbInstance::new("mem", "", Default::default()).expect("in-memory Cozo");
    let config = archon_leann::indexer::EmbeddingConfig {
        provider: archon_leann::indexer::EmbeddingProviderKind::Mock,
        dimension: 8,
    };
    let index = Arc::new(
        archon_leann::CodeIndex::from_db(database, config).expect("LEANN index test fixture"),
    );
    let registry = create_default_registry(std::env::temp_dir(), Some(index));

    for name in ["LeannSearch", "LeannFindSimilar"] {
        assert_eq!(
            registry
                .get(name)
                .expect("optional LEANN tool registration")
                .working_tree_effect(),
            WorkingTreeEffect::None,
            "{name} must remain read-only"
        );
    }
}

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
    assert!(result.content.contains("not available in Plan Mode"));
    assert!(!tmp.path().join("escape.md").exists());
    assert!(!tmp.path().join(".archon/plan-audit").exists());
}
