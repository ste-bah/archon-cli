//! #552 review: the saved outcome of a check the judge could not decide.

use super::*;

/// Gives no usable reply, counting provider calls.
struct UnusableJudge {
    calls: Arc<AtomicUsize>,
}

#[async_trait]
impl WorkflowLlmClient for UnusableJudge {
    async fn send_message_with_temperature(
        &self,
        _messages: Vec<serde_json::Value>,
        _system: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
        _model: &str,
        _temperature: f64,
    ) -> WorkflowResult<WorkflowAgentOutcome> {
        self.calls.fetch_add(1, SeqCst);
        Ok(WorkflowAgentOutcome {
            content: "no json".into(),
            stop_reason: Some("end_turn".into()),
            ..WorkflowAgentOutcome::default()
        })
    }

    async fn send_message(
        &self,
        _messages: Vec<serde_json::Value>,
        _system: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
        _model: &str,
    ) -> WorkflowResult<WorkflowAgentOutcome> {
        unreachable!("the judge samples explicitly")
    }
}

/// #552 review: the saved outcome of a check the judge could not decide is
/// reused only while the raw content of its named files is unchanged, as a
/// saved verdict is; a change the prompt redacts identically is still a miss.
#[tokio::test]
async fn an_undecided_outcome_is_reused_only_for_unchanged_named_file_content_552() {
    let project = tempfile::tempdir().unwrap();
    let source = project.path().join("settings.toml");
    std::fs::write(&source, "token = \"old-secret-value-0001\"\n").unwrap();
    let roots = super::super::judge::JudgeRoots {
        covered_texts: Default::default(),
        project: Some(project.path().to_path_buf()),
        repository: None,
        task_root: Some(project.path().to_path_buf()),
        repository_commit: None,
    };
    let calls = Arc::new(AtomicUsize::new(0));
    let client = UnusableJudge {
        calls: calls.clone(),
    };
    let expected = BTreeSet::from(["AC-X-001".to_string()]);
    let store = || JudgeStore::for_project(project.path(), Arc::default(), roots.clone());
    let check = super::contract("grep -q token settings.toml");
    store()
        .judge_subset(&client, check.clone(), &expected)
        .await
        .expect_err("no usable verdict");
    let first = calls.load(SeqCst);
    assert!(first > 0);
    store()
        .judge_subset(&client, check.clone(), &expected)
        .await
        .expect_err("the saved outcome");
    assert_eq!(calls.load(SeqCst), first, "unchanged content reuses it");
    std::fs::write(&source, "token = \"new-secret-value-0002\"\n").unwrap();
    store()
        .judge_subset(&client, check, &expected)
        .await
        .expect_err("no usable verdict");
    assert!(
        calls.load(SeqCst) > first,
        "changed named-file content is judged again"
    );
}
