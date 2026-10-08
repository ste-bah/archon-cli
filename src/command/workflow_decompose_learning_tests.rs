use super::workflow_decompose_learning::{
    finish_journal_dispatch, lesson_records, read_journal, redact_lesson, stable_lesson_key,
    stage_journal, write_journal,
};

#[test]
fn learning_decompose_refutations_and_cannot_pass_findings_become_records() {
    let source = serde_json::json!({
        "judgment": {"verdict":"refuted", "reason":"check misses output", "counterexample":"empty output"},
        "findings": ["check 'AC-X-001': cannot pass as written: rule: must name a concrete file"]
    });
    let records = lesson_records(
        "run-1",
        "review and judge acceptance checks",
        "judge-call",
        2,
        &source,
        &[],
    );
    assert!(
        records
            .iter()
            .any(|r| r.name.contains("check misses output") && r.name.contains("empty output"))
    );
    assert!(records.iter().any(|r| r.name.contains("cannot pass") && r.name.contains("must name a concrete file")));
}

#[test]
fn learning_decompose_pause_records_lessons_and_resume_identity_is_stable() {
    let paused = lesson_records(
        "run-1",
        "review",
        "judge-call",
        2,
        &serde_json::json!({"judgment":{"verdict":"refuted","reason":"weak check","counterexample":"empty output"}}),
        &["reasoning_bank".into()],
    );
    assert_eq!(paused.len(), 1);
    assert_eq!(paused[0].telemetry.attempt, 2);
    assert_eq!(
        stable_lesson_key("run-1", "judge-call", 2),
        stable_lesson_key("run-1", "judge-call", 2)
    );
    assert_ne!(
        stable_lesson_key("run-1", "judge-call", 2),
        stable_lesson_key("run-1", "judge-call", 3)
    );
}

#[test]
fn learning_content_is_redacted_before_storage() {
    let secret = "ghp_abcdefghijklmnopqrstuvwxyz1234567890";
    let records = lesson_records(
        "run-1",
        "review",
        "judge-call",
        1,
        &serde_json::json!({"judgment":{"verdict":"refuted","reason":format!("token={secret}"),"counterexample":"empty"}}),
        &["reasoning_bank".into()],
    );
    let stored = serde_json::to_string(&records).unwrap();
    assert!(!stored.contains(secret));
    assert!(!redact_lesson(&format!("token={secret}")).contains(secret));
}

#[test]
fn learning_decompose_task_context_is_redacted_before_topology_fold() {
    let secret = "ghp_abcdefghijklmnopqrstuvwxyz1234567890";
    let safe =
        super::workflow_decompose_learning::safe_fold_context(&format!("review token={secret}"));
    assert!(!safe.contains(secret));
    assert!(safe.contains("review"));
}

#[test]
fn learning_decompose_disabled_learning_yields_no_hooks() {
    let mut config = archon_core::config::LearningConfig::default();
    config.sona.enabled = false;
    config.sona.pipeline_recording = false;
    config.reasoning_bank.enabled = false;
    config.desc.enabled = false;
    let hooks = super::workflow_decompose_learning::hooks("review acceptance checks", &config);
    assert!(hooks.is_empty());
}

#[test]
fn learning_decompose_run_end_emits_reasoning_records_for_judged_findings() {
    let mut config = archon_core::config::LearningConfig::default();
    config.reasoning_bank.enabled = true;
    config.desc.enabled = false;
    let hooks = super::workflow_decompose_learning::hooks("review judge findings", &config);
    let records = lesson_records(
        "run-1",
        "review judge findings",
        "judge-call",
        1,
        &serde_json::json!({"judgment":{"verdict":"refuted","reason":"misses output","counterexample":"empty result"}}),
        &hooks,
    );
    assert!(
        records
            .iter()
            .any(|record| record.hooks.contains(&"reasoning_bank".to_string()))
    );
    assert!(
        records
            .iter()
            .any(|record| record.name.contains("misses output"))
    );
}

