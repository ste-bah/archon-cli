use archon_tools::{glob_tool::GlobTool, tool::{Tool, ToolContext}};
use serde_json::json;

#[tokio::test]
async fn glob_caps_matches_and_counts_omitted_on_both_paths() {
    let root = tempfile::tempdir().unwrap();
    for n in 0..237 {
        std::fs::write(root.path().join(format!("file-{n:03}.txt")), "x").unwrap();
    }
    for denied_directory_names in [vec![], vec![".archon".into()]] {
        let ctx = ToolContext {working_dir:root.path().into(), denied_directory_names, ..Default::default()};
        let result = GlobTool.execute(json!({"pattern":"*"}), &ctx).await;
        assert!(!result.is_error, "{}", result.content);
        assert_eq!(result.content.lines().filter(|line| line.ends_with(".txt")).count(), 200);
        assert!(result.content.contains("37 matches omitted"), "{}", result.content);
        assert!(result.content.contains("narrow"));
    }
}

#[tokio::test]
async fn glob_small_and_empty_results_remain_unchanged() {
    let root = tempfile::tempdir().unwrap();
    let ctx = ToolContext {working_dir:root.path().into(), ..Default::default()};
    assert_eq!(GlobTool.execute(json!({"pattern":"*"}), &ctx).await.content, "No files matched the pattern.");
    let path = root.path().join("one.txt");
    std::fs::write(&path, "x").unwrap();
    assert_eq!(GlobTool.execute(json!({"pattern":"*"}), &ctx).await.content, std::fs::canonicalize(path).unwrap().to_string_lossy());
}

#[tokio::test]
async fn glob_bounds_long_paths_without_splitting_paths_or_losing_omitted_count() {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("x".repeat(180));
    std::fs::create_dir(&dir).unwrap();
    for n in 0..200 { std::fs::write(dir.join(format!("{n:03}-{}.txt", "y".repeat(180))), "x").unwrap(); }
    let ctx = ToolContext {working_dir:root.path().into(), ..Default::default()};
    let result = GlobTool.execute(json!({"pattern":"**/*.txt"}), &ctx).await;
    assert!(!result.is_error);
    assert!(result.content.len() <= 32 * 1024 + 200, "{} bytes", result.content.len());
    let shown = result.content.lines().filter(|line| line.ends_with(".txt")).count();
    assert!(shown > 0 && shown < 200);
    assert!(result.content.contains(&format!("{} matches omitted", 200 - shown)));
}
