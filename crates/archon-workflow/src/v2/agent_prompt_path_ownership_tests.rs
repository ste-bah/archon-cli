use serde_json::json;

use super::path_ownership_prompt_section;
use crate::v2::verification::path_ownership::PATH_OWNERSHIP_INPUT_KEY;

fn section() -> String {
    path_ownership_prompt_section(&json!({
        "item": {},
        PATH_OWNERSHIP_INPUT_KEY: {
            "own_declared": ["crates/engine/src/mine.rs"],
            "declared_elsewhere": [{"path": "docs/plan.md", "owner_task": "TASK-B"}],
        }
    }))
}

#[test]
fn without_a_stamp_there_is_no_section_and_so_no_exemption() {
    assert_eq!(path_ownership_prompt_section(&json!({"item": {}})), "");
}

#[test]
fn the_section_carries_both_lists_their_completeness_and_the_rule() {
    let text = section();
    assert!(text.starts_with("## Path Ownership\n"), "{text}");
    // Completeness is the whole point: it is what lets a path in neither
    // list be read as nobody's.
    assert!(
        text.contains("a repository path in NEITHER list is declared by no task at all"),
        "{text}"
    );
    assert!(
        text.contains("An entry naming a directory covers every file beneath it"),
        "{text}"
    );
    assert!(
        text.contains("- Declared by this task (its writable scope): crates/engine/src/mine.rs\n"),
        "{text}"
    );
    assert!(
        text.contains("- Declared by another task: docs/plan.md (owned by TASK-B)\n"),
        "{text}"
    );
}

#[test]
fn the_rule_is_fail_closed_and_never_an_escape_hatch() {
    let text = section();
    for required in [
        "is a residual gap",
        "NOT a reason to withhold acceptance from this task",
        "No branch can ever be dispatched to write a file no task owns",
        "Judge this task on what it owns",
        "The exemption is narrow and never an escape hatch",
        "it applies only when the defect lies wholly outside this task's declared writable \
         scope AND this task's own deliverables and declared tests pass",
        "still fails the task exactly as before",
        "is that task's to fix",
    ] {
        assert!(text.contains(required), "missing {required:?} in {text}");
    }
}

#[test]
fn a_task_declaring_nothing_is_told_so_rather_than_shown_an_empty_list() {
    let text = path_ownership_prompt_section(&json!({
        "item": {},
        PATH_OWNERSHIP_INPUT_KEY: {
            "own_declared": [],
            "declared_elsewhere": [{"path": "docs/plan.md", "owner_task": "TASK-B"}],
        }
    }));
    assert!(
        text.contains("none recorded; treat every path as outside its scope"),
        "{text}"
    );
}
