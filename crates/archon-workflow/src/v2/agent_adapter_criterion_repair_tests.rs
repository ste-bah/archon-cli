use super::*;

struct CriterionRepairClient {
    outputs: Mutex<Vec<String>>,
    prompts: Mutex<Vec<String>>,
}

#[async_trait::async_trait]
impl WorkflowV2AgentClient for CriterionRepairClient {
    async fn run_agent(&self, prompt: String) -> Result<String, WorkflowV2AgentError> {
        self.prompts.lock().unwrap().push(prompt);
        let mut outputs = self.outputs.lock().unwrap();
        (!outputs.is_empty())
            .then(|| outputs.remove(0))
            .ok_or_else(|| WorkflowV2AgentError::Transport("no scripted answer".into()))
    }

    async fn continue_agent_request(
        &self,
        _: &WorkflowV2AgentRequest,
        prompt: String,
    ) -> Result<String, WorkflowV2AgentError> {
        self.run_agent(prompt).await
    }
}

fn criterion_request(repo_root: &str) -> WorkflowV2AgentRequest {
    let mut request = write_request(repo_root);
    request.input = serde_json::json!({
        "item": {"canonical_task_ids": ["TASK-TDL-020"]},
        "task_universe": {"tasks": [{
            "canonical_task_id": "TASK-TDL-020",
            "acceptance_criteria": ["the native validation gate works"]
        }]}
    });
    request
}

fn criterion_output(repo_root: &str, criterion_results: Option<serde_json::Value>) -> String {
    let mut result = accepted_result_with_absolute_change(repo_root);
    if let Some(criterion_results) = criterion_results {
        result.data = serde_json::json!({"criterion_results": criterion_results});
    }
    serde_json::to_string(&result).unwrap()
}

#[tokio::test]
async fn malformed_criterion_envelopes_are_repaired_through_the_bounded_path() {
    let temp = tempfile::tempdir().unwrap();
    let repo = temp.path().join("repo");
    std::fs::create_dir_all(repo.join("crates/archon-trading/src")).unwrap();
    std::fs::write(repo.join("crates/archon-trading/src/data_lake.rs"), "// changed").unwrap();
    let root = repo.display().to_string();
    let repaired = criterion_output(
        &root,
        Some(serde_json::json!([{
            "task_id": "TASK-TDL-020", "criterion_index": 1,
            "status": "met", "evidence": "focused validation passed"
        }])),
    );
    let invalid_cases = [
        ("missing array", criterion_output(&root, None), "must be an array"),
        (
            "missing entry",
            criterion_output(&root, Some(serde_json::json!([]))),
            "missing an entry",
        ),
        (
            "wrong type",
            criterion_output(&root, Some(serde_json::json!({"bad": true}))),
            "must be an array",
        ),
    ];
    for (name, invalid, quoted_error) in invalid_cases {
        let client = CriterionRepairClient {
            outputs: Mutex::new(vec![invalid, repaired.clone()]),
            prompts: Mutex::new(Vec::new()),
        };
        let result = WorkflowV2AgentAdapter::new()
            .run_with_repair(&client, &criterion_request(&root))
            .await
            .unwrap_or_else(|err| panic!("{name}: {err}"));
        assert_eq!(result.status, WorkflowV2Status::Accepted, "{name}");
        assert!(client.prompts.lock().unwrap()[1].contains(quoted_error), "{name}");
        assert!(client.outputs.lock().unwrap().is_empty(), "{name}: no extra retry");
    }
}

#[tokio::test]
async fn read_only_request_with_claimed_criteria_is_left_untouched() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().display().to_string();
    let mut request = criterion_request(&root);
    request.call.write_mode = None;
    request.call.method = WorkflowV2HostMethod::Parallel;
    request.role = "reviewer".to_string();
    request.target_files.clear();

    let mut read_only_result = WorkflowV2Result::default();
    read_only_result.status = WorkflowV2Status::NeedsReview;
    read_only_result.summary = "criteria need more evidence".to_string();
    let original = read_only_result.clone();
    let client = CriterionRepairClient {
        outputs: Mutex::new(vec![serde_json::to_string(&read_only_result).unwrap()]),
        prompts: Mutex::new(Vec::new()),
    };

    let result = WorkflowV2AgentAdapter::new()
        .run_with_repair(&client, &request)
        .await
        .expect("read-only requests do not have to report criterion_results");

    assert_eq!(result, original);
    assert_eq!(client.prompts.lock().unwrap().len(), 1, "no repair call");
}

#[tokio::test]
async fn a_needs_review_result_with_malformed_criteria_is_repaired() {
    let temp = tempfile::tempdir().unwrap();
    let repo = temp.path().join("repo");
    std::fs::create_dir_all(repo.join("crates/archon-trading/src")).unwrap();
    std::fs::write(repo.join("crates/archon-trading/src/data_lake.rs"), "// changed").unwrap();
    let root = repo.display().to_string();
    let mut malformed: WorkflowV2Result =
        serde_json::from_str(&criterion_output(&root, None)).unwrap();
    malformed.status = WorkflowV2Status::NeedsReview;
    let client = CriterionRepairClient {
        outputs: Mutex::new(vec![
            serde_json::to_string(&malformed).unwrap(),
            criterion_output(
                &root,
                Some(serde_json::json!([{
                    "task_id": "TASK-TDL-020", "criterion_index": 1,
                    "status": "met", "evidence": "focused validation passed"
                }])),
            ),
        ]),
        prompts: Mutex::new(Vec::new()),
    };
    let result = WorkflowV2AgentAdapter::new()
        .run_with_repair(&client, &criterion_request(&root))
        .await
        .expect("malformed claimed criteria are repaired regardless of status");
    assert_eq!(result.status, WorkflowV2Status::Accepted);
    assert_eq!(client.prompts.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn a_well_formed_unmet_criterion_stays_needs_review_without_retry() {
    let temp = tempfile::tempdir().unwrap();
    let repo = temp.path().join("repo");
    std::fs::create_dir_all(repo.join("crates/archon-trading/src")).unwrap();
    std::fs::write(repo.join("crates/archon-trading/src/data_lake.rs"), "// changed").unwrap();
    let root = repo.display().to_string();
    let unmet = criterion_output(
        &root,
        Some(serde_json::json!([{
            "task_id": "TASK-TDL-020", "criterion_index": 1,
            "status": "unmet", "evidence": "the focused check failed"
        }])),
    );
    let client = CriterionRepairClient {
        outputs: Mutex::new(vec![unmet]),
        prompts: Mutex::new(Vec::new()),
    };
    let result = WorkflowV2AgentAdapter::new()
        .run_with_repair(&client, &criterion_request(&root))
        .await
        .expect("a well-shaped unmet answer is a result, not a repair error");
    assert_eq!(result.status, WorkflowV2Status::NeedsReview);
    assert_eq!(client.prompts.lock().unwrap().len(), 1, "no repair call");
}
