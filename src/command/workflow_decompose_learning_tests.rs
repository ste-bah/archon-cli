use super::workflow_decompose_learning::journal::read_journal;
use super::workflow_decompose_learning::{
    finish_journal_dispatch, lesson_records, redact_lesson, stable_lesson_key, stage_journal,
    write_journal,
};

#[path = "workflow_decompose_learning_evidence_tests.rs"]
mod evidence;

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
fn secret_crossing_the_storage_boundary_is_redacted_before_truncation() {
    let secret = "ghp_abcdefghijklmnopqrstuvwxyz1234567890";
    let source = format!("{}token={secret}", "x".repeat(1190));
    let stored = redact_lesson(&source);
    assert!(!stored.contains(secret));
    assert!(
        !stored.contains("ghp_"),
        "partial credential escaped: {stored}"
    );
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
fn learning_decompose_run_end_routes_judged_findings_to_both_sinks() {
    let mut config = archon_core::config::LearningConfig::default();
    config.reasoning_bank.enabled = true;
    config.desc.enabled = true;
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
            .any(|record| record.hooks.contains(&"desc".to_string()))
    );
    assert!(
        records
            .iter()
            .any(|record| record.name.contains("misses output"))
    );
    assert!(
        records
            .iter()
            .all(|record| record.hooks.contains(&"reasoning_bank".to_string()))
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
    let history = std::fs::read_to_string(&journal).unwrap();
    assert_eq!(history.lines().count(), 1);
    assert!(
        stage_journal(&journal, &pending_journal, candidates)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn learning_decompose_history_is_scoped_to_each_run() {
    let dir = tempfile::tempdir().unwrap();
    let candidates = lesson_records(
        "run",
        "review",
        "judge-call",
        1,
        &serde_json::json!({"judgment":{"verdict":"refuted","reason":"lesson prose","counterexample":"example"}}),
        &[],
    );
    for index in 0..32 {
        let run_dir = dir.path().join(format!("run-{index}"));
        let (history, pending) = super::workflow_decompose_learning::journal_paths(&run_dir);
        std::fs::create_dir_all(history.parent().unwrap()).unwrap();
        stage_journal(&history, &pending, candidates.clone()).unwrap();
        let history_text = std::fs::read_to_string(history).unwrap();
        assert_eq!(history_text.lines().count(), 1);
        assert_eq!(history_text.trim().len(), 16);
        assert!(!history_text.contains("lesson prose"));
        assert!(!history_text.contains("example"));
    }
    assert!(!dir.path().join("decomposition-records.jsonl").exists());
}

#[test]
fn learning_decompose_history_stores_only_bounded_lesson_keys() {
    let dir = tempfile::tempdir().unwrap();
    let journal = dir.path().join("decomposition-records.jsonl");
    let pending_journal = dir.path().join("decomposition-pending.jsonl");
    let candidates = lesson_records(
        "run-1",
        "review",
        "judge-call",
        1,
        &serde_json::json!({"judgment":{"verdict":"refuted","reason":"private lesson prose","counterexample":"secret example"}}),
        &[],
    );

    stage_journal(&journal, &pending_journal, candidates).unwrap();

    let history = std::fs::read_to_string(&journal).unwrap();
    assert!(!history.contains("private lesson prose"));
    assert!(!history.contains("secret example"));
    let keys: Vec<_> = history.lines().collect();
    assert_eq!(keys.len(), 1);
    assert_eq!(keys[0].len(), 16, "one fixed-size digest per seen lesson");
}

#[test]
fn learning_decompose_resume_migrates_legacy_history_without_redelivery() {
    let dir = tempfile::tempdir().unwrap();
    let journal = dir.path().join("decomposition-records.jsonl");
    let pending_journal = dir.path().join("decomposition-pending.jsonl");
    let candidates = lesson_records(
        "run-1",
        "review",
        "judge-call",
        1,
        &serde_json::json!({"judgment":{"verdict":"refuted","reason":"legacy lesson prose","counterexample":"legacy example"}}),
        &[],
    );
    write_journal(&journal, &candidates).unwrap();

    assert!(
        stage_journal(&journal, &pending_journal, candidates)
            .unwrap()
            .is_empty()
    );

    let history = std::fs::read_to_string(journal).unwrap();
    assert!(!history.contains("legacy lesson prose"));
    assert!(!history.contains("legacy example"));
    assert_eq!(history.lines().count(), 1);
    assert_eq!(history.trim().len(), 16);
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

#[tracing_test::traced_test]
#[test]
fn learning_decompose_best_effort_fold_calls_once_and_handles_both_results() {
    let error_calls = std::cell::Cell::new(0);
    let fold_result: Option<()> =
        super::workflow_decompose_learning::best_effort_fold("run-1", || {
            error_calls.set(error_calls.get() + 1);
            Err(anyhow::anyhow!("db offline"))
        });
    assert_eq!(error_calls.get(), 1);
    assert!(fold_result.is_none());
    assert!(logs_contain("run_id=run-1"));
    assert!(logs_contain("error=db offline"));

    let success_calls = std::cell::Cell::new(0);
    let fold_result = super::workflow_decompose_learning::best_effort_fold("run-2", || {
        success_calls.set(success_calls.get() + 1);
        Ok("folded")
    });
    assert_eq!(success_calls.get(), 1);
    assert_eq!(fold_result, Some("folded"));
}

/// A host command's gate findings, in the shape the host writes them
/// (`result.data.gateEnvelope.policy_findings[]`).
fn gate_envelope(findings: serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "exitCode": 0,
        "gateEnvelope": { "policy_findings": findings, "report": "", "schema_version": 1 },
    })
}

fn judge_refutation() -> serde_json::Value {
    serde_json::json!({
        "remediation_scope": "candidate_artifact",
        "subject": "AC-X-001",
        "text": "check 'AC-X-001' was refuted by the host judge; reason: \"REASON-NO-PINNED-TEST\"; counterexample: \"COUNTER-PROBABILITY-PASSES\"",
    })
}

#[test]
fn learning_decompose_real_gate_findings_become_lessons_by_kind() {
    let source = gate_envelope(serde_json::json!([
        judge_refutation(),
        {"remediation_scope": "body", "subject": "TASK-X-001",
         "text": "task TASK-X-001: its claim of 'DONE-1' is refuted at base commit abc: `a.txt` is not in the repository"},
        {"deterministic_defect": {"code": "check_not_proven", "subject": "AC-X-002"},
         "remediation_scope": "candidate_artifact", "subject": "AC-X-002",
         "text": "check 'AC-X-002': it names the live root by its absolute path"},
        {"subject": "AC-X-003", "text": "check 'AC-X-003': cannot pass as written: rule: name a concrete file"},
        {"subject": "AC-X-004", "text": "  "},
        judge_refutation(),
    ]));
    let records = super::workflow_decompose_learning::gate_finding_records(
        "run-1",
        "freeze-acceptance",
        &source,
        &["desc".into()],
    );
    let kinds: Vec<&str> = records
        .iter()
        .map(|record| record.artifact_refs[0].as_str())
        .collect();
    assert_eq!(
        kinds,
        [
            "refutation",
            "claim_refutation",
            "gate_finding",
            "cannot_pass",
            "refutation"
        ]
    );
    assert!(
        records[0].name.contains("REASON-NO-PINNED-TEST"),
        "{}",
        records[0].name
    );
    assert!(records[0].name.contains("COUNTER-PROBABILITY-PASSES"));
    assert!(
        records[0]
            .name
            .starts_with("freeze-acceptance refutation on AC-X-001")
    );
    assert!(records[2].name.contains("(check_not_proven)"));
    // One finding repeated is one lesson: the same key.
    assert_eq!(records[0].stage_id, records[4].stage_id);
    assert!(
        records
            .iter()
            .all(|record| record.verification == archon_workflow::Verification::Failed)
    );
    // The generic extractor does not read the envelope a second time.
    assert!(lesson_records("run-1", "", "c", 1, &source, &[]).is_empty());
}

/// #284 end to end: a fold of a run whose host command refuted a check sends
/// the lesson through the bridge into the DESC episode store, drains the
/// pending spool, and a second fold sends nothing again.
#[test]
fn learning_decompose_fold_delivers_gate_findings_to_desc_and_drains_the_spool() {
    let _guard = crate::command::topology_trace::test_lock();
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    std::fs::create_dir_all(root.join(".archon")).unwrap();
    let store = archon_workflow::WorkflowStore::project(root);
    let run = store
        .create_run(archon_workflow::WorkflowSpec {
            schema: archon_workflow::spec::WORKFLOW_SCHEMA.to_string(),
            name: "fold-test".to_string(),
            task: "review and judge acceptance checks".to_string(),
            target_repository_root: None,
            max_parallelism: 1,
            max_agents: 1,
            stages: Vec::new(),
            permissions: std::collections::BTreeMap::new(),
            learning_hooks: Vec::new(),
        })
        .unwrap();
    let v2 = archon_workflow::WorkflowV2ResultStore::new(store.run_dir(&run.id).join("v2"));
    let record: archon_workflow::WorkflowV2CallRecord = serde_json::from_value(serde_json::json!({
        "run_id": run.id,
        "call": {"id": "3f2a", "method": "hostCommand",
                 "options": {"host_command": {"commandId": "freeze-acceptance"}}},
        "attempt": 1,
        "input_hash": "in",
        "status": "needs_review",
        "result": {"status": "needs_review", "summary": "host command", "data": gate_envelope(serde_json::json!([judge_refutation()]))},
    }))
    .unwrap();
    v2.save_call_record(&record).unwrap();
    let mut config = archon_core::config::LearningConfig::default();
    config.sona.enabled = false;
    config.reasoning_bank.enabled = false;
    config.desc.enabled = true;

    super::workflow_decompose_learning::fold(root, &store, &run.id, &config).unwrap();

    let episodes = || {
        let path = root
            .join(".archon/learning-state.db")
            .to_string_lossy()
            .to_string();
        let db = archon_cozo::open_sqlite_guarded_instance(
            &path,
            "read learning test store",
            archon_cozo::CozoGuardConfig::for_db_path(&path),
        )
        .unwrap()
        .db_arc();
        // No episode was ever written: the relation does not exist yet.
        db.run_script(
            "?[episode_id, description] := *desc_episodes{episode_id, description}",
            Default::default(),
            cozo::ScriptMutability::Immutable,
        )
        .map(|rows| rows.rows)
        .unwrap_or_default()
        .iter()
        .map(|row| row[1].get_str().unwrap_or_default().to_string())
        .collect::<Vec<_>>()
    };
    let first = episodes();
    assert_eq!(
        first
            .iter()
            .filter(|description| description.contains("REASON-NO-PINNED-TEST")
                && description.contains("COUNTER-PROBABILITY-PASSES"))
            .count(),
        1,
        "the refutation reached DESC once: {first:?}"
    );
    let (_, pending) = super::workflow_decompose_learning::journal_paths(&store.run_dir(&run.id));
    assert!(
        read_journal(&pending).unwrap().is_empty(),
        "the spool drains once dispatched"
    );

    super::workflow_decompose_learning::fold(root, &store, &run.id, &config).unwrap();
    assert_eq!(
        episodes().len(),
        first.len(),
        "a second fold re-sends nothing"
    );
}
