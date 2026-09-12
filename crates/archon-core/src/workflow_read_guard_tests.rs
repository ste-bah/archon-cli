use super::*;
use archon_tools::workflow_read_guard::WorkflowReadGuard;
use serde_json::json;
use std::sync::Arc;

fn fixture(limit: u32) -> (tempfile::TempDir, ToolRegistry, ToolContext) {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("a.rs"), "fn old() {}\nsecond\n").unwrap();
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(archon_tools::file_read::ReadTool));
    registry.register(Box::new(archon_tools::file_write::WriteTool));
    registry.register(Box::new(archon_tools::file_edit::EditTool));
    registry.register(Box::new(archon_tools::glob_tool::GlobTool));
    let ctx = ToolContext {
        working_dir: temp.path().to_path_buf(),
        workflow_read_guard: Some(Arc::new(WorkflowReadGuard::new(limit, false))),
        ..Default::default()
    };
    (temp, registry, ctx)
}

#[tokio::test]
async fn workflow_read_guard_dispatch_blocks_after_budget_until_real_write() {
    let (temp, registry, ctx) = fixture(1);
    let path = temp.path().join("a.rs");
    assert!(
        !registry
            .dispatch("Read", json!({"file_path": path}), &ctx)
            .await
            .is_error
    );
    let denied = registry
        .dispatch("Glob", json!({"pattern": "*.rs"}), &ctx)
        .await;
    assert!(denied.is_error && denied.content.contains("read budget exhausted"));
    for content in ["fn old() {}\nsecond\n", " fn old() {}\nsecond\n "] {
        assert!(
            !registry
                .dispatch(
                    "Write",
                    json!({"file_path": path, "content": content}),
                    &ctx
                )
                .await
                .is_error
        );
        assert!(
            registry
                .dispatch("Glob", json!({"pattern": "*.rs"}), &ctx)
                .await
                .is_error
        );
    }
    assert!(
        registry
            .dispatch(
                "Edit",
                json!({"file_path": path, "old_string": "missing", "new_string": "new"}),
                &ctx
            )
            .await
            .is_error
    );
    assert!(
        registry
            .dispatch("Read", json!({"file_path": path}), &ctx)
            .await
            .is_error
    );
    assert!(
        !registry
            .dispatch(
                "Edit",
                json!({"file_path": path, "old_string": "old", "new_string": "new"}),
                &ctx
            )
            .await
            .is_error
    );
    assert!(
        !registry
            .dispatch("Glob", json!({"pattern": "*.rs"}), &ctx)
            .await
            .is_error
    );
}

#[tokio::test]
async fn workflow_read_guard_dedup_is_range_and_hash_aware_with_force_refresh() {
    let (temp, registry, ctx) = fixture(10);
    let path = temp.path().join("a.rs");
    let input = json!({"file_path": path, "offset": 0, "limit": 1});
    let first = registry.dispatch("Read", input.clone(), &ctx).await;
    assert!(first.content.contains("fn old()"));
    let repeat = registry.dispatch("Read", input.clone(), &ctx).await;
    assert!(
        repeat.content.contains("unchanged since your read at call")
            && !repeat.content.contains("fn old()")
    );
    let next = registry
        .dispatch(
            "Read",
            json!({"file_path": path, "offset": 1, "limit": 1}),
            &ctx,
        )
        .await;
    assert!(next.content.contains("second"));
    let refreshed = registry
        .dispatch(
            "Read",
            json!({"file_path": path, "offset": 0, "limit": 1, "force_refresh": true}),
            &ctx,
        )
        .await;
    assert!(refreshed.content.contains("fn old()"));
    std::fs::write(&path, "fn changed() {}\nsecond\n").unwrap();
    assert!(
        registry
            .dispatch("Read", input, &ctx)
            .await
            .content
            .contains("changed")
    );
}

#[tokio::test]
async fn workflow_read_guard_absent_leaves_operator_reads_unchanged() {
    let (temp, registry, mut ctx) = fixture(0);
    ctx.workflow_read_guard = None;
    for _ in 0..3 {
        let result = registry
            .dispatch("Read", json!({"file_path": temp.path().join("a.rs")}), &ctx)
            .await;
        assert!(!result.is_error && result.content.contains("fn old()"));
    }
}

