use super::*;

#[test]
fn no_host_stamp_means_no_boundary() {
    let layout = layout();
    let worktree = layout.worktree.to_str().unwrap();
    let unstamped = WorkflowReadGuard::new(40, 20, false, false).with_declared_targets(
        DeclaredTargetScope::new(&["src/lib.rs".to_string()], Some(worktree))
            .in_isolated_worktree(true),
    );
    assert_eq!(unstamped.boundary_paths(), None);
    assert_eq!(
        unstamped.before_tool("Write", &json!({"file_path": path_str(&layout.data)})),
        None
    );
}
