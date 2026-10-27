use crate::file_read::ReadTool;
use crate::grep::GrepTool;
use crate::tool::{Tool, ToolContext, ToolResult};
use crate::workflow_read_guard::{WorkflowReadGuard, WorkflowReadGuardSettings};
use serde_json::{Value, json};
use std::sync::Arc;

fn context(root: &std::path::Path) -> ToolContext {
    ToolContext {
        working_dir: root.to_owned(),
        session_id: "AC-X-001".into(),
        read_result_replay_limit_bytes: crate::READ_REPLAY_CONTEXT_BYTES,
        workflow_read_guard: Some(Arc::new(WorkflowReadGuard::shell_only(
            &WorkflowReadGuardSettings {
                read_only_soft_call_ceiling: 0,
                read_only_hard_call_ceiling: 4,
                ..Default::default()
            },
        ))),
        ..Default::default()
    }
}

async fn execute(tool: &dyn Tool, input: Value, ctx: &ToolContext) -> ToolResult {
    let guard = ctx.workflow_read_guard.as_ref().unwrap();
    if let Some(reason) = guard.before_tool_for(ctx, tool.name(), &input) {
        return ToolResult::refusal(reason);
    }
    let result = tool.execute(input.clone(), ctx).await;
    guard.after_tool(tool.name(), &input, !result.is_error, "ok");
    result
}

fn inspections(ctx: &ToolContext) -> u32 {
    ctx.workflow_read_guard
        .as_ref()
        .unwrap()
        .read_only_inspections_for(ctx)
}

#[tokio::test]
async fn workflow_read_guard_cached_reads_leave_room_for_new_information() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("decision.data");
    std::fs::write(&path, "alpha\nbeta").unwrap();
    let ctx = context(dir.path());
    let input = json!({"file_path":path});
    assert_eq!(
        execute(&ReadTool, input.clone(), &ctx).await.content,
        "1\talpha\n2\tbeta"
    );
    for _ in 0..8 {
        let repeated = execute(&ReadTool, input.clone(), &ctx).await;
        assert!(
            repeated
                .content
                .contains("unchanged since your earlier Read call 1"),
            "{}",
            repeated.content
        );
        assert_eq!(
            inspections(&ctx),
            1,
            "cached content must not spend the ceiling"
        );
    }
    std::fs::write(&path, "changed").unwrap();
    assert_eq!(
        execute(&ReadTool, input.clone(), &ctx).await.content,
        "1\tchanged"
    );
    assert_eq!(inspections(&ctx), 2);
    assert_eq!(
        execute(
            &ReadTool,
            json!({"file_path":path,"force_refresh":true}),
            &ctx
        )
        .await
        .content,
        "1\tchanged"
    );
    assert_eq!(inspections(&ctx), 3);
    ctx.workflow_read_guard
        .as_ref()
        .unwrap()
        .clear_ranges_for(&ctx);
    assert_eq!(
        execute(&ReadTool, input.clone(), &ctx).await.content,
        "1\tchanged"
    );
    assert_eq!(inspections(&ctx), 4);
    assert!(execute(&ReadTool, input, &ctx).await.is_error);
}

#[tokio::test]
async fn workflow_read_guard_identical_searches_reuse_only_visible_current_results() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("decision.data");
    std::fs::write(&path, "alpha\nbeta").unwrap();
    let ctx = context(dir.path());
    let input = json!({"path":dir.path(),"pattern":"alpha","output_mode":"content"});
    let first = execute(&GrepTool, input.clone(), &ctx).await;
    assert!(first.content.contains(":1:alpha"));
    let repeated = execute(&GrepTool, input.clone(), &ctx).await;
    assert!(
        repeated
            .content
            .contains("unchanged since your earlier Grep call 1"),
        "{}",
        repeated.content
    );
    assert_eq!(inspections(&ctx), 1);
    std::fs::write(&path, "alpha changed").unwrap();
    assert!(
        execute(&GrepTool, input.clone(), &ctx)
            .await
            .content
            .contains(":1:alpha changed")
    );
    assert_eq!(inspections(&ctx), 2);
    let mut child = ctx.clone();
    child.subagent_id = Some("TASK-X-001".into());
    assert!(
        execute(&GrepTool, input.clone(), &child)
            .await
            .content
            .contains(":1:alpha changed")
    );
    assert_eq!(inspections(&child), 1);
    ctx.workflow_read_guard
        .as_ref()
        .unwrap()
        .clear_ranges_for(&ctx);
    assert!(
        execute(&GrepTool, input, &ctx)
            .await
            .content
            .contains(":1:alpha changed")
    );
    assert_eq!(inspections(&ctx), 3);
}

#[tokio::test]
async fn workflow_read_guard_search_flags_and_replay_budget_preserve_information() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("decision.data");
    std::fs::write(&path, "alpha\nALPHA").unwrap();
    let ctx = context(dir.path());
    let input = json!({"path":path,"pattern":"alpha","output_mode":"content"});
    let first = execute(&GrepTool, input.clone(), &ctx).await;
    assert!(
        execute(&GrepTool, input.clone(), &ctx)
            .await
            .content
            .contains("content omitted")
    );
    let changed_flags = execute(
        &GrepTool,
        json!({"path":path,"pattern":"alpha","output_mode":"content","case_insensitive":true}),
        &ctx,
    )
    .await;
    assert!(changed_flags.content.contains(":2:ALPHA"));
    assert_eq!(inspections(&ctx), 2);
    let same_bytes_with_filter = execute(
        &GrepTool,
        json!({"path":path,"pattern":"alpha","output_mode":"content","glob":"*.data"}),
        &ctx,
    )
    .await;
    assert_eq!(
        same_bytes_with_filter.content, first.content,
        "different flags must return their own result even when its bytes match"
    );
    assert_eq!(inspections(&ctx), 3);
    let mut narrow = ctx.clone();
    narrow.read_result_replay_limit_bytes = 1;
    assert_eq!(
        execute(&GrepTool, input, &narrow).await.content,
        first.content
    );
    assert_eq!(inspections(&ctx), 4);
}

#[tokio::test]
async fn workflow_read_guard_default_read_returns_whole_file_only_when_it_fits() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("decision.data");
    let body = (0..2002)
        .map(|i| format!("row-{i}"))
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(&path, body).unwrap();
    let ctx = context(dir.path());
    let input = json!({"file_path":path});
    let whole = execute(&ReadTool, input.clone(), &ctx).await;
    assert!(
        whole.content.contains("2002\trow-2001"),
        "a fitting file must be returned whole"
    );
    assert!(
        execute(&ReadTool, input.clone(), &ctx)
            .await
            .content
            .contains("content omitted")
    );
    assert!(
        execute(&ReadTool, json!({"file_path":path,"offset":0}), &ctx)
            .await
            .content
            .contains("content omitted")
    );
    assert_eq!(inspections(&ctx), 1);
    assert_eq!(
        execute(
            &ReadTool,
            json!({"file_path":path,"offset":2000,"limit":1}),
            &ctx
        )
        .await
        .content,
        "2001\trow-2000"
    );
    let mut narrow = ctx.clone();
    narrow.read_result_replay_limit_bytes = 1024;
    let page = execute(&ReadTool, input, &narrow).await;
    assert!(page.content.contains("2000\trow-1999"));
    assert!(!page.content.contains("2001\trow-2000"));
}
