//! #284: a fold that is killed half way, folded again, or run while a resume
//! needs the run's write lock. Plus an ignored diagnostic that times one fold
//! on a COPY of a real run, with a probe on that lock.
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

#[test]
#[ignore = "diagnostic; requires ARCHON_FOLD_PROJECT and ARCHON_FOLD_RUN naming a copied project"]
fn diagnostic_fold_a_copied_run() {
    let (Ok(project), Ok(run_id)) = (
        std::env::var("ARCHON_FOLD_PROJECT"),
        std::env::var("ARCHON_FOLD_RUN"),
    ) else {
        return;
    };
    let project = std::path::PathBuf::from(project);
    let store = archon_workflow::WorkflowStore::project(&project);
    let config = crate::command::workflow_live::load_learning_config_for_fixed(&project);
    let stop = Arc::new(AtomicBool::new(false));
    let probe = {
        let (store, run_id, stop) = (store.clone(), run_id.clone(), Arc::clone(&stop));
        std::thread::spawn(move || {
            let (mut max_wait, mut probes) = (std::time::Duration::ZERO, 0u32);
            while !stop.load(Ordering::SeqCst) {
                let asked = std::time::Instant::now();
                let _ = store.with_run_lock(&run_id, |_| Ok(()));
                max_wait = max_wait.max(asked.elapsed());
                probes += 1;
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            (max_wait, probes)
        })
    };
    let started = std::time::Instant::now();
    let result = super::workflow_decompose_learning::fold(&project, &store, &run_id, &config);
    let seconds = started.elapsed().as_secs_f64();
    stop.store(true, Ordering::SeqCst);
    let (max_wait, probes) = probe.join().unwrap();
    eprintln!(
        "FOLD result={result:?} seconds={seconds:.3} run_lock_max_wait_ms={} run_lock_probes={probes}",
        max_wait.as_millis()
    );
}

fn desc_episodes(root: &std::path::Path) -> Vec<String> {
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
    db.run_script(
        "?[episode_id] := *desc_episodes{episode_id}",
        Default::default(),
        cozo::ScriptMutability::Immutable,
    )
    .map(|rows| rows.rows)
    .unwrap_or_default()
    .iter()
    .map(|row| row[0].get_str().unwrap_or_default().to_string())
    .collect()
}

fn reasoning_patterns(root: &std::path::Path) -> Vec<(String, String, String)> {
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
    db.run_script(
        "?[pattern_id, pattern_type, description] := *patterns{pattern_id, pattern_type, description}",
        Default::default(),
        cozo::ScriptMutability::Immutable,
    )
    .unwrap()
    .rows
    .iter()
    .map(|row| {
        (
            row[0].get_str().unwrap().to_string(),
            row[1].get_str().unwrap().to_string(),
            row[2].get_str().unwrap().to_string(),
        )
    })
    .collect()
}

fn reasoning_embeddings_are_empty(root: &std::path::Path) -> bool {
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
    db.run_script(
        "?[embedding] := *patterns{embedding}",
        Default::default(),
        cozo::ScriptMutability::Immutable,
    )
    .unwrap()
    .rows
    .iter()
    .all(|row| matches!(&row[0], cozo::DataValue::List(embedding) if embedding.is_empty()))
}

/// A run with one freeze-acceptance call that reported representative findings.
fn run_with_findings(root: &std::path::Path) -> (archon_workflow::WorkflowStore, String) {
    run_with_task_and_findings(root, "review and judge acceptance checks")
}

fn run_with_task_and_findings(
    root: &std::path::Path,
    task: &str,
) -> (archon_workflow::WorkflowStore, String) {
    std::fs::create_dir_all(root.join(".archon")).unwrap();
    let store = archon_workflow::WorkflowStore::project(root);
    let run = store
        .create_run(archon_workflow::WorkflowSpec {
            schema: archon_workflow::spec::WORKFLOW_SCHEMA.to_string(),
            name: "fold-test".to_string(),
            task: task.to_string(),
            target_repository_root: None,
            max_parallelism: 1,
            max_agents: 1,
            stages: Vec::new(),
            permissions: std::collections::BTreeMap::new(),
            learning_hooks: Vec::new(),
        })
        .unwrap();
    let findings = vec![
        serde_json::json!({"subject": "AC-X-001", "text": "check 'AC-X-001' was refuted by the host judge; reason: \"judge reason\"; counterexample: \"failing output\""}),
        serde_json::json!({"subject": "AC-X-002", "text": "Claim was refuted: required file is absent"}),
        serde_json::json!({"subject": "AC-X-003", "text": "cannot pass as written: missing test evidence"}),
        serde_json::json!({"subject": "AC-X-004", "text": "accepted correction: use the checked-in test name"}),
        serde_json::json!({"subject": "AC-X-005", "text": "set-gate finding: gate must reject an empty result"}),
    ];
    let record: archon_workflow::WorkflowV2CallRecord = serde_json::from_value(serde_json::json!({
        "run_id": run.id,
        "call": {"id": "3f2a", "method": "hostCommand",
                 "options": {"host_command": {"commandId": "freeze-acceptance"}}},
        "attempt": 1,
        "input_hash": "in",
        "status": "needs_review",
        "result": {"status": "needs_review", "summary": "host command",
                   "data": {"gateEnvelope": {"policy_findings": findings}}},
    }))
    .unwrap();
    archon_workflow::WorkflowV2ResultStore::new(store.run_dir(&run.id).join("v2"))
        .save_call_record(&record)
        .unwrap();
    (store, run.id)
}

fn desc_only() -> archon_core::config::LearningConfig {
    let mut config = archon_core::config::LearningConfig::default();
    config.sona.enabled = false;
    config.reasoning_bank.enabled = false;
    config.desc.enabled = true;
    config
}

fn reasoning_and_desc() -> archon_core::config::LearningConfig {
    let mut config = desc_only();
    config.reasoning_bank.enabled = true;
    config
}

#[test]
fn enabled_reasoning_bank_fold_persists_gate_findings() {
    let _guard = crate::command::topology_trace::test_lock();
    let temp = tempfile::tempdir().unwrap();
    let (store, run_id) = run_with_findings(temp.path());
    super::workflow_decompose_learning::fold(temp.path(), &store, &run_id, &reasoning_and_desc())
        .unwrap();

    let rows = reasoning_patterns(temp.path());
    assert_eq!(rows.len(), 5, "{rows:?}");
    assert!(reasoning_embeddings_are_empty(temp.path()));
    assert!(rows.iter().all(|(key, kind, description)| {
        key.starts_with("workflow-")
            && kind == "workflow_lesson"
            && description.contains("\"run_id\"")
            && description.contains("\"stage_id\"")
            && description.contains("\"quality\"")
    }));
    let content = rows
        .iter()
        .map(|(_, _, description)| description.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        content.contains("reason: \\\"judge reason\\\""),
        "{content}"
    );
    assert!(
        content.contains("counterexample: \\\"failing output\\\""),
        "{content}"
    );
    assert!(content.contains("required file is absent"), "{content}");
    assert!(content.contains("cannot pass as written"), "{content}");
    assert!(content.contains("accepted correction"), "{content}");
    assert!(content.contains("set-gate finding"), "{content}");
}

#[test]
fn decomposition_fold_routes_structured_gate_findings_to_enabled_reasoning_bank() {
    let _guard = crate::command::topology_trace::test_lock();
    let temp = tempfile::tempdir().unwrap();
    let (store, run_id) =
        run_with_task_and_findings(temp.path(), "Decompose PRD into staged deliverables");
    let config = reasoning_and_desc();

    // EXPECTED: the enabled bank hook is selected for structured gate findings,
    // even when the generated run's top-level task text has no Review marker.
    super::workflow_decompose_learning::fold(temp.path(), &store, &run_id, &config).unwrap();

    let rows = reasoning_patterns(temp.path());
    assert_eq!(
        rows.len(),
        5,
        "gate findings must reach the enabled bank: {rows:?}"
    );
}

#[test]
fn disabled_reasoning_bank_fold_writes_no_patterns() {
    let _guard = crate::command::topology_trace::test_lock();
    let temp = tempfile::tempdir().unwrap();
    let (store, enabled_run) = run_with_findings(temp.path());
    super::workflow_decompose_learning::fold(
        temp.path(),
        &store,
        &enabled_run,
        &reasoning_and_desc(),
    )
    .unwrap();
    let before = reasoning_patterns(temp.path());
    assert_eq!(before.len(), 5, "enabled toggle must persist its rows");

    let (store, disabled_run) = run_with_findings(temp.path());
    super::workflow_decompose_learning::fold(temp.path(), &store, &disabled_run, &desc_only())
        .unwrap();
    let after = reasoning_patterns(temp.path());
    assert_eq!(after, before, "disabled toggle added ReasoningBank rows");
}

#[test]
fn repeated_reasoning_bank_fold_replaces_the_same_rows() {
    let _guard = crate::command::topology_trace::test_lock();
    let temp = tempfile::tempdir().unwrap();
    let (store, run_id) = run_with_findings(temp.path());
    let config = reasoning_and_desc();
    super::workflow_decompose_learning::fold(temp.path(), &store, &run_id, &config).unwrap();
    let mut before = reasoning_patterns(temp.path());
    before.sort();
    super::workflow_decompose_learning::fold(temp.path(), &store, &run_id, &config).unwrap();
    let mut after = reasoning_patterns(temp.path());
    after.sort();

    assert_eq!(after.len(), 5);
    assert_eq!(after, before);
}

#[test]
fn unavailable_learning_store_keeps_the_workflow_report_and_resume_state() {
    let _guard = crate::command::topology_trace::test_lock();
    let temp = tempfile::tempdir().unwrap();
    let (store, run_id) = run_with_findings(temp.path());
    let before = store.load_state(&run_id).unwrap();
    std::fs::create_dir(temp.path().join(".archon/learning-state.db")).unwrap();

    let result = super::workflow_decompose_learning::fold(
        temp.path(),
        &store,
        &run_id,
        &reasoning_and_desc(),
    );

    assert!(
        result.is_ok(),
        "best-effort sink failure changed the fold: {result:?}"
    );
    let after = store.load_state(&run_id).unwrap();
    assert_eq!(after.status, before.status);
    assert_eq!(after.spec, before.spec);
    assert_eq!(after.stages, before.stages);
    assert!(
        store.load_state(&run_id).is_ok(),
        "run remains available for resume"
    );
}

/// A fold killed after delivering part of the spool (its spool kept, one
/// lesson's key recorded) delivers the rest on the next fold, never a second
/// episode for a lesson already stored, and empties the spool.
#[test]
fn a_fold_killed_half_way_resumes_without_duplicate_episodes() {
    let _guard = crate::command::topology_trace::test_lock();
    let temp = tempfile::tempdir().unwrap();
    let (store, run_id) = run_with_findings(temp.path());
    let fold = super::workflow_decompose_learning::fold;
    fold(temp.path(), &store, &run_id, &desc_only()).unwrap();
    let complete = desc_episodes(temp.path());
    assert_eq!(complete.len(), 5, "{complete:?}");

    // The state a kill leaves after the first lesson landed: the spool still
    // holds all five, the progress file names the first.
    let (_, pending) = super::workflow_decompose_learning::journal_paths(&store.run_dir(&run_id));
    let lessons = super::workflow_decompose_learning::gate_finding_records(
        &run_id,
        "freeze-acceptance",
        &serde_json::json!({"gateEnvelope": {"policy_findings": [
            {"subject": "AC-X-001", "text": "check 'AC-X-001' was refuted by the host judge; reason: \"judge reason\"; counterexample: \"failing output\""},
            {"subject": "AC-X-002", "text": "Claim was refuted: required file is absent"},
            {"subject": "AC-X-003", "text": "cannot pass as written: missing test evidence"}
        ]}}),
        &["desc".into()],
    );
    super::workflow_decompose_learning::write_journal(&pending, &lessons).unwrap();
    let first_key = archon_workflow::workflow_scaffold_hash(&lessons[0].stage_id)[..16].to_string();
    std::fs::write(
        pending.with_file_name("decomposition-delivered.keys"),
        format!("{first_key}\n"),
    )
    .unwrap();

    fold(temp.path(), &store, &run_id, &desc_only()).unwrap();
    let mut after = desc_episodes(temp.path());
    after.sort();
    let mut complete = complete;
    complete.sort();
    assert_eq!(
        after, complete,
        "re-delivery replaced episodes, never added one"
    );
    assert!(
        super::workflow_decompose_learning::read_journal(&pending)
            .unwrap()
            .is_empty(),
        "the spool empties once every lesson landed"
    );
    assert!(
        !pending
            .with_file_name("decomposition-delivered.keys")
            .exists()
    );
}

/// A fold holds its own lock, never the run's write lock: a resume (or any
/// run-control write) is not blocked while a fold runs.
#[test]
fn a_fold_never_holds_the_run_write_lock() {
    let temp = tempfile::tempdir().unwrap();
    let (store, run_id) = run_with_findings(temp.path());
    let (taken_tx, taken_rx) = std::sync::mpsc::channel();
    let (store2, run2) = (store.clone(), run_id.clone());
    super::workflow_decompose_learning::with_fold_lock(&store, &run_id, |_| {
        std::thread::spawn(move || {
            let taken = store2.with_run_lock(&run2, |_| Ok(())).is_ok();
            let _ = taken_tx.send(taken);
        });
        let taken = taken_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("the run lock was blocked by the fold lock");
        assert!(taken);
        Ok(())
    })
    .unwrap();
}
