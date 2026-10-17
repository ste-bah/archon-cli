use super::*;

#[tokio::test]
async fn refusal_rejects_repair_and_next_validator_judges_old_paper() {
    let tmp = tempfile::tempdir().unwrap();
    let facade = ValidatorFacade {
        session_id: "repair-refusal".into(),
        fail_prompt_attempt: None,
        panic_repair_prompt: false,
        fail_repair_prompt: false,
    };
    let llm = Arc::new(RepairLlm {
        call: AtomicUsize::new(0),
        prompts: Mutex::new(Vec::new()),
        requests: Mutex::new(Vec::new()),
        always_fail: false,
        refuse_repair: true,
        panic_at: None,
        provider_fail_at: None,
        block_persist_at: None,
    });
    let result = run_pipeline_audited(&facade, llm.as_ref(), "task", tmp.path(), None, None, None)
        .await
        .expect("validator accepts the unchanged paper on retry");
    assert_eq!(
        result
            .agent_results
            .iter()
            .find(|(a, _)| a.key == "paper")
            .unwrap()
            .1
            .output,
        PAPER
    );
    let judged = llm.prompts.lock().unwrap();
    assert!(judged[3].contains(PAPER));
    assert!(!judged[3].contains("I cannot help"));
    let bundle = PipelineBundleStore::new(tmp.path()).bundle_dir("repair-refusal");
    let log = std::fs::read_to_string(bundle.join("audit.log")).unwrap();
    assert!(log.contains("artifact_repair_rejected"));
    let events = log
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .collect::<Vec<_>>();
    let rejection_index = events
        .iter()
        .position(|event| event["type"] == "artifact_repair_rejected")
        .unwrap();
    assert!(
        events.iter().skip(rejection_index + 1).any(
            |event| event["type"] == "llm_attempt_started" && event["agent_key"] == "validator"
        )
    );
    assert!(
        !events
            .iter()
            .skip(rejection_index + 1)
            .any(|event| event["type"] == "artifact_repair_pending")
    );
    let paper_files = std::fs::read_dir(bundle.join("outputs/attempts"))
        .unwrap()
        .filter_map(Result::ok)
        .collect::<Vec<_>>();
    assert!(
        paper_files.iter().any(
            |entry| std::fs::read_to_string(entry.path()).is_ok_and(|content| content == PAPER)
        )
    );
    assert!(!paper_files.iter().any(|entry| {
        std::fs::read_to_string(entry.path()).is_ok_and(|content| content.contains("I cannot help"))
    }));
}

#[tokio::test]
async fn last_attempt_provider_failure_writes_draft_and_findings() {
    let tmp = tempfile::tempdir().unwrap();
    let facade = ValidatorFacade {
        session_id: "provider-failure-draft".into(),
        fail_prompt_attempt: None,
        panic_repair_prompt: false,
        fail_repair_prompt: false,
    };
    let llm = Arc::new(RepairLlm {
        call: AtomicUsize::new(0),
        prompts: Mutex::new(Vec::new()),
        requests: Mutex::new(Vec::new()),
        always_fail: true,
        refuse_repair: false,
        panic_at: None,
        provider_fail_at: Some(5),
        block_persist_at: None,
    });
    assert!(
        run_pipeline_audited(&facade, llm.as_ref(), "task", tmp.path(), None, None, None)
            .await
            .is_err()
    );
    let bundle = PipelineBundleStore::new(tmp.path()).bundle_dir("provider-failure-draft");
    let draft = std::fs::read_to_string(bundle.join("exports/draft-paper.md")).unwrap();
    let findings = std::fs::read_to_string(bundle.join("exports/draft-paper.findings.md")).unwrap();
    assert!(draft.contains(REPAIRED));
    assert!(findings.contains("provider failed: simulated provider failure"));
}

#[tokio::test]
async fn persistence_failure_keeps_old_paper_and_records_rejection() {
    let tmp = tempfile::tempdir().unwrap();
    let attempted_output = tmp
        .path()
        .join(".archon/pipelines/repair-persist-failure/outputs/repairs/001-paper-repair-1.txt");
    let facade = ValidatorFacade {
        session_id: "repair-persist-failure".into(),
        fail_prompt_attempt: None,
        panic_repair_prompt: false,
        fail_repair_prompt: false,
    };
    let llm = Arc::new(RepairLlm {
        call: AtomicUsize::new(0),
        prompts: Mutex::new(Vec::new()),
        requests: Mutex::new(Vec::new()),
        always_fail: false,
        refuse_repair: false,
        panic_at: None,
        provider_fail_at: None,
        block_persist_at: Some((2, attempted_output.clone())),
    });
    let result = run_pipeline_audited(&facade, llm.as_ref(), "task", tmp.path(), None, None, None)
        .await
        .expect("validator should accept the unchanged paper on retry");
    assert_eq!(
        result
            .agent_results
            .iter()
            .find(|(agent, _)| agent.key == "paper")
            .unwrap()
            .1
            .output,
        PAPER
    );
    let log = std::fs::read_to_string(
        PipelineBundleStore::new(tmp.path())
            .bundle_dir("repair-persist-failure")
            .join("audit.log"),
    )
    .unwrap();
    assert!(log.contains("artifact_repair_rejected"));
    assert!(attempted_output.is_dir());
}
