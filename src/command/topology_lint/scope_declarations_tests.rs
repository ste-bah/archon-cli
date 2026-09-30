//! Batch O: an empty write set and prose forbidden entries are findings.

use super::*;

fn task(owns: &[&str], forbids: &[&str]) -> WorkflowV2TaskUniverseTask {
    WorkflowV2TaskUniverseTask {
        canonical_task_id: "TASK-X-001".into(),
        files_expected_to_change: owns.iter().map(|f| f.to_string()).collect(),
        files_forbidden_to_change: forbids.iter().map(|f| f.to_string()).collect(),
        ..Default::default()
    }
}

#[test]
fn a_task_that_declares_no_file_is_a_finding_naming_it() {
    let findings = inspect(&task(&[], &[]));
    assert_eq!(findings.len(), 1, "{findings:?}");
    assert!(
        findings[0].starts_with("task TASK-X-001:")
            && findings[0].contains("Files Expected to Change"),
        "{findings:?}"
    );
    let mut shared = task(&[], &[]);
    shared.shared_append_target_files = vec!["`src/registry.rs`".into()];
    assert!(inspect(&shared).is_empty());
}

#[test]
fn catch_all_forbidden_prose_is_a_finding_and_literal_entries_are_not() {
    let findings = inspect(&task(
        &["`src/a.rs` — exists (3 lines)"],
        &[
            "Everything else in the repository",
            "`docs/`",
            "crates/x/src/store.rs — owned by another task",
            ".mcp.json",
            "*.lock",
        ],
    ));
    assert_eq!(findings.len(), 1, "{findings:?}");
    assert!(
        findings[0].contains("Everything else in the repository"),
        "{findings:?}"
    );
}
