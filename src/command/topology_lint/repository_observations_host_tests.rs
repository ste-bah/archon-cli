use super::*;
use crate::command::topology_lint::repository_observations_stamp::stamp;

#[test]
fn host_stamps_all_declaration_sources_and_is_idempotent() {
    let (_temp, _project, tasks, tree) = grounded();
    let raw = body(
        "[{kind: source, artifact_path: src/existing.rs}]",
        "- `src/lib.rs` — adjust `report.rs`\n- `src/new.rs`\n- `src/widgets/`",
    )
    .replace(
        "required_tools: []",
        "required_tools: []\nshared_append_target_files: [src/shared.rs]",
    );
    let path = tasks.join("TASK-X-001.md");
    let stamped = stamp(&tasks, &path, &raw).unwrap();
    assert_eq!(stamp(&tasks, &path, &stamped).unwrap(), stamped);
    let json = stamped
        .rsplit_once("```json\n")
        .unwrap()
        .1
        .split_once("\n```")
        .unwrap()
        .0;
    let facts: serde_json::Value = serde_json::from_str(json).unwrap();
    assert_eq!(facts["base_commit"], tree.base_commit());
    assert_eq!(facts["checkout_revision"], tree.base_commit());
    assert_eq!(
        facts["paths"]["src/existing.rs"]["lines"], 3,
        "no trailing newline"
    );
    assert_eq!(facts["paths"]["src/lib.rs"]["lines"], 1);
    assert_eq!(facts["paths"]["src/new.rs"]["kind"], "absent");
    assert_eq!(facts["paths"]["src/shared.rs"]["kind"], "absent");
    assert_eq!(facts["paths"]["src/widgets"]["kind"], "directory");
    assert!(facts["paths"].get("report.rs").is_none());
}

#[test]
fn generated_facts_do_not_hide_author_contradictions() {
    let (_temp, project, tasks, tree) = grounded();
    let raw = body("[]", "- `src/existing.rs` — exists (3 lines)");
    for claim in [
        "`src/existing.rs` — exists (2 lines)",
        "`src/existing.rs` — absent",
    ] {
        let raw = format!("{raw}\n## Notes\n\n{claim}\n");
        let stamped = stamp(&tasks, &tasks.join("TASK-X-001.md"), &raw).unwrap();
        assert_eq!(findings(&tasks, &project, &tree, &stamped).len(), 1);
    }
}

#[test]
fn stamp_refreshes_facts_after_checkout_changes() {
    let (_temp, _project, tasks, tree) = grounded();
    let raw = body("[]", "- `src/new.rs`");
    let path = tasks.join("TASK-X-001.md");
    let first = stamp(&tasks, &path, &raw).unwrap();
    std::fs::write(tree.root().join("src/new.rs"), "a\nb").unwrap();
    let second = stamp(&tasks, &path, &first).unwrap();
    assert!(second.contains("\"lines\": 2"));
    assert_ne!(first, second);
    assert_eq!(stamp(&tasks, &path, &second).unwrap(), second);
}

#[cfg(unix)]
#[test]
fn unreadable_path_is_an_operational_error_in_stamp_and_lint() {
    let (_temp, project, tasks, tree) = grounded();
    // A symlink loop fails metadata even for privileged test runners.
    std::os::unix::fs::symlink("unreadable.rs", tree.root().join("unreadable.rs")).unwrap();
    let raw = body("[]", "- `unreadable.rs`");
    let path = tasks.join("TASK-X-001.md");
    let error = stamp(&tasks, &path, &raw).unwrap_err();
    assert!(
        error.to_string().contains("repository observation failed"),
        "{error:#}"
    );
    assert!(error.to_string().contains("unreadable.rs"), "{error:#}");
    let error = crate::command::topology_lint::repository_claims::inspect(
        &project,
        &tasks,
        "TASK-X-001",
        &path,
        &raw,
    )
    .unwrap_err();
    assert!(error.to_string().contains("unreadable.rs"));
}