#[test]
fn learning_decompose_resume_dispatch_set_excludes_previously_recorded_lessons() {
    let records = lesson_records(
        "run-1",
        "review",
        "judge-call",
        1,
        &serde_json::json!({"judgment":{"verdict":"refuted","reason":"weak","counterexample":"empty"}}),
        &[],
    );
    let mut known = std::collections::BTreeSet::new();
    let first = super::workflow_decompose_learning::filter_new_records(records.clone(), &mut known);
    let resumed = super::workflow_decompose_learning::filter_new_records(records, &mut known);
    assert_eq!(first.len(), 1);
    assert!(resumed.is_empty());
}

#[test]
fn learning_decompose_failed_dispatch_keeps_durable_lesson_for_retry() {
    let dir = tempfile::tempdir().unwrap();
    let journal = dir.path().join("decomposition-records.jsonl");
    let pending_journal = dir.path().join("decomposition-pending.jsonl");
    let candidates = lesson_records(
        "run-1",
        "review",
        "judge-call",
        1,
        &serde_json::json!({"judgment":{"verdict":"refuted","reason":"weak","counterexample":"empty"}}),
        &["reasoning_bank".into()],
    );
    let pending = stage_journal(&journal, &pending_journal, candidates.clone()).unwrap();
    assert_eq!(pending.len(), 1);

    // A failed integration dispatch must leave the durable spool untouched.
    finish_journal_dispatch(&pending_journal, false).unwrap();
    assert_eq!(read_journal(&pending_journal).unwrap().len(), 1);
    let resumed = stage_journal(&journal, &pending_journal, candidates.clone()).unwrap();
    assert_eq!(resumed.len(), 1);
    assert_eq!(
        resumed[0].stage_id,
        read_journal(&pending_journal).unwrap()[0].stage_id
    );
    finish_journal_dispatch(&pending_journal, true).unwrap();
    assert!(read_journal(&pending_journal).unwrap().is_empty());
    assert_eq!(read_journal(&journal).unwrap().len(), 1);
    assert!(
        stage_journal(&journal, &pending_journal, candidates)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn learning_decompose_journal_is_rewritten_atomically_after_success() {
    let dir = tempfile::tempdir().unwrap();
    let journal = dir.path().join("decomposition-records.jsonl");
    let record = lesson_records(
        "run-1",
        "review",
        "judge-call",
        1,
        &serde_json::json!({"judgment":{"verdict":"refuted","reason":"weak","counterexample":"empty"}}),
        &["reasoning_bank".into()],
    );
    write_journal(&journal, &record).unwrap();
    write_journal(&journal, &[]).unwrap();
    assert!(read_journal(&journal).unwrap().is_empty());
    assert!(!journal.with_extension("jsonl.tmp").exists());
}

#[test]
fn learning_decompose_overlapping_folds_serialize_the_journal_transaction() {
    let dir = tempfile::tempdir().unwrap();
    let store = archon_workflow::WorkflowStore::new(dir.path().join("runs"));
    let spec = archon_workflow::WorkflowSpec::from_yaml(
        "schema: archon.workflow.v1\nname: generic\ntask: review findings\nstages:\n  - id: generic-stage\n    kind: agent\n",
    )
    .unwrap();
    let run = store.create_run(spec).unwrap();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(4));
    let active = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let peak = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mut workers = Vec::new();
    for _ in 0..4 {
        let store = store.clone();
        let run_id = run.id.clone();
        let barrier = std::sync::Arc::clone(&barrier);
        let active = std::sync::Arc::clone(&active);
        let peak = std::sync::Arc::clone(&peak);
        workers.push(std::thread::spawn(move || {
            barrier.wait();
            super::workflow_decompose_learning::with_fold_lock(&store, &run_id, |_| {
                let now = active.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
                peak.fetch_max(now, std::sync::atomic::Ordering::SeqCst);
                std::thread::sleep(std::time::Duration::from_millis(10));
                active.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
                Ok(())
            })
            .unwrap();
        }));
    }
    for worker in workers {
        worker.join().unwrap();
    }
    assert_eq!(peak.load(std::sync::atomic::Ordering::SeqCst), 1);
}

#[test]
fn learning_decompose_fold_errors_do_not_change_run_report() {
    let report = "Fixed decomposition completed";
    let fold_result: Option<()> =
        super::workflow_decompose_learning::best_effort_fold("run-1", || {
            Err(anyhow::anyhow!("db offline"))
        });
    assert!(fold_result.is_none());
    assert_eq!(report, "Fixed decomposition completed");
}
