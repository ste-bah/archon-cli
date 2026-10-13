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

/// A run with one freeze-acceptance call that reported three findings.
fn run_with_findings(root: &std::path::Path) -> (archon_workflow::WorkflowStore, String) {
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
    let findings: Vec<serde_json::Value> = (1..=3)
        .map(|n| serde_json::json!({"subject": format!("AC-X-00{n}"), "text": format!("check 'AC-X-00{n}' was refuted by the host judge; reason: \"R{n}\"; counterexample: \"C{n}\"")}))
        .collect();
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
    assert_eq!(complete.len(), 3, "{complete:?}");

    // The state a kill leaves after the first lesson landed: the spool still
    // holds all three, the progress file names the first.
    let (_, pending) = super::workflow_decompose_learning::journal_paths(&store.run_dir(&run_id));
    let lessons = super::workflow_decompose_learning::gate_finding_records(
        &run_id,
        "freeze-acceptance",
        &serde_json::json!({"gateEnvelope": {"policy_findings": (1..=3)
            .map(|n| serde_json::json!({"subject": format!("AC-X-00{n}"), "text": format!("check 'AC-X-00{n}' was refuted by the host judge; reason: \"R{n}\"; counterexample: \"C{n}\"")}))
            .collect::<Vec<_>>()}}),
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
