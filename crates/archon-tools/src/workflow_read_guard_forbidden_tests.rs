//! Issue-30: a mutating call at a forbidden path is refused; everything else
//! passes the guard exactly as before.
use super::{ForbiddenPathScope, scope_forbidden_paths};
use crate::workflow_read_guard::{WorkflowReadGuard, WorkflowReadGuardSettings};
use serde_json::json;

fn absolute(path: &str) -> String {
    if cfg!(windows) {
        format!("C:{path}")
    } else {
        path.to_string()
    }
}

fn patterns(entries: &[&str]) -> Vec<String> {
    entries.iter().map(|e| e.to_string()).collect()
}

fn guard(scope: ForbiddenPathScope) -> WorkflowReadGuard {
    WorkflowReadGuard::from_settings(&WorkflowReadGuardSettings::default())
        .with_forbidden_paths(scope)
}

#[test]
fn a_write_to_a_forbidden_path_is_refused_by_either_root_and_a_write_elsewhere_passes() {
    let temp = tempfile::tempdir().unwrap();
    let worktree = temp.path().join("iso/item");
    let canonical = temp.path().join("repo");
    std::fs::create_dir_all(&worktree).unwrap();
    std::fs::create_dir_all(&canonical).unwrap();
    let scope = ForbiddenPathScope::new(
        &patterns(&["src/gate.rs", "**/coverage.rs", "docs/"]),
        &[
            worktree.display().to_string(),
            canonical.display().to_string(),
        ],
    );
    let guard = guard(scope);
    let forbidden = worktree.join("src/gate.rs");
    let refusal = guard
        .before_tool(
            "Write",
            &json!({"file_path": forbidden.display().to_string(), "content": "x"}),
        )
        .expect("a forbidden write is refused");
    assert_eq!(
        refusal,
        format!(
            "Error: {} is forbidden by the task's Files Forbidden to Change list; leave it \
             unchanged and report the need as a residual gap.",
            forbidden.display()
        )
    );
    // The same file named from the canonical checkout, through the
    // canonicalised root, and a relative spelling.
    for (tool, key, path) in [
        (
            "Edit",
            "file_path",
            canonical.join("src/gate.rs").display().to_string(),
        ),
        (
            "ApplyPatch",
            "path",
            std::fs::canonicalize(&worktree)
                .unwrap()
                .join("crates/x/src/coverage.rs")
                .display()
                .to_string(),
        ),
        ("NotebookEdit", "path", "docs/nb.ipynb".to_string()),
        ("LargeEditBegin", "file_path", "./src/gate.rs".to_string()),
    ] {
        assert!(
            guard.before_tool(tool, &json!({key: path})).is_some(),
            "{tool} {path}"
        );
    }
    for (tool, key, path) in [
        (
            "Write",
            "file_path",
            worktree.join("src/owned.rs").display().to_string(),
        ),
        ("Edit", "file_path", "src/gate_tests.rs".to_string()),
        ("Read", "file_path", forbidden.display().to_string()),
        ("Grep", "path", forbidden.display().to_string()),
        ("Write", "file_path", "/elsewhere/src/gate.rs".to_string()),
    ] {
        assert!(
            guard.before_tool(tool, &json!({key: path})).is_none(),
            "{tool} {path}"
        );
    }
    // A staged large-edit mutation carries no path and is not judged here.
    assert!(
        guard
            .before_tool("LargeEditInsertAfter", &json!({"edit_id": "e1"}))
            .is_none()
    );
}

#[test]
fn a_read_only_guard_refuses_the_same_write_and_an_empty_scope_refuses_nothing() {
    let scope = ForbiddenPathScope::new(&patterns(&["src/gate.rs"]), &[absolute("/iso")]);
    let read_only = WorkflowReadGuard::shell_only(&WorkflowReadGuardSettings::default())
        .with_forbidden_paths(scope);
    assert!(
        read_only
            .before_tool("Write", &json!({"file_path": absolute("/iso/src/gate.rs")}))
            .is_some()
    );
    let none = guard(ForbiddenPathScope::new(&[], &[absolute("/iso")]));
    assert!(
        none.before_tool("Write", &json!({"file_path": absolute("/iso/src/gate.rs")}))
            .is_none()
    );
    let prose_only = guard(ForbiddenPathScope::new(&patterns(&["Frozen chain"]), &[]));
    assert!(
        prose_only
            .before_tool("Write", &json!({"file_path": "src/gate.rs"}))
            .is_none()
    );
}

/// The live path: the host scopes the list around the call, and the guard
/// built inside the scope carries it.
#[test]
fn a_guard_built_inside_the_scope_carries_the_list() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    let scope = ForbiddenPathScope::new(&patterns(&["src/gate.rs"]), &[absolute("/iso")]);
    let guard = runtime.block_on(scope_forbidden_paths(scope, async {
        WorkflowReadGuard::from_settings(&WorkflowReadGuardSettings::default())
    }));
    assert!(
        guard
            .before_tool("Edit", &json!({"file_path": absolute("/iso/src/gate.rs")}))
            .is_some()
    );
    let outside = WorkflowReadGuard::from_settings(&WorkflowReadGuardSettings::default());
    assert!(
        outside
            .before_tool("Edit", &json!({"file_path": absolute("/iso/src/gate.rs")}))
            .is_none()
    );
}
