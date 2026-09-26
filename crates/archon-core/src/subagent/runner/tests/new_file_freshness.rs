//! Creation must work through the actual runner, not by bypassing it with Bash.
use super::*;

async fn write_through_runner(existing: bool) -> (tempfile::TempDir, Vec<LlmRequest>) {
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("new.txt");
    if existing {
        std::fs::write(&file, "original").unwrap();
    }
    let provider = Arc::new(MockProvider::new(vec![
        tool_use_response(
            "write-new",
            "Write",
            &serde_json::json!({
                "file_path":file,"content":"created by Write"
            })
            .to_string(),
        ),
        text_response("done"),
    ]));
    let mut config = AgentConfig::default();
    config.filesystem.read_before_edit = crate::config::ReadBeforeEdit::Block;
    let mut runner = make_runner_with_config(provider.clone(), 2, config);
    runner.tool_context.working_dir = temp.path().to_path_buf();
    runner.tool_context.session_id = uuid::Uuid::new_v4().to_string();
    runner.tool_context.workflow_read_guard = Some(Arc::new(
        archon_tools::workflow_read_guard::WorkflowReadGuard::new(0, 20, false, false),
    ));
    runner.run("Write the deliverable").await.unwrap();
    (temp, provider.requests())
}

#[tokio::test]
async fn freshness_allows_new_file_without_impossible_prior_read() {
    let (temp, requests) = write_through_runner(false).await;
    assert_eq!(
        std::fs::read_to_string(temp.path().join("new.txt")).unwrap(),
        "created by Write"
    );
    assert_eq!(
        requests[1].messages.last().unwrap()["content"][0]["is_error"],
        false
    );
}

#[tokio::test]
async fn freshness_still_refuses_unread_existing_file() {
    let (temp, requests) = write_through_runner(true).await;
    assert_eq!(
        std::fs::read_to_string(temp.path().join("new.txt")).unwrap(),
        "original"
    );
    assert_eq!(
        requests[1].messages.last().unwrap()["content"][0]["is_error"],
        true
    );
}

/// Issue-115: the live write-branch sequence through the real runner and
/// tools — reads to the wall, refused inspection, a refused Edit of an
/// undeclared source file, then a NEW test file inside a declared test
/// directory and an Edit of the declared parent test file. Both successful
/// writes reach the same guard the dispatch consults: they count as
/// substantive, lift the wall, and grant the fresh read budget, so the
/// shell calls after them are never thrash.
#[tokio::test]
async fn writes_to_a_new_file_in_a_declared_test_dir_earn_credit_through_the_runner() {
    use archon_tools::workflow_read_guard::{
        DeclaredTargetScope, MAX_NON_WRITING_CALLS_AFTER_WALL, WorkflowReadGuard,
    };
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    for dir in ["crates/t/src", "crates/t/tests/lane"] {
        std::fs::create_dir_all(root.join(dir)).unwrap();
    }
    let parent = root.join("crates/t/tests/lane.rs");
    let store = root.join("crates/t/src/store.rs");
    std::fs::write(&parent, "#[test]\nfn lane() {}\n").unwrap();
    std::fs::write(&store, "pub fn store() {}\n").unwrap();
    let scope = DeclaredTargetScope::new(
        &[
            "crates/t/tests/lane.rs".to_string(),
            "crates/t/tests/lane/".to_string(),
        ],
        Some(root.to_str().unwrap()),
    );
    let guard = Arc::new(WorkflowReadGuard::new(3, 20, false, false).with_declared_targets(scope));
    let read = |path: &std::path::Path| serde_json::json!({"file_path": path}).to_string();
    let bash = |command: &str| serde_json::json!({"command": command}).to_string();
    let mut turns = vec![
        tool_use_response("r1", "Read", &read(&parent)),
        tool_use_response("r2", "Read", &read(&store)),
        tool_use_response("r3", "Grep", &serde_json::json!({"pattern": "fn", "path": root}).to_string()),
        tool_use_response("wall", "Read", &read(&store)),
        tool_use_response("g", "Bash", &bash("grep -c \"\" crates/t/src/store.rs")),
        tool_use_response(
            "undeclared",
            "Edit",
            &serde_json::json!({"file_path": store, "old_string": "store()", "new_string": "store2()"})
                .to_string(),
        ),
        tool_use_response(
            "new",
            "Write",
            &serde_json::json!({
                "file_path": root.join("crates/t/tests/lane/twin_lane_contract.rs"),
                "content": "#[test]\nfn twin() { assert_eq!(1, 1); }\n",
            })
            .to_string(),
        ),
        tool_use_response(
            "register",
            "Edit",
            &serde_json::json!({
                "file_path": parent,
                "old_string": "fn lane() {}\n",
                "new_string": "fn lane() {}\n\n#[path = \"lane/twin_lane_contract.rs\"]\nmod twin_lane_contract;\n",
            })
            .to_string(),
        ),
    ];
    // Twice the thrash cutoff of non-writing shell calls: not one is
    // counted once the writes have lifted the wall.
    for n in 0..MAX_NON_WRITING_CALLS_AFTER_WALL * 2 {
        turns.push(tool_use_response(
            &format!("e{n}"),
            "Bash",
            &bash("echo uu"),
        ));
    }
    turns.push(text_response("done"));
    let provider = Arc::new(MockProvider::new(turns));
    let mut runner = make_runner(provider.clone(), 60);
    runner.tool_context.working_dir = root.to_path_buf();
    runner.tool_context.session_id = uuid::Uuid::new_v4().to_string();
    runner.tool_context.workflow_read_guard = Some(guard.clone());
    runner.run("Write the deliverable").await.unwrap();

    let results: Vec<(bool, String)> = provider
        .requests()
        .iter()
        .skip(1)
        .map(|request| {
            let block = &request.messages.last().unwrap()["content"][0];
            (
                block["is_error"].as_bool().unwrap_or(false),
                block["content"].to_string(),
            )
        })
        .collect();
    assert!(
        !results[0].0 && !results[1].0 && !results[2].0,
        "{results:?}"
    );
    assert!(
        results[3].0 && results[3].1.contains("read budget exhausted"),
        "{results:?}"
    );
    assert!(
        results[4].0 && results[4].1.contains("read budget exhausted"),
        "{results:?}"
    );
    assert!(
        results[5].0,
        "the undeclared source edit is refused: {results:?}"
    );
    assert!(
        !results[6].0 && results[6].1.contains("File created successfully"),
        "{results:?}"
    );
    assert!(
        !results[7].0 && results[7].1.contains("updated successfully"),
        "{results:?}"
    );
    for (n, (is_error, text)) in results[8..].iter().enumerate() {
        assert!(!is_error, "shell call {n} after the writes: {text}");
    }
    assert_eq!(guard.terminal_failure(), None);
    // The credit is the full post-write budget, and the guard says two
    // writes were counted when it is next exhausted.
    for _ in 0..20 {
        assert_eq!(
            guard.before_tool("Read", &serde_json::json!({"file_path": parent})),
            None
        );
    }
    let refusal = guard
        .before_tool("Read", &serde_json::json!({"file_path": parent}))
        .unwrap();
    assert!(refusal.contains("2 writes so far"), "{refusal}");
    assert!(
        std::fs::read_to_string(&parent)
            .unwrap()
            .contains("mod twin_lane_contract;")
    );
}
