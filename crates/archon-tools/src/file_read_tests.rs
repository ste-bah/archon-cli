use super::ReadTool;
use crate::tool::{Tool, ToolContext};
use crate::workflow_read_guard::{WorkflowReadGuard, WorkflowReadGuardSettings};
use serde_json::json;
use std::sync::Arc;

fn context(
    root: &std::path::Path,
    agent: Option<&str>,
    guard: Arc<WorkflowReadGuard>,
) -> ToolContext {
    ToolContext {
        working_dir: root.to_path_buf(),
        session_id: "AC-X-001".into(),
        read_result_replay_limit_bytes: 1_024,
        subagent_id: agent.map(str::to_owned),
        workflow_read_guard: Some(guard),
        ..Default::default()
    }
}

async fn read(ctx: &ToolContext, path: &std::path::Path, extra: serde_json::Value) -> String {
    let mut input = json!({"file_path": path});
    input
        .as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    let guard = ctx.workflow_read_guard.as_ref().unwrap();
    assert!(guard.before_tool_for(ctx, "Read", &input).is_none());
    let result = ReadTool.execute(input.clone(), ctx).await;
    guard.after_tool("Read", &input, !result.is_error, "ok");
    let mut content = result.content;
    if let Some(note) = guard.result_note_for(ctx, "Read", &input) {
        content.push_str("\n\n");
        content.push_str(&note);
    }
    content
}

#[tokio::test]
async fn default_read_returns_only_the_default_range_and_later_lines_remain_available() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("data.txt");
    let body = (0..2002)
        .map(|index| format!("row-{index}"))
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(&path, body).unwrap();
    let guard = Arc::new(WorkflowReadGuard::shell_only(&WorkflowReadGuardSettings {
        read_only_hard_call_ceiling: 20,
        ..Default::default()
    }));
    let ctx = context(dir.path(), None, guard);

    let first = read(&ctx, &path, json!({})).await;
    assert!(first.contains("2000\trow-1999"));
    assert!(!first.contains("2001\trow-2000"));
    let later = read(&ctx, &path, json!({"offset": 2000, "limit": 1})).await;
    assert_eq!(later, "2001\trow-2000");
    assert_eq!(
        ctx.workflow_read_guard
            .as_ref()
            .unwrap()
            .read_only_inspections_for(&ctx),
        2
    );
}

#[tokio::test]
async fn force_refresh_and_changed_bytes_return_content() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("data.txt");
    std::fs::write(&path, "alpha\nbeta").unwrap();
    let guard = Arc::new(WorkflowReadGuard::shell_only(&WorkflowReadGuardSettings {
        read_only_hard_call_ceiling: 20,
        ..Default::default()
    }));
    let ctx = context(dir.path(), None, guard);

    assert_eq!(read(&ctx, &path, json!({})).await, "1\talpha\n2\tbeta");
    assert_eq!(
        read(&ctx, &path, json!({"force_refresh": true})).await,
        "1\talpha\n2\tbeta"
    );
    std::fs::write(&path, "omega\nbeta").unwrap();
    assert_eq!(read(&ctx, &path, json!({})).await, "1\tomega\n2\tbeta");
}

#[tokio::test]
async fn a_child_agent_does_not_reuse_the_parent_agent_range() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("data.txt");
    std::fs::write(&path, "alpha\nbeta").unwrap();
    let guard = Arc::new(WorkflowReadGuard::shell_only(&WorkflowReadGuardSettings {
        read_only_hard_call_ceiling: 20,
        ..Default::default()
    }));
    let parent = context(dir.path(), None, guard.clone());
    let child = context(dir.path(), Some("TASK-X-001"), guard);

    assert_eq!(read(&parent, &path, json!({})).await, "1\talpha\n2\tbeta");
    assert_eq!(read(&child, &path, json!({})).await, "1\talpha\n2\tbeta");
}

#[tokio::test]
async fn write_capable_guard_keeps_parent_and_child_ranges_separate() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("data.txt");
    std::fs::write(&path, "alpha\nbeta").unwrap();
    let guard = Arc::new(WorkflowReadGuard::new(40, 20, false, false));
    let parent = context(dir.path(), None, guard.clone());
    let child = context(dir.path(), Some("TASK-X-001"), guard);

    assert_eq!(read(&parent, &path, json!({})).await, "1\talpha\n2\tbeta");
    assert_eq!(read(&child, &path, json!({})).await, "1\talpha\n2\tbeta");
}

#[tokio::test]
async fn oversized_read_replay_is_bounded_by_the_existing_read_budget() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("data.txt");
    let body = "x".repeat(70_000);
    std::fs::write(&path, &body).unwrap();
    let guard = Arc::new(WorkflowReadGuard::new(1, 20, false, false));
    let ctx = context(dir.path(), None, guard);

    let first = read(&ctx, &path, json!({"limit": 1})).await;
    assert!(first.len() > 64_000);
    let input = json!({"file_path": path, "limit": 1});
    let refusal = ctx
        .workflow_read_guard
        .as_ref()
        .unwrap()
        .before_tool_for(&ctx, "Read", &input)
        .unwrap();
    assert!(refusal.starts_with("read budget exhausted"), "{refusal}");
}

