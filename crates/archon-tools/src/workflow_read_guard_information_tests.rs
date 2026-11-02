use crate::file_read::ReadTool;
use crate::grep::GrepTool;
use crate::tool::{Tool, ToolContext, ToolResult};
use crate::workflow_read_guard::{WorkflowReadGuard, WorkflowReadGuardSettings};
use serde_json::{Value, json};
use std::sync::Arc;

fn context(root: &std::path::Path) -> ToolContext {
    context_with(root, 4)
}

fn context_with(root: &std::path::Path, hard: u32) -> ToolContext {
    ToolContext {
        working_dir: root.to_owned(),
        session_id: "AC-X-001".into(),
        read_result_replay_limit_bytes: crate::READ_REPLAY_CONTEXT_BYTES,
        workflow_read_guard: Some(Arc::new(WorkflowReadGuard::shell_only(
            &WorkflowReadGuardSettings {
                read_only_soft_call_ceiling: 0,
                read_only_hard_call_ceiling: hard,
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

/// Issue-556 replaces `workflow_read_guard_cached_reads_leave_room_for_new_information`:
/// only the first cached repeat is free, so eight repeats now count seven.
#[tokio::test]
async fn workflow_read_guard_cached_reads_charge_after_one_free_repeat_and_leave_room_for_new_information()
 {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("decision.data");
    std::fs::write(&path, "alpha\nbeta").unwrap();
    let ctx = context_with(dir.path(), 11);
    let input = json!({"file_path":path});
    assert_eq!(
        execute(&ReadTool, input.clone(), &ctx).await.content,
        "1\talpha\n2\tbeta"
    );
    for repeat in 1..=8 {
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
            repeat,
            "only the first cached repeat is free"
        );
    }
    std::fs::write(&path, "changed").unwrap();
    assert_eq!(
        execute(&ReadTool, input.clone(), &ctx).await.content,
        "1\tchanged"
    );
    assert_eq!(inspections(&ctx), 9);
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
    assert_eq!(inspections(&ctx), 10);
    ctx.workflow_read_guard
        .as_ref()
        .unwrap()
        .clear_ranges_for(&ctx);
    assert_eq!(
        execute(&ReadTool, input.clone(), &ctx).await.content,
        "1\tchanged"
    );
    assert_eq!(inspections(&ctx), 11);
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

/// Issue-556 replaces `workflow_read_guard_default_read_returns_whole_file_only_when_it_fits`:
/// the second cached repeat of the whole-file range is charged.
#[tokio::test]
async fn workflow_read_guard_default_read_returns_whole_file_only_when_it_fits_and_charges_a_second_repeat()
 {
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
    assert_eq!(inspections(&ctx), 2);
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

/// Issue-556: a read-only call that cycles cached repeats over more files
/// than any repeat window holds still reaches its hard ceiling. Only the
/// first cached repeat of each range is free; every later one keeps the
/// short note and keeps its charge.
#[tokio::test]
async fn workflow_read_guard_cyclic_cached_reads_reach_the_hard_ceiling() {
    let dir = tempfile::tempdir().unwrap();
    let files = (0..10)
        .map(|i| {
            let path = dir.path().join(format!("context-{i}.json"));
            std::fs::write(&path, format!("{{\"n\":{i}}}")).unwrap();
            path
        })
        .collect::<Vec<_>>();
    let hard = 25;
    let ctx = context_with(dir.path(), hard);
    for path in &files {
        let first = execute(&ReadTool, json!({"file_path":path}), &ctx).await;
        assert!(first.content.starts_with("1\t{"), "{}", first.content);
    }
    assert_eq!(inspections(&ctx), 10);
    for path in &files {
        let repeated = execute(&ReadTool, json!({"file_path":path}), &ctx).await;
        assert!(
            repeated.content.contains("content omitted"),
            "{}",
            repeated.content
        );
    }
    assert_eq!(inspections(&ctx), 10, "the first repeat of a range is free");
    let mut calls = 2 * files.len();
    let refusal = loop {
        assert!(
            calls <= 2 * hard as usize,
            "a repeat cycle over {} files evaded the hard ceiling: {} inspections after {calls} calls",
            files.len(),
            inspections(&ctx)
        );
        let path = &files[calls % files.len()];
        let result = execute(&ReadTool, json!({"file_path":path}), &ctx).await;
        calls += 1;
        if result.is_error {
            break result.content;
        }
        assert!(
            result
                .content
                .contains("unchanged since your earlier Read call")
                && result.content.contains("content omitted"),
            "a later repeat keeps the short note: {}",
            result.content
        );
    };
    assert!(
        refusal.contains(crate::workflow_read_guard::READ_CEILING_MARKER),
        "{refusal}"
    );
    assert_eq!(inspections(&ctx), hard);
    assert_eq!(calls, 2 * files.len() + (hard as usize - files.len()) + 1);
}

/// Issue-556: one free repeat per charged read of a range. A force refresh
/// or a read after a history clear is charged and re-earns one free repeat,
/// so refunds never outnumber charged reads.
#[tokio::test]
async fn workflow_read_guard_one_free_repeat_per_charged_read() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("decision.data");
    std::fs::write(&path, "alpha").unwrap();
    let ctx = context_with(dir.path(), 10);
    let input = json!({"file_path":path});
    let note = |content: &str| content.contains("unchanged since your earlier Read call 1");
    assert_eq!(
        execute(&ReadTool, input.clone(), &ctx).await.content,
        "1\talpha"
    );
    assert_eq!(inspections(&ctx), 1);
    assert!(note(&execute(&ReadTool, input.clone(), &ctx).await.content));
    assert_eq!(inspections(&ctx), 1, "a single repeat stays free");
    let second = execute(&ReadTool, input.clone(), &ctx).await.content;
    assert!(note(&second), "{second}");
    assert_eq!(inspections(&ctx), 2, "a second repeat is charged");
    let refreshed = json!({"file_path":path,"force_refresh":true});
    assert_eq!(
        execute(&ReadTool, refreshed, &ctx).await.content,
        "1\talpha"
    );
    assert_eq!(inspections(&ctx), 3);
    let after_refresh = execute(&ReadTool, input.clone(), &ctx).await.content;
    assert!(
        after_refresh.contains("unchanged since your earlier Read call"),
        "{after_refresh}"
    );
    assert_eq!(
        inspections(&ctx),
        3,
        "a charged re-read earns one free repeat"
    );
    execute(&ReadTool, input.clone(), &ctx).await;
    assert_eq!(inspections(&ctx), 4);
    ctx.workflow_read_guard
        .as_ref()
        .unwrap()
        .clear_ranges_for(&ctx);
    assert_eq!(
        execute(&ReadTool, input.clone(), &ctx).await.content,
        "1\talpha",
        "a cleared cache returns full content and charges it"
    );
    assert_eq!(inspections(&ctx), 5);
    execute(&ReadTool, input.clone(), &ctx).await;
    assert_eq!(inspections(&ctx), 5);
    execute(&ReadTool, input, &ctx).await;
    assert_eq!(inspections(&ctx), 6);
}

/// Issue-556: the Grep twin of the read cycle.
#[tokio::test]
async fn workflow_read_guard_cyclic_cached_searches_reach_the_hard_ceiling() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("decision.data");
    let words = (0..9).map(|i| format!("word{i}")).collect::<Vec<_>>();
    std::fs::write(&path, words.join("\n")).unwrap();
    let hard = 20;
    let ctx = context_with(dir.path(), hard);
    let search =
        |i: usize| json!({"path":path,"pattern":words[i % words.len()],"output_mode":"content"});
    for i in 0..words.len() {
        let first = execute(&GrepTool, search(i), &ctx).await;
        assert!(first.content.contains(&words[i]), "{}", first.content);
    }
    for i in 0..words.len() {
        let repeated = execute(&GrepTool, search(i), &ctx).await;
        assert!(
            repeated
                .content
                .contains("search result unchanged since your earlier Grep call"),
            "{}",
            repeated.content
        );
    }
    assert_eq!(inspections(&ctx), 9, "the first repeat of a search is free");
    let mut calls = 2 * words.len();
    let refusal = loop {
        assert!(
            calls <= 2 * hard as usize,
            "a search cycle evaded the hard ceiling: {} inspections after {calls} calls",
            inspections(&ctx)
        );
        let result = execute(&GrepTool, search(calls), &ctx).await;
        calls += 1;
        if result.is_error {
            break result.content;
        }
        assert!(
            result
                .content
                .contains("search result unchanged since your earlier Grep call"),
            "a later repeat keeps the short note: {}",
            result.content
        );
    };
    assert!(
        refusal.contains(crate::workflow_read_guard::READ_CEILING_MARKER),
        "{refusal}"
    );
    assert_eq!(inspections(&ctx), hard);
}
