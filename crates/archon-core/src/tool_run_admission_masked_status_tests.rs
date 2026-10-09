//! Issue-124 through the real dispatch path: a check whose status is echoed
//! (`...; echo "EXIT=$?"`) exits 0, and is still counted as the failure it
//! echoed; a report written into the run's artifact directory from an isolated
//! worktree does not lift the read wall.
use std::sync::Arc;

use archon_tools::tool::ToolContext;
use archon_tools::workflow_read_guard::{
    DeclaredTargetScope, IDENTICAL_FAILURES_BEFORE_REFUSAL, REPEATED_FAILURE_MARKER, RunStoreScope,
    WorkflowReadGuard,
};
use serde_json::json;

use crate::dispatch::ToolRegistry;

fn registry() -> ToolRegistry {
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(archon_tools::bash::BashTool::default()));
    registry.register(Box::new(archon_tools::file_write::WriteTool));
    registry.register(Box::new(archon_tools::glob_tool::GlobTool));
    registry
}

#[tokio::test]
async fn an_echoed_failing_status_is_counted_and_the_repeat_is_refused() {
    let temp = tempfile::tempdir().unwrap();
    let ctx = ToolContext {
        working_dir: temp.path().to_path_buf(),
        session_id: "masked-status-session".into(),
        workflow_read_guard: Some(Arc::new(WorkflowReadGuard::new(40, 20, false, false))),
        ..Default::default()
    };
    let registry = registry();
    let check = json!({"command": "test -f data/registry.json; echo \"EXIT=$?\""});
    for attempt in 0..IDENTICAL_FAILURES_BEFORE_REFUSAL {
        let result = registry.dispatch("Bash", check.clone(), &ctx).await;
        assert!(!result.is_error, "attempt {attempt}: {}", result.content);
        assert!(result.content.contains("EXIT=1"), "{}", result.content);
    }
    let refused = registry.dispatch("Bash", check, &ctx).await;
    assert!(refused.is_error, "{}", refused.content);
    assert!(
        refused
            .content
            .contains(archon_tools::tool::TOOL_REFUSAL_MARKER),
        "{}",
        refused.content
    );
    assert!(
        refused.content.contains(REPEATED_FAILURE_MARKER),
        "{}",
        refused.content
    );

    // An echoed 0 is a pass, and a pass is never refused.
    let passing = json!({"command": "true; echo \"EXIT=$?\""});
    for _ in 0..=IDENTICAL_FAILURES_BEFORE_REFUSAL {
        let result = registry.dispatch("Bash", passing.clone(), &ctx).await;
        assert!(
            !result.content.contains(REPEATED_FAILURE_MARKER),
            "{}",
            result.content
        );
    }
}

#[tokio::test]
async fn a_run_report_from_an_isolated_worktree_leaves_the_wall_up() {
    let store = tempfile::tempdir().unwrap();
    let run_root = store.path().join("wf-synthetic");
    let worktree = run_root.join("v2/worktrees/item-a/item-a-0");
    std::fs::create_dir_all(run_root.join("artifacts")).unwrap();
    std::fs::create_dir_all(&worktree).unwrap();
    let guard = WorkflowReadGuard::new(0, 20, false, false)
        .with_run_store(RunStoreScope::new(
            run_root.to_str(),
            store.path().to_str(),
            worktree.to_str(),
        ))
        .with_declared_targets(
            DeclaredTargetScope::new(&["lib.rs".to_string()], worktree.to_str())
                .in_isolated_worktree(true),
        );
    let ctx = ToolContext {
        working_dir: worktree.clone(),
        extra_dirs: vec![run_root.join("artifacts")],
        session_id: "run-report-session".into(),
        workflow_read_guard: Some(Arc::new(guard)),
        ..Default::default()
    };
    let registry = registry();
    let glob = || json!({"pattern": "*.rs"});
    assert!(registry.dispatch("Glob", glob(), &ctx).await.is_error);
    let report = run_root.join("artifacts/review-item-a.md");
    let wrote = registry
        .dispatch(
            "Write",
            json!({"file_path": report, "content": "blocked: fixture shape"}),
            &ctx,
        )
        .await;
    assert!(!wrote.is_error, "{}", wrote.content);
    assert!(registry.dispatch("Glob", glob(), &ctx).await.is_error);
    let source = worktree.join("lib.rs");
    let wrote = registry
        .dispatch(
            "Write",
            json!({"file_path": source, "content": "fn a() {}"}),
            &ctx,
        )
        .await;
    assert!(!wrote.is_error, "{}", wrote.content);
    assert!(!registry.dispatch("Glob", glob(), &ctx).await.is_error);
}