// A harmless sentinel named Bash proves the dispatch guard runs BEFORE execution;
// this test never invokes cargo or any subprocess.
struct ShellSentinel;
#[async_trait::async_trait]
impl Tool for ShellSentinel {
    fn name(&self) -> &str {
        "Bash"
    }
    fn capability(&self) -> archon_tools::tool::ToolCapability {
        archon_tools::tool::ToolCapability::HostLocal
    }
    fn description(&self) -> &str {
        "sentinel"
    }
    fn input_schema(&self) -> serde_json::Value {
        json!({})
    }
    fn permission_level(&self, _: &serde_json::Value) -> archon_tools::tool::PermissionLevel {
        archon_tools::tool::PermissionLevel::Safe
    }
    async fn execute(&self, _: serde_json::Value, _: &ToolContext) -> ToolResult {
        ToolResult::success("executed")
    }
}

#[tokio::test]
async fn workflow_read_guard_shell_inspection_and_release_build_are_not_word_matches() {
    let (_, mut registry, mut ctx) = fixture(0);
    registry.register(Box::new(ShellSentinel));
    for cmd in [
        "cat file",
        "sed -n '1,20p' file",
        "cd src && head -20 lib.rs",
        "ls | head",
        "git diff --stat",
        "rg foo src",
        "cat src 2>/dev/null",
        "rg foo src || true",
        "git -C /repo diff",
        "cat src && echo \"$?\"",
    ] {
        assert!(
            registry
                .dispatch("Bash", json!({"command": cmd}), &ctx)
                .await
                .is_error,
            "{cmd}"
        );
    }
    for cmd in [
        "echo cat",
        "printf 'cargo build --release'",
        "sed -i 's/a/b/' file",
        "cat input > output",
        "cargo check -p demo",
        "echo ls && touch file",
    ] {
        assert_eq!(
            registry
                .dispatch("Bash", json!({"command": cmd}), &ctx)
                .await
                .content,
            "executed",
            "{cmd}"
        );
    }
    for cmd in [
        "cargo build --release",
        "cargo build --release && echo \"$?\"",
        "cd src && cargo build --release",
        "env FOO=1 cargo +stable build --release",
        "cargo build --profile release",
        "cargo build -r",
        "cargo build --release > build.log 2>&1",
    ] {
        assert!(
            registry
                .dispatch("Bash", json!({"command": cmd}), &ctx)
                .await
                .is_error,
            "{cmd}"
        );
    }
    ctx.workflow_read_guard = Some(Arc::new(WorkflowReadGuard::new(0, true)));
    assert_eq!(
        registry
            .dispatch("Bash", json!({"command": "cargo build --release"}), &ctx)
            .await
            .content,
        "executed"
    );
    ctx.workflow_read_guard = None;
    assert_eq!(
        registry
            .dispatch("Bash", json!({"command": "cargo build --release"}), &ctx)
            .await
            .content,
        "executed"
    );
}

#[tokio::test]
async fn workflow_read_guard_parallel_calls_share_one_budget_and_clone_retains_it() {
    let (_temp, registry, ctx) = fixture(1);
    let cloned = ctx.clone();
    let (a, b) = tokio::join!(
        registry.dispatch("Glob", json!({"pattern": "*.rs"}), &ctx),
        registry.dispatch("Glob", json!({"pattern": "*.rs"}), &cloned)
    );
    assert_ne!(a.is_error, b.is_error);
    assert!(
        registry
            .dispatch("Glob", json!({"pattern": "*.rs"}), &cloned)
            .await
            .is_error
    );
}

