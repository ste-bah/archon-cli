//! Real tool calls against stale source nested beneath an excluded directory.
use archon_tools::{file_read::ReadTool, glob_tool::GlobTool, grep::GrepTool};
use archon_tools::tool::{Tool, ToolContext};
use serde_json::json;

fn fixture() -> (tempfile::TempDir, ToolContext) {
    let root = tempfile::tempdir().unwrap();
    for (path, text) in [
        ("src/current.rs", "CURRENT needle"),
        (".archon/workflows/old/v2/worktrees/src/stale.rs", "STALE needle"),
        ("prds/input.md", "requirements"),
        ("tasks/input.md", "task contract"),
    ] {
        let path = root.path().join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    let ctx = ToolContext { working_dir: root.path().to_path_buf(), denied_directory_names: vec![".archon".into()], ..Default::default() };
    (root, ctx)
}

#[tokio::test]
async fn excluded_subtree_direct_read_grep_and_glob_are_refused() {
    let (_root, ctx) = fixture();
    let path = ".archon/workflows/old/v2/worktrees/src/stale.rs";
    let read = ReadTool.execute(json!({"file_path":path}), &ctx).await;
    let grep = GrepTool.execute(json!({"path":path,"pattern":"needle"}), &ctx).await;
    let glob = GlobTool.execute(json!({"path":".archon","pattern":"**/*.rs"}), &ctx).await;
    for result in [read, grep, glob] {
        assert!(result.is_error, "excluded subtree was admitted: {}", result.content);
        assert!(!result.content.contains("STALE needle"));
    }
}

#[tokio::test]
async fn excluded_subtree_recursive_search_does_not_return_stale_source() {
    let (_root, ctx) = fixture();
    let grep = GrepTool.execute(json!({"pattern":"needle","output_mode":"content"}), &ctx).await;
    assert!(!grep.is_error, "{}", grep.content);
    assert!(grep.content.contains("CURRENT"));
    assert!(!grep.content.contains("STALE"), "{}", grep.content);
    let glob = GlobTool.execute(json!({"pattern":"**/*.rs"}), &ctx).await;
    assert!(!glob.is_error, "{}", glob.content);
    assert!(glob.content.contains("current.rs"));
    assert!(!glob.content.contains("stale.rs"), "{}", glob.content);
}

#[cfg(unix)]
#[tokio::test]
async fn excluded_subtree_symlink_alias_cannot_expose_stale_source() {
    let (root, ctx) = fixture();
    std::os::unix::fs::symlink(root.path().join(".archon"), root.path().join("alias")).unwrap();
    let read = ReadTool.execute(json!({"file_path":"alias/workflows/old/v2/worktrees/src/stale.rs"}), &ctx).await;
    assert!(read.is_error, "{}", read.content);
    let grep = GrepTool.execute(json!({"pattern":"STALE","output_mode":"content"}), &ctx).await;
    assert!(!grep.content.contains("STALE needle"), "{}", grep.content);
    let glob = GlobTool.execute(json!({"pattern":"alias/**/*.rs"}), &ctx).await;
    assert!(!glob.content.contains("stale.rs"), "{}", glob.content);
}

#[tokio::test]
async fn excluded_subtree_policy_keeps_current_source_prd_and_tasks_readable() {
    let (_root, ctx) = fixture();
    for path in ["src/current.rs", "prds/input.md", "tasks/input.md"] {
        let result = ReadTool.execute(json!({"file_path":path}), &ctx).await;
        assert!(!result.is_error, "{}", result.content);
    }
}

#[tokio::test]
async fn ordinary_context_keeps_existing_access() {
    let (_root, mut ctx) = fixture(); ctx.denied_directory_names.clear();
    assert!(!ReadTool.execute(json!({"file_path":".archon/workflows/old/v2/worktrees/src/stale.rs"}), &ctx).await.is_error);
}
