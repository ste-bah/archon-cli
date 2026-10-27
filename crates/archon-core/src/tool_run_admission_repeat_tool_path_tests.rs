//! Read-path repeat behavior through the actual tool attempt dispatch.

use std::sync::Arc;

use archon_tools::tool::ToolContext;
use archon_tools::workflow_read_guard::{WorkflowReadGuard, WorkflowReadGuardSettings};

#[tokio::test]
async fn a_repeated_read_reaches_the_tool_through_attempt_dispatch() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("data.txt");
    std::fs::write(&path, "alpha\nbeta").unwrap();
    let guard = Arc::new(WorkflowReadGuard::shell_only(&WorkflowReadGuardSettings {
        read_only_hard_call_ceiling: 8,
        ..Default::default()
    }));
    let ctx = ToolContext {
        working_dir: dir.path().to_path_buf(),
        session_id: "AC-X-001".into(),
        workflow_read_guard: Some(guard),
        ..Default::default()
    };
    let tool = archon_tools::file_read::ReadTool;
    let input = serde_json::json!({"file_path": path});

    let first =
        crate::tool_run_admission::execute_tool_attempt(&tool, input.clone(), &ctx, false).await;
    let repeat = crate::tool_run_admission::execute_tool_attempt(&tool, input, &ctx, false).await;
    assert_eq!(first.content, "1\talpha\n2\tbeta");
    assert!(
        repeat
            .content
            .contains("unchanged since your earlier Read call 1")
    );
    assert!(repeat.content.contains(path.to_str().unwrap()));
    assert!(repeat.content.contains("force_refresh=true"));
}

#[tokio::test]
async fn a_repeated_read_names_the_requested_link_path() {
    let dir = tempfile::tempdir().unwrap();
    let real_dir = dir.path().join("real");
    let link_dir = dir.path().join("alias");
    std::fs::create_dir(&real_dir).unwrap();
    std::fs::write(real_dir.join("data.txt"), "alpha").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&real_dir, &link_dir).unwrap();
    #[cfg(windows)]
    std::os::windows::fs::symlink_dir(&real_dir, &link_dir).unwrap_or_else(|error| {
        panic!("creating a directory symlink failed (PermissionDenied may indicate a host privilege requirement): {error}")
    });
    let ctx = ToolContext {
        working_dir: dir.path().to_path_buf(),
        session_id: "AC-X-525-A".into(),
        workflow_read_guard: Some(Arc::new(WorkflowReadGuard::shell_only(
            &WorkflowReadGuardSettings {
                read_only_hard_call_ceiling: 8,
                ..Default::default()
            },
        ))),
        ..Default::default()
    };
    let tool = archon_tools::file_read::ReadTool;
    let requested = link_dir.join("data.txt").to_string_lossy().into_owned();
    let canonical = std::fs::canonicalize(real_dir.join("data.txt"))
        .unwrap()
        .to_string_lossy()
        .into_owned();
    crate::tool_run_admission::execute_tool_attempt(
        &tool,
        serde_json::json!({"file_path": requested}),
        &ctx,
        false,
    )
    .await;
    let repeat = crate::tool_run_admission::execute_tool_attempt(
        &tool,
        serde_json::json!({"file_path": requested}),
        &ctx,
        false,
    )
    .await;
    assert!(
        repeat.content.contains(&format!("{requested} offset=0")),
        "{}",
        repeat.content
    );
    assert!(!repeat.content.contains(&canonical), "{}", repeat.content);
}

#[tokio::test]
async fn alternate_spellings_share_one_range_and_note_the_current_spelling() {
    let dir = tempfile::tempdir().unwrap();
    let real_dir = dir.path().join("real");
    let link_dir = dir.path().join("alias");
    std::fs::create_dir(&real_dir).unwrap();
    std::fs::write(real_dir.join("data.txt"), "alpha").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&real_dir, &link_dir).unwrap();
    #[cfg(windows)]
    std::os::windows::fs::symlink_dir(&real_dir, &link_dir).unwrap_or_else(|error| {
        panic!("creating a directory symlink failed (PermissionDenied may indicate a host privilege requirement): {error}")
    });
    let ctx = ToolContext {
        working_dir: dir.path().to_path_buf(),
        session_id: "AC-X-525-B".into(),
        workflow_read_guard: Some(Arc::new(WorkflowReadGuard::shell_only(
            &WorkflowReadGuardSettings {
                read_only_hard_call_ceiling: 8,
                ..Default::default()
            },
        ))),
        ..Default::default()
    };
    let tool = archon_tools::file_read::ReadTool;
    let requested = link_dir.join("data.txt").to_string_lossy().into_owned();
    let current = real_dir.join("data.txt").to_string_lossy().into_owned();
    crate::tool_run_admission::execute_tool_attempt(
        &tool,
        serde_json::json!({"file_path": requested}),
        &ctx,
        false,
    )
    .await;
    let repeat = crate::tool_run_admission::execute_tool_attempt(
        &tool,
        serde_json::json!({"file_path": current}),
        &ctx,
        false,
    )
    .await;
    assert!(
        repeat
            .content
            .contains("unchanged since your earlier Read call 1"),
        "{}",
        repeat.content
    );
    assert!(
        repeat.content.contains(&format!("{current} offset=0")),
        "{}",
        repeat.content
    );
}

#[tokio::test]
async fn relative_read_repeat_names_the_requested_relative_path_and_force_refresh_returns_content()
{
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("data.txt"), "alpha\nbeta").unwrap();
    let ctx = ToolContext {
        working_dir: dir.path().to_path_buf(),
        session_id: "AC-X-525-C".into(),
        workflow_read_guard: Some(Arc::new(WorkflowReadGuard::shell_only(
            &WorkflowReadGuardSettings {
                read_only_hard_call_ceiling: 8,
                ..Default::default()
            },
        ))),
        ..Default::default()
    };
    let tool = archon_tools::file_read::ReadTool;
    let input = |force_refresh| {
        serde_json::json!({
            "file_path": "data.txt",
            "force_refresh": force_refresh,
        })
    };
    let first =
        crate::tool_run_admission::execute_tool_attempt(&tool, input(false), &ctx, false).await;
    let repeat =
        crate::tool_run_admission::execute_tool_attempt(&tool, input(false), &ctx, false).await;
    let refreshed =
        crate::tool_run_admission::execute_tool_attempt(&tool, input(true), &ctx, false).await;
    assert_eq!(first.content, "1\talpha\n2\tbeta");
    assert!(
        repeat.content.contains("data.txt offset=0"),
        "{}",
        repeat.content
    );
    assert_eq!(refreshed.content, "1\talpha\n2\tbeta");
}