#[test]
fn workflow_read_guard_configuration_defaults_and_overrides_drive_budget() {
    let default: crate::config::GeneratedWorkflowConfig =
        serde_json::from_value(json!({})).unwrap();
    let guard = WorkflowReadGuard::new(
        default.max_reads_before_first_write,
        default.allow_release_builds,
    );
    for _ in 0..40 {
        assert!(guard.before_tool("Glob", &json!({})).is_none());
    }
    assert!(guard.before_tool("Glob", &json!({})).is_some());
    assert!(
        guard
            .before_tool("Bash", &json!({"command":"cargo build --release"}))
            .is_some()
    );
    let custom: crate::config::GeneratedWorkflowConfig = serde_json::from_value(
        json!({"max_reads_before_first_write":1,"allow_release_builds":true}),
    )
    .unwrap();
    let guard = WorkflowReadGuard::new(
        custom.max_reads_before_first_write,
        custom.allow_release_builds,
    );
    assert!(guard.before_tool("Glob", &json!({})).is_none());
    assert!(guard.before_tool("Glob", &json!({})).is_some());
    assert!(
        guard
            .before_tool("Bash", &json!({"command":"cargo build --release"}))
            .is_none()
    );
}

#[tokio::test]
async fn workflow_read_guard_persists_ranges_outside_workspace_and_reports_io_failure() {
    let (temp, registry, mut ctx) = fixture(10);
    let sidecar = temp.path().join("evidence/reads.jsonl");
    let guard = archon_tools::workflow_read_guard::scope_read_set(sidecar.clone(), async {
        Arc::new(WorkflowReadGuard::new(10, false))
    })
    .await;
    ctx.workflow_read_guard = Some(guard);
    let result = registry
        .dispatch(
            "Read",
            json!({"file_path":temp.path().join("a.rs"), "offset":1,"limit":1}),
            &ctx,
        )
        .await;
    assert!(!result.is_error, "{}", result.content);
    let record: serde_json::Value =
        serde_json::from_str(std::fs::read_to_string(&sidecar).unwrap().trim()).unwrap();
    assert_eq!(record["path"], "a.rs");
    assert_eq!(record["offset"], 1);
    assert_eq!(record["limit"], 1);
    let bad_sink = temp.path().join("a.rs/reads.jsonl");
    ctx.workflow_read_guard = Some(
        archon_tools::workflow_read_guard::scope_read_set(bad_sink, async {
            Arc::new(WorkflowReadGuard::new(10, false))
        })
        .await,
    );
    let result = registry
        .dispatch("Read", json!({"file_path":temp.path().join("a.rs")}), &ctx)
        .await;
    assert!(
        result.is_error && result.content.contains("read-set"),
        "{}",
        result.content
    );
}

#[test]
fn workflow_read_guard_assignment_chains_consume_budget() {
    for command in ["ROOT=/x; cd $ROOT; sed -n 1,5p f", "export ROOT=/x; cd $ROOT; cat f 2>/dev/null",
        "unset OLD; local ROOT=/x; grep pattern f", "X=1 cat f"] {
        let guard = WorkflowReadGuard::new(1, true);
        assert!(guard.before_tool("Bash", &json!({"command":command})).is_none());
        assert!(guard.before_tool("Bash", &json!({"command":command})).is_some(), "{command}");
    }
}
#[test]
fn workflow_read_guard_fallback_blocks_only_inspection_not_progress() {
    let guard = WorkflowReadGuard::new(2, true);
    for _ in 0..5 { assert!(guard.before_tool("Bash", &json!({"command":"cargo check"})).is_none()); }
    assert!(guard.before_tool("Bash", &json!({"command":"find . -type f"})).is_some());
    assert!(guard.before_tool("Read", &json!({"file_path":"f"})).is_some());
    for name in ["Write", "Edit", "ApplyPatch", "LargeEditBegin", "LargeEditCommit", "NotebookEdit"] {
        assert!(guard.before_tool(name, &json!({})).is_none(), "{name}");
    }
    for command in ["ROOT=/x; cd $ROOT; sed -n 1,5p f; cargo check", "cargo test", "cargo build --release",
        "python script.py", "tee f", "cat > f", "sed -i 's/old/new/' f", "npm test", "go test ./...", "git apply change.patch"] {
        assert!(guard.before_tool("Bash", &json!({"command":command})).is_none(), "{command}");
    }
}
