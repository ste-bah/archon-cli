use super::*;

use archon_pipeline::audit::PipelineEvent;
use archon_pipeline::audit::runtime::PipelineAuditRun;

struct PassLlm {
    prompts: Mutex<Vec<String>>,
}

#[async_trait::async_trait]
impl LlmClient for PassLlm {
    async fn send_message(
        &self,
        messages: Vec<serde_json::Value>,
        _system: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
        _model: &str,
    ) -> Result<LlmResponse> {
        self.prompts.lock().unwrap().push(
            messages[0]["content"]
                .as_str()
                .unwrap_or_default()
                .to_string(),
        );
        Ok(LlmResponse {
            content: "PASS".to_string(),
            tool_uses: Vec::new(),
            tokens_in: None,
            tokens_out: None,
            stop_reason: None,
        })
    }
}

#[tokio::test]
async fn pending_repair_attempt_mismatch_fails_before_llm() {
    let tmp = tempfile::tempdir().unwrap();
    let facade = ValidatorFacade {
        session_id: "repair-attempt-mismatch".into(),
        fail_prompt_attempt: None,
        panic_repair_prompt: false,
        fail_repair_prompt: false,
    };
    let interrupted = Arc::new(RepairLlm {
        call: AtomicUsize::new(0),
        prompts: Mutex::new(Vec::new()),
        requests: Mutex::new(Vec::new()),
        always_fail: false,
        refuse_repair: false,
        panic_at: Some(2),
        provider_fail_at: None,
        block_persist_at: None,
    });
    let first_facade = ValidatorFacade {
        session_id: facade.session_id.clone(),
        fail_prompt_attempt: None,
        panic_repair_prompt: false,
        fail_repair_prompt: false,
    };
    let first_root = tmp.path().to_path_buf();
    let first = tokio::spawn(async move {
        run_pipeline_audited(
            &first_facade,
            interrupted.as_ref(),
            "task",
            &first_root,
            None,
            None,
            None,
        )
        .await
    });
    assert!(matches!(first.await, Err(join_error) if join_error.is_panic()));

    let audit_path = PipelineBundleStore::new(tmp.path())
        .bundle_dir(&facade.session_id)
        .join("audit.log");
    let mut events = std::fs::read_to_string(&audit_path)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .collect::<Vec<_>>();
    let marker = events
        .iter_mut()
        .find(|event| event["type"] == "artifact_repair_pending")
        .unwrap();
    marker["validator_key"] = serde_json::json!("different-validator");
    std::fs::write(
        &audit_path,
        events
            .iter()
            .map(serde_json::Value::to_string)
            .collect::<Vec<_>>()
            .join("\n")
            + "\n",
    )
    .unwrap();

    let resumed_llm = ResumeRepairLlm {
        call: AtomicUsize::new(0),
        prompts: Mutex::new(Vec::new()),
    };
    let error = resume_pipeline_audited(
        &facade,
        &resumed_llm,
        &facade.session_id,
        tmp.path(),
        None,
        None,
        None,
    )
    .await
    .err()
    .expect("mismatched pending attempt must fail before LLM use");
    let message = format!("{error:#}");
    assert!(message.contains("pending repair validator mismatch for paper"));
    assert_eq!(resumed_llm.call.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn failed_repair_commit_append_leaves_orphan_ignored_on_resume() {
    let tmp = tempfile::tempdir().unwrap();
    let facade = ValidatorFacade {
        session_id: "repair-commit-append-failure".into(),
        fail_prompt_attempt: None,
        panic_repair_prompt: false,
        fail_repair_prompt: false,
    };
    let interrupted = Arc::new(RepairLlm {
        call: AtomicUsize::new(0),
        prompts: Mutex::new(Vec::new()),
        requests: Mutex::new(Vec::new()),
        always_fail: false,
        refuse_repair: false,
        panic_at: Some(2),
        provider_fail_at: None,
        block_persist_at: None,
    });
    let first_facade = ValidatorFacade {
        session_id: facade.session_id.clone(),
        fail_prompt_attempt: None,
        panic_repair_prompt: false,
        fail_repair_prompt: false,
    };
    let first_root = tmp.path().to_path_buf();
    let first = tokio::spawn(async move {
        run_pipeline_audited(
            &first_facade,
            interrupted.as_ref(),
            "task",
            &first_root,
            None,
            None,
            None,
        )
        .await
    });
    assert!(matches!(first.await, Err(join_error) if join_error.is_panic()));

    let audit = PipelineAuditRun::resume(tmp.path(), &facade.session_id).unwrap();
    let audit_path = PipelineBundleStore::new(tmp.path())
        .bundle_dir(&facade.session_id)
        .join("audit.log");
    let prior_log = std::fs::read(&audit_path).unwrap();
    std::fs::remove_file(&audit_path).unwrap();
    std::fs::create_dir(&audit_path).unwrap();
    let _append_error = audit
        .persist_artifact_repair(
            "paper",
            1,
            1,
            archon_pipeline::audit::store::sha256_hex(PAPER),
            archon_pipeline::audit::store::sha256_hex(REPAIRED),
            REPAIRED,
        )
        .unwrap_err();
    assert!(audit_path.is_dir());
    let orphan = PipelineBundleStore::new(tmp.path())
        .bundle_dir(&facade.session_id)
        .join("outputs/repairs/001-paper-repair-1.txt");
    assert_eq!(std::fs::read_to_string(orphan).unwrap(), REPAIRED);
    std::fs::remove_dir(&audit_path).unwrap();
    std::fs::write(&audit_path, prior_log).unwrap();
    audit
        .store()
        .append_event(
            &facade.session_id,
            PipelineEvent::ArtifactRepairRejected {
                agent_key: "paper".to_string(),
                attempt: 1,
                reason: "repair commit append failed".to_string(),
            },
        )
        .unwrap();

    let llm = PassLlm {
        prompts: Mutex::new(Vec::new()),
    };
    let result = resume_pipeline_audited(
        &facade,
        &llm,
        &facade.session_id,
        tmp.path(),
        None,
        None,
        None,
    )
    .await
    .expect("resume should continue with the original paper");
    let paper = result
        .agent_results
        .iter()
        .find(|(agent, _)| agent.key == "paper")
        .unwrap()
        .1
        .output
        .clone();
    assert_eq!(paper, PAPER);
    assert!(llm.prompts.lock().unwrap()[0].contains(PAPER));
    assert!(!llm.prompts.lock().unwrap()[0].contains(REPAIRED));
    let hydrated = PipelineAuditRun::resume(tmp.path(), &facade.session_id)
        .unwrap()
        .hydrate_results()
        .unwrap();
    assert_eq!(
        hydrated
            .iter()
            .find(|(agent, _)| agent.key == "paper")
            .unwrap()
            .1
            .output,
        PAPER
    );
    assert!(
        archon_pipeline::audit::verify_bundle(
            &PipelineBundleStore::new(tmp.path()),
            &facade.session_id,
            false,
        )
        .unwrap()
        .valid
    );
}
