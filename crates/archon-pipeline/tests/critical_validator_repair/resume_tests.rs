use super::*;

#[tokio::test]
async fn completed_validator_resume_supersedes_stale_draft_at_finalization() {
    let tmp = tempfile::tempdir().unwrap();
    let session_id = "completed-validator-stale-draft";
    let facade = ValidatorFacade {
        session_id: session_id.into(),
        fail_prompt_attempt: None,
        panic_repair_prompt: false,
        fail_repair_prompt: false,
    };
    let initial_llm = llm(false);
    run_pipeline_audited(
        &facade,
        initial_llm.as_ref(),
        "task",
        tmp.path(),
        None,
        None,
        None,
    )
    .await
    .expect("initial run should complete");

    let bundle = PipelineBundleStore::new(tmp.path()).bundle_dir(session_id);
    write_draft_research_artifacts(&bundle, "stale failed draft", "stale findings").unwrap();
    let resumed_llm = ResumeRepairLlm {
        call: AtomicUsize::new(0),
        prompts: Mutex::new(Vec::new()),
    };
    resume_pipeline_audited(
        &facade,
        &resumed_llm,
        session_id,
        tmp.path(),
        None,
        None,
        None,
    )
    .await
    .expect("resume with completed validator should finalize");

    assert_eq!(resumed_llm.call.load(Ordering::SeqCst), 0);
    assert!(!bundle.join("exports/draft-paper.md").exists());
    assert!(!bundle.join("exports/draft-paper.findings.md").exists());
    assert!(bundle.join("exports/superseded-draft-paper.md").exists());
    assert!(
        bundle
            .join("exports/superseded-draft-paper.findings.md")
            .exists()
    );
    assert!(bundle.join("exports/final-paper.md").exists());
}