#[tokio::test]
async fn clearing_discarded_oversized_read_restores_content_and_budget_bounds_repeat() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("data.txt");
    let body = "x".repeat(70_000);
    std::fs::write(&path, &body).unwrap();
    let guard = Arc::new(WorkflowReadGuard::from_settings(
        &WorkflowReadGuardSettings {
            max_reads_before_first_write: 2,
            ..Default::default()
        },
    ));
    let ctx = context(dir.path(), Some("TASK-X-001"), guard.clone());

    let first = read(&ctx, &path, json!({"limit": 1})).await;
    assert!(first.contains(&body));
    guard.clear_ranges_for(&ctx);
    assert_eq!(read(&ctx, &path, json!({"limit": 1})).await, first);

    let input = json!({"file_path": path, "limit": 1});
    let refusal = guard.before_tool_for(&ctx, "Read", &input).unwrap();
    assert!(refusal.starts_with("read budget exhausted"), "{refusal}");
}

#[tokio::test]
async fn read_exceeding_the_agent_limit_is_replayed_in_full() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("data.txt");
    let body = (0..300)
        .map(|index| format!("row-{index}-{}", "x".repeat(30)))
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(&path, &body).unwrap();
    let guard = Arc::new(WorkflowReadGuard::new(40, 20, false, false));
    let ctx = context(dir.path(), None, guard);

    let first = read(&ctx, &path, json!({"limit": 400})).await;
    assert!(first.len() < crate::READ_REPLAY_CONTEXT_BYTES);
    assert!(first.len() > ctx.read_result_replay_limit_bytes);
    let second = read(&ctx, &path, json!({"limit": 400})).await;

    assert_eq!(second, first);
}

#[tokio::test]
async fn read_with_soft_ceiling_note_is_not_deduplicated_past_replay_limit() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("target.txt");
    let other = dir.path().join("other.txt");
    std::fs::write(&target, "x".repeat(990)).unwrap();
    std::fs::write(&other, "other").unwrap();
    let guard = Arc::new(WorkflowReadGuard::shell_only(&WorkflowReadGuardSettings {
        read_only_soft_call_ceiling: 2,
        read_only_hard_call_ceiling: 20,
        ..Default::default()
    }));
    let ctx = context(dir.path(), None, guard);

    let first = read(&ctx, &target, json!({})).await;
    assert!(first.len() < ctx.read_result_replay_limit_bytes);
    let _ = read(&ctx, &other, json!({})).await;
    let replay = read(&ctx, &target, json!({})).await;

    assert!(replay.starts_with("1\t"), "{replay}");
    assert!(replay.len() > ctx.read_result_replay_limit_bytes);
    assert!(replay.contains("produce your deliverable now"), "{replay}");
}

#[tokio::test]
async fn forced_refresh_returns_content_after_a_deduplicated_read() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("data.txt");
    std::fs::write(&path, "alpha\nbeta").unwrap();
    let guard = Arc::new(WorkflowReadGuard::new(40, 20, false, false));
    let ctx = context(dir.path(), None, guard);

    assert_eq!(read(&ctx, &path, json!({})).await, "1\talpha\n2\tbeta");
    let omitted = read(&ctx, &path, json!({})).await;
    assert!(omitted.contains("content omitted"));
    assert!(omitted.contains("may have been trimmed or compacted"));
    assert!(omitted.contains("force_refresh=true"));
    assert_eq!(
        read(&ctx, &path, json!({"force_refresh": true})).await,
        "1\talpha\n2\tbeta"
    );
}

#[tokio::test]
async fn clearing_discarded_agent_ranges_restores_full_content() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("data.txt");
    std::fs::write(&path, "alpha\nbeta").unwrap();
    let guard = Arc::new(WorkflowReadGuard::new(40, 20, false, false));
    let ctx = context(dir.path(), Some("AC-X-001"), guard.clone());

    assert_eq!(read(&ctx, &path, json!({})).await, "1\talpha\n2\tbeta");
    assert!(
        read(&ctx, &path, json!({}))
            .await
            .contains("content omitted")
    );
    guard.clear_ranges_for(&ctx);
    assert_eq!(read(&ctx, &path, json!({})).await, "1\talpha\n2\tbeta");
}

#[tokio::test]
async fn history_clear_forgets_all_ranges_for_its_agent() {
    let dir = tempfile::tempdir().unwrap();
    let large = dir.path().join("large.txt");
    let small = dir.path().join("small.txt");
    let body = "z".repeat(70_000);
    std::fs::write(&large, &body).unwrap();
    std::fs::write(&small, "alpha\nbeta").unwrap();
    let guard = Arc::new(WorkflowReadGuard::new(40, 20, false, false));
    let ctx = context(dir.path(), Some("TASK-X-001"), guard.clone());

    assert!(
        read(&ctx, &large, json!({"limit": 1}))
            .await
            .contains(&body)
    );
    assert_eq!(read(&ctx, &small, json!({})).await, "1\talpha\n2\tbeta");
    guard.clear_ranges_for(&ctx);

    let large_again = read(&ctx, &large, json!({"limit": 1})).await;
    let small_again = read(&ctx, &small, json!({})).await;
    assert!(large_again.contains(&body));
    assert_eq!(small_again, "1\talpha\n2\tbeta");
}

#[tokio::test]
async fn withdrawal_reason_wins_after_a_previous_read() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("data.txt");
    std::fs::write(&path, "alpha\nbeta").unwrap();
    let guard = Arc::new(WorkflowReadGuard::shell_only(&WorkflowReadGuardSettings {
        read_only_hard_call_ceiling: 20,
        ..Default::default()
    }));
    let ctx = context(dir.path(), None, guard.clone());

    assert_eq!(read(&ctx, &path, json!({})).await, "1\talpha\n2\tbeta");
    assert!(guard.withdraw_inspection_for(&ctx, "prior results repeated"));
    let input = json!({"file_path":path});
    let refusal = guard.before_tool_for(&ctx, "Read", &input).unwrap();
    assert!(refusal.starts_with("inspection withdrawn:"), "{refusal}");
}