#[tokio::test]
async fn interrupted_pending_repair_is_replayed_before_validator_resume() {
    let tmp = tempfile::tempdir().unwrap();
    let facade = ValidatorFacade {
        session_id: "repair-resume".into(),
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
        session_id: "repair-resume".into(),
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
    match first.await {
        Err(join_error) => assert!(join_error.is_panic()),
        Ok(_) => panic!("interrupted run unexpectedly completed"),
    }

    let resumed_llm = ResumeRepairLlm {
        call: AtomicUsize::new(0),
        prompts: Mutex::new(Vec::new()),
    };
    resume_pipeline_audited(
        &facade,
        &resumed_llm,
        "repair-resume",
        tmp.path(),
        None,
        None,
        None,
    )
    .await
    .expect("resume should replay repair and validate repaired paper");
    let prompts = resumed_llm.prompts.lock().unwrap();
    assert!(prompts[0].contains(PAPER));
    assert!(prompts[0].contains(FINDINGS));
    assert!(prompts[1].contains(REPAIRED));
}

#[tokio::test]
async fn failed_single_pending_append_does_not_advance_validator_progress() {
    let tmp = tempfile::tempdir().unwrap();
    let facade = ValidatorFacade {
        session_id: "repair-pending-append-failure".into(),
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
    let root = tmp.path().to_path_buf();
    let first = tokio::spawn(async move {
        run_pipeline_audited(
            &first_facade,
            interrupted.as_ref(),
            "task",
            &root,
            None,
            None,
            None,
        )
        .await
    });
    assert!(matches!(first.await, Err(error) if error.is_panic()));

    let bundle = PipelineBundleStore::new(tmp.path()).bundle_dir(&facade.session_id);
    let audit_path = bundle.join("audit.log");
    let original = std::fs::read_to_string(&audit_path).unwrap();
    let without_pending = original
        .lines()
        .filter(|line| !line.contains("artifact_repair_pending"))
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    std::fs::remove_file(&audit_path).unwrap();
    std::fs::create_dir(&audit_path).unwrap();
    let audit = archon_pipeline::audit::PipelineAuditRun::resume(tmp.path(), &facade.session_id);
    assert!(audit.is_err());
    std::fs::remove_dir(&audit_path).unwrap();
    std::fs::write(&audit_path, &without_pending).unwrap();

    let audit =
        archon_pipeline::audit::PipelineAuditRun::resume(tmp.path(), &facade.session_id).unwrap();
    assert_eq!(audit.last_validator_attempt("validator").unwrap(), 0);
    std::fs::remove_file(&audit_path).unwrap();
    std::fs::create_dir(&audit_path).unwrap();
    assert!(
        audit
            .record_validator_repair_pending("validator", "paper", 1, FINDINGS)
            .is_err()
    );
    std::fs::remove_dir(&audit_path).unwrap();
    std::fs::write(&audit_path, without_pending).unwrap();

    let resumed = ResumeRepairLlm {
        call: AtomicUsize::new(0),
        prompts: Mutex::new(Vec::new()),
    };
    resume_pipeline_audited(
        &facade,
        &resumed,
        &facade.session_id,
        tmp.path(),
        None,
        None,
        None,
    )
    .await
    .expect("resume should replay attempt one after the failed checkpoint append");
    let prompts = resumed.prompts.lock().unwrap();
    assert!(prompts.iter().any(|prompt| prompt.contains(PAPER)));
}

#[tokio::test]
async fn repair_prompt_construction_interruption_replays_repair_before_attempt_two() {
    let tmp = tempfile::tempdir().unwrap();
    let session_id = "repair-prompt-interruption-resume";
    let first_facade = ValidatorFacade {
        session_id: session_id.into(),
        fail_prompt_attempt: None,
        panic_repair_prompt: true,
        fail_repair_prompt: false,
    };
    let first_llm = llm(false);
    let first_root = tmp.path().to_path_buf();
    let first = tokio::spawn(async move {
        run_pipeline_audited(
            &first_facade,
            first_llm.as_ref(),
            "task",
            &first_root,
            None,
            None,
            None,
        )
        .await
    });
    assert!(matches!(first.await, Err(join_error) if join_error.is_panic()));

    let facade = ValidatorFacade {
        session_id: session_id.into(),
        fail_prompt_attempt: None,
        panic_repair_prompt: false,
        fail_repair_prompt: false,
    };
    let resumed_llm = ResumeRepairLlm {
        call: AtomicUsize::new(0),
        prompts: Mutex::new(Vec::new()),
    };
    resume_pipeline_audited(
        &facade,
        &resumed_llm,
        session_id,
        tmp.path(),
        None,
        None,
        None,
    )
    .await
    .expect("resume should replay attempt 1 repair before validator attempt 2");
    let prompts = resumed_llm.prompts.lock().unwrap();
    assert!(prompts[0].contains("Current paper:\n"));
    assert!(prompts[0].contains(PAPER));
    assert!(prompts[0].contains(FINDINGS));
    assert!(prompts[1].contains(REPAIRED));
}

#[tokio::test]
async fn repair_prompt_build_error_rejects_repair_and_attempt_two_judges_old_paper() {
    let tmp = tempfile::tempdir().unwrap();
    let facade = ValidatorFacade {
        session_id: "repair-prompt-build-error".into(),
        fail_prompt_attempt: None,
        panic_repair_prompt: false,
        fail_repair_prompt: true,
    };
    let llm = llm(false);
    run_pipeline_audited(&facade, llm.as_ref(), "task", tmp.path(), None, None, None)
        .await
        .expect("validator attempt 2 should accept the unchanged paper");

    let prompts = llm.prompts.lock().unwrap();
    assert!(prompts[2].contains(PAPER));
    assert!(!prompts[2].contains(REPAIRED));
    let events = std::fs::read_to_string(
        PipelineBundleStore::new(tmp.path())
            .bundle_dir("repair-prompt-build-error")
            .join("audit.log"),
    )
    .unwrap();
    let events = events
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .collect::<Vec<_>>();
    assert!(events.iter().any(|event| {
        event["type"] == "artifact_repair_rejected"
            && event["attempt"] == 1
            && event["reason"]
                .as_str()
                .unwrap()
                .contains("simulated repair prompt construction failure")
    }));
}

#[tokio::test]
async fn prompt_construction_interruption_replays_unjudged_validator_attempt() {
    let tmp = tempfile::tempdir().unwrap();
    let session_id = "prompt-interruption-resume";
    let first_facade = ValidatorFacade {
        session_id: session_id.into(),
        fail_prompt_attempt: Some(2),
        panic_repair_prompt: false,
        fail_repair_prompt: false,
    };
    let first_llm = llm(false);
    assert!(
        run_pipeline_audited(
            &first_facade,
            first_llm.as_ref(),
            "task",
            tmp.path(),
            None,
            None,
            None,
        )
        .await
        .is_err()
    );

    let facade = ValidatorFacade {
        session_id: session_id.into(),
        fail_prompt_attempt: None,
        panic_repair_prompt: false,
        fail_repair_prompt: false,
    };
    let resumed_llm = ResumeBudgetLlm {
        call: AtomicUsize::new(0),
    };
    resume_pipeline_audited(
        &facade,
        &resumed_llm,
        session_id,
        tmp.path(),
        None,
        None,
        None,
    )
    .await
    .expect("resume should replay attempt 2 and finish within the validator budget");

    let events = std::fs::read_to_string(
        PipelineBundleStore::new(tmp.path())
            .bundle_dir(session_id)
            .join("audit.log"),
    )
    .unwrap()
    .lines()
    .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
    .collect::<Vec<_>>();
    let judged_attempts = events
        .iter()
        .filter(|event| {
            event["type"] == "llm_attempt_completed" && event["agent_key"] == "validator"
        })
        .map(|event| event["attempt"].as_u64().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(judged_attempts, vec![1, 2, 3]);
    let progress_attempts = events
        .iter()
        .filter(|event| event["type"] == "validator_progress")
        .map(|event| event["attempt"].as_u64().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(progress_attempts, vec![3]);
    let pending_attempts = events
        .iter()
        .filter(|event| event["type"] == "artifact_repair_pending")
        .map(|event| event["attempt"].as_u64().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(pending_attempts, vec![1, 2]);
}

#[tokio::test]
async fn non_opted_critical_agent_resume_restarts_attempt_cursor() {
    let tmp = tempfile::tempdir().unwrap();
    let facade = ValidatorFacade {
        session_id: "coding-critical-resume".into(),
        fail_prompt_attempt: None,
        panic_repair_prompt: false,
        fail_repair_prompt: false,
    };
    let first_llm = Arc::new(RepairLlm {
        call: AtomicUsize::new(0),
        prompts: Mutex::new(Vec::new()),
        requests: Mutex::new(Vec::new()),
        always_fail: false,
        refuse_repair: false,
        panic_at: None,
        provider_fail_at: Some(1),
        block_persist_at: None,
    });
    assert!(
        run_pipeline_audited(
            &facade,
            first_llm.as_ref(),
            "task",
            tmp.path(),
            None,
            None,
            None
        )
        .await
        .is_err()
    );

    let resumed_llm = ResumeRepairLlm {
        call: AtomicUsize::new(0),
        prompts: Mutex::new(Vec::new()),
    };
    resume_pipeline_audited(
        &facade,
        &resumed_llm,
        "coding-critical-resume",
        tmp.path(),
        None,
        None,
        None,
    )
    .await
    .expect("non-opted agent should resume with main's attempt behavior");
    let log = std::fs::read_to_string(
        PipelineBundleStore::new(tmp.path())
            .bundle_dir("coding-critical-resume")
            .join("audit.log"),
    )
    .unwrap();
    assert!(!log.contains("validator_progress"));
    let completed = log
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .find(|event| event["type"] == "llm_attempt_completed" && event["agent_key"] == "validator")
        .unwrap();
    assert_eq!(completed["attempt"], 1);
}
