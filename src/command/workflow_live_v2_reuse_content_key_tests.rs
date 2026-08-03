use super::*;

// Reuse must be CONTENT-KEYED. These tests cover the two halves of that:
// the cache key itself must distinguish inputs that differ (including array
// order), and the reuse paths on a resume must consult it.
//
// Included from `workflow_live_v2_script_tests.rs` — do NOT run rustfmt on
// this file directly; it is `include!`d, not a `mod`.

pub(super) fn reuse_test_store(
    temp: &tempfile::TempDir,
) -> (WorkflowStore, archon_workflow::WorkflowRun) {
    let workflow_store = WorkflowStore::new(temp.path().join("workflows"));
    let run = workflow_store.create_run(test_spec()).expect("run");
    (workflow_store, run)
}

pub(super) fn reuse_test_runner(
    workflow_store: &WorkflowStore,
    run: &archon_workflow::WorkflowRun,
    v2_store: &WorkflowV2ResultStore,
    script_args: serde_json::Value,
    universe: Option<WorkflowV2TaskUniverse>,
) -> WorkflowV2ScriptRunner {
    let (tui_tx, tui_rx) = bounded_tui_event_channel();
    // The TUI receiver must outlive the run: a closed channel aborts every
    // host call before it can reuse or execute.
    std::mem::forget(tui_rx);
    let client = LiveV2AgentClient::new(
        Arc::new(PanicLlm),
        tui_tx,
        Vec::new(),
        run.id.clone(),
        None,
        None,
    );
    WorkflowV2ScriptRunner::new(
        "content-keyed reuse".to_string(),
        test_runtime(&test_spec()),
        WorkflowV2AgentAdapter::new(),
        client,
        v2_store.clone(),
        workflow_store.clone(),
        run.id.clone(),
        true,
        universe,
        Some(script_args),
    )
}

pub(super) const REUSE_PROBE_SCRIPT: &str = r#"
async function workflow(w) {
  await w.checkpoint("downstream", { inputs: args.items });
  return "done";
}
"#;

/// A `checkpoint` result echoes its input, so the persisted record's data
/// tells us whether the call ran fresh or replayed a stored result.
pub(super) fn recorded_items(v2_store: &WorkflowV2ResultStore, call_id: &str) -> serde_json::Value {
    v2_store
        .load_call_record(call_id)
        .expect("call record lookup")
        .expect("call record present")
        .result
        .data
        .get("source_data")
        .cloned()
        .unwrap_or(serde_json::Value::Null)
}

#[tokio::test]
async fn resume_reuses_a_call_whose_input_is_unchanged() {
    let temp = tempfile::tempdir().expect("tempdir");
    let (workflow_store, run) = reuse_test_store(&temp);
    let v2_store = WorkflowV2ResultStore::new(workflow_store.run_dir(&run.id).join("v2"));
    let args = serde_json::json!({ "items": [{ "item_id": "a" }, { "item_id": "b" }] });

    let first = reuse_test_runner(&workflow_store, &run, &v2_store, args.clone(), None)
        .run(REUSE_PROBE_SCRIPT)
        .await
        .expect("first run");
    assert_eq!(first.executed, 1, "first run must execute the call");
    assert_eq!(first.reused, 0);

    let resumed = reuse_test_runner(&workflow_store, &run, &v2_store, args, None)
        .with_frontier_resume(true)
        .run(REUSE_PROBE_SCRIPT)
        .await
        .expect("resumed run");
    assert_eq!(
        resumed.reused, 1,
        "an unchanged input must still be served from the frontier"
    );
    assert_eq!(resumed.executed, 0);
}

#[tokio::test]
async fn resume_reexecutes_a_call_whose_upstream_output_changed() {
    let temp = tempfile::tempdir().expect("tempdir");
    let (workflow_store, run) = reuse_test_store(&temp);
    let v2_store = WorkflowV2ResultStore::new(workflow_store.run_dir(&run.id).join("v2"));

    let first = reuse_test_runner(
        &workflow_store,
        &run,
        &v2_store,
        serde_json::json!({ "items": [{ "item_id": "a" }] }),
        None,
    )
    .run(REUSE_PROBE_SCRIPT)
    .await
    .expect("first run");
    assert_eq!(first.executed, 1);

    // The resume feeds the SAME call a different upstream payload. Nothing
    // invalidated the stored record — `invalidate_*` only ever runs from the
    // operator's `workflow restart` command — so only the input hash can
    // catch this.
    let resumed = reuse_test_runner(
        &workflow_store,
        &run,
        &v2_store,
        serde_json::json!({ "items": [{ "item_id": "a" }, { "item_id": "c" }] }),
        None,
    )
    .with_frontier_resume(true)
    .run(REUSE_PROBE_SCRIPT)
    .await
    .expect("resumed run");

    assert_eq!(
        resumed.reused, 0,
        "frontier reuse must not replay a record whose input changed"
    );
    assert_eq!(resumed.executed, 1);
    assert_eq!(
        recorded_items(&v2_store, "downstream"),
        serde_json::json!([{ "item_id": "a" }, { "item_id": "c" }]),
        "the persisted record must hold the fresh input, not the replayed one"
    );
}

#[tokio::test]
async fn resume_reexecutes_a_call_whose_source_array_was_only_reordered() {
    // Array position IS branch identity on the v3 path: `fanout_item_id`
    // falls back to the item index, so permuting `source_data` reassigns
    // every branch. The old canonicalizing hash sorted array elements and
    // gave both orderings the same key.
    let temp = tempfile::tempdir().expect("tempdir");
    let (workflow_store, run) = reuse_test_store(&temp);
    let v2_store = WorkflowV2ResultStore::new(workflow_store.run_dir(&run.id).join("v2"));

    reuse_test_runner(
        &workflow_store,
        &run,
        &v2_store,
        serde_json::json!({ "items": [{ "item_id": "a" }, { "item_id": "b" }] }),
        None,
    )
    .run(REUSE_PROBE_SCRIPT)
    .await
    .expect("first run");

    let resumed = reuse_test_runner(
        &workflow_store,
        &run,
        &v2_store,
        serde_json::json!({ "items": [{ "item_id": "b" }, { "item_id": "a" }] }),
        None,
    )
    .with_frontier_resume(true)
    .run(REUSE_PROBE_SCRIPT)
    .await
    .expect("resumed run");

    assert_eq!(
        resumed.reused, 0,
        "a permuted source array is a different input and must re-execute"
    );
    assert_eq!(
        recorded_items(&v2_store, "downstream"),
        serde_json::json!([{ "item_id": "b" }, { "item_id": "a" }])
    );
}

/// The hash scheme this workspace shipped before the fix: sha256 over a
/// canonical form that sorted object keys AND array elements recursively.
pub(super) fn legacy_stable_hash(value: &serde_json::Value) -> String {
    use sha2::{Digest, Sha256};

    fn canonical(value: &serde_json::Value) -> serde_json::Value {
        match value {
            serde_json::Value::Array(items) => {
                let mut values = items.iter().map(canonical).collect::<Vec<_>>();
                values.sort_by(|left, right| {
                    serde_json::to_string(left)
                        .unwrap_or_default()
                        .cmp(&serde_json::to_string(right).unwrap_or_default())
                });
                serde_json::Value::Array(values)
            }
            serde_json::Value::Object(object) => {
                let mut sorted = serde_json::Map::new();
                for (key, value) in object.iter().collect::<std::collections::BTreeMap<_, _>>() {
                    sorted.insert(key.clone(), canonical(value));
                }
                serde_json::Value::Object(sorted)
            }
            other => other.clone(),
        }
    }

    let bytes = serde_json::to_vec(&canonical(value)).unwrap_or_default();
    format!("{:x}", Sha256::digest(bytes))
}

#[tokio::test]
async fn a_run_recorded_under_the_old_hash_scheme_resumes_cleanly() {
    // Changing the hash makes every persisted `input_hash` stale. That must
    // cost one re-execution and nothing else: no crash, no partial state, no
    // orphaned record, no stale checkpoint entry.
    let temp = tempfile::tempdir().expect("tempdir");
    let (workflow_store, run) = reuse_test_store(&temp);
    let v2_store = WorkflowV2ResultStore::new(workflow_store.run_dir(&run.id).join("v2"));
    let items = serde_json::json!([{ "item_id": "a" }, { "item_id": "b" }]);

    let legacy_input = serde_json::json!({
        "objective": "content-keyed reuse",
        "call_id": "downstream",
        "method": "checkpoint",
        "write_mode": serde_json::Value::Null,
        "options": { "inputs": items.clone() },
        "inputs": items.clone(),
        "source_data": items.clone(),
    });
    let mut legacy_result = WorkflowV2Result::accepted("recorded under the old hash scheme");
    legacy_result.data = legacy_input.clone();
    legacy_result.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Inspection,
        "legacy checkpoint evidence",
    ));
    let legacy_record = WorkflowV2CallRecord::new(
        run.id.clone(),
        WorkflowV2HostCall {
            id: "downstream".to_string(),
            method: WorkflowV2HostMethod::Checkpoint,
            write_mode: None,
            options: WorkflowV2HostOptions::default(),
        },
        1,
        legacy_stable_hash(&legacy_input),
        legacy_result,
        Vec::new(),
    )
    .with_scaffold_hash(Some(workflow_scaffold_hash(REUSE_PROBE_SCRIPT)));
    v2_store
        .save_call_record(&legacy_record)
        .expect("seed legacy record");
    let mut checkpoint = WorkflowV2Checkpoint::default();
    checkpoint.mark_completed("downstream");
    v2_store
        .save_checkpoint(&checkpoint)
        .expect("seed legacy checkpoint");

    let resumed = reuse_test_runner(
        &workflow_store,
        &run,
        &v2_store,
        serde_json::json!({ "items": items.clone() }),
        None,
    )
    .with_frontier_resume(true)
    .run(REUSE_PROBE_SCRIPT)
    .await
    .expect("a resume across a hash-scheme change must not error");

    assert_eq!(resumed.status, WorkflowV2Status::Accepted);
    assert_eq!(
        resumed.executed, 1,
        "a stale key must cost exactly one clean re-execution"
    );
    assert_eq!(resumed.reused, 0);
    let record = v2_store
        .load_call_record("downstream")
        .expect("call record lookup")
        .expect("record present");
    assert_eq!(record.attempt, 2, "the legacy record must be superseded");
    assert_eq!(
        record.input_hash,
        archon_workflow::stable_value_hash(&record.result.data),
        "the rewritten record must be keyed under the new scheme"
    );
    assert!(
        v2_store
            .load_checkpoint()
            .expect("checkpoint")
            .expect("checkpoint present")
            .completed_call_ids
            .contains(&"downstream".to_string()),
        "the checkpoint must still credit the call after re-execution"
    );
}

pub(super) const TWO_TASK_SCRIPT: &str = r#"
async function workflow(w) {
  await w.checkpoint("implement-task-tdl-001", { inputs: args.upstream });
  await w.checkpoint("implement-task-tdl-010", { inputs: args.downstream });
  return "done";
}
"#;

pub(super) fn completed_task_record(
    run_id: &str,
    call_id: &str,
    task_id: &str,
    input_hash: &str,
) -> WorkflowV2CallRecord {
    let mut result = WorkflowV2Result::accepted("recorded in an earlier session");
    result.data = serde_json::json!({ "source_data": [{ "item_id": "stale" }] });
    result.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Inspection,
        "durable evidence",
    ));
    WorkflowV2CallRecord::new(
        run_id,
        WorkflowV2HostCall {
            id: call_id.to_string(),
            method: WorkflowV2HostMethod::Checkpoint,
            write_mode: None,
            options: WorkflowV2HostOptions::default(),
        },
        1,
        input_hash.to_string(),
        result,
        Vec::new(),
    )
    .with_completion_evidence(vec![WorkflowV2TaskCompletionEvidence::new(
        task_id,
        archon_workflow::WorkflowV2TaskCompletionEvidenceKind::ImplementationCandidate,
        call_id,
        "item-0",
        WorkflowV2Status::Accepted,
    )])
}

/// Build the two-task universe with an explicit dependency edge from the
/// downstream task to the upstream one (or without it, for the control).
pub(super) fn two_task_universe(downstream_depends_on_upstream: bool) -> WorkflowV2TaskUniverse {
    let mut universe = task_universe();
    if !downstream_depends_on_upstream {
        for task in &mut universe.tasks {
            task.dependency_ids.clear();
        }
    }
    universe
}

pub(super) async fn run_completed_task_resume(
    temp: &tempfile::TempDir,
    downstream_depends_on_upstream: bool,
) -> (WorkflowV2ScriptSummary, WorkflowV2ResultStore) {
    let (workflow_store, run) = reuse_test_store(temp);
    let v2_store = WorkflowV2ResultStore::new(workflow_store.run_dir(&run.id).join("v2"));
    // Only the DOWNSTREAM task has a stored record, so the upstream call has
    // to do real work in this resume — exactly the mid-run re-execution that
    // no invalidation pass covers.
    v2_store
        .save_call_record(&completed_task_record(
            &run.id,
            "implement-task-tdl-010",
            "TASK-TDL-010",
            "input-hash-from-an-earlier-session",
        ))
        .expect("seed downstream record");

    let summary = reuse_test_runner(
        &workflow_store,
        &run,
        &v2_store,
        serde_json::json!({
            "upstream": [{ "item_id": "fresh-upstream" }],
            "downstream": [{ "item_id": "fresh-downstream" }],
        }),
        Some(two_task_universe(downstream_depends_on_upstream)),
    )
    .with_frontier_resume(true)
    .with_resume_completed_ids(std::collections::BTreeSet::from([
        "TASK-TDL-010".to_string()
    ]))
    .run(TWO_TASK_SCRIPT)
    .await
    .expect("resumed run");
    (summary, v2_store)
}

#[tokio::test]
async fn completed_task_reuse_is_refused_once_an_upstream_task_reexecutes() {
    // `record_tasks_all_completed` waives the input hash on purpose so
    // `restart task <id>` does not force every prior task to re-validate.
    // The waiver is bounded to work this run has not redone: TASK-TDL-010
    // depends on TASK-TDL-001, which just re-executed, so its stored record
    // is stale and must not be replayed.
    let temp = tempfile::tempdir().expect("tempdir");
    let (summary, v2_store) = run_completed_task_resume(&temp, true).await;

    assert_eq!(
        summary.reused, 0,
        "a completed-task record downstream of re-executed work must not replay"
    );
    assert_eq!(summary.executed, 2);
    assert_eq!(
        recorded_items(&v2_store, "implement-task-tdl-010"),
        serde_json::json!([{ "item_id": "fresh-downstream" }]),
        "the downstream record must hold freshly executed output"
    );
}

#[tokio::test]
async fn completed_task_reuse_survives_unrelated_reexecution() {
    // The control: the same re-execution, but nothing links the two tasks.
    // The restart-task waiver must still skip the completed task, hash
    // mismatch and all — that is the whole reason the path exists.
    let temp = tempfile::tempdir().expect("tempdir");
    let (summary, v2_store) = run_completed_task_resume(&temp, false).await;

    assert_eq!(
        summary.reused, 1,
        "an unrelated re-execution must not force a completed task to re-validate"
    );
    assert_eq!(summary.executed, 1);
    assert_eq!(
        recorded_items(&v2_store, "implement-task-tdl-010"),
        serde_json::json!([{ "item_id": "stale" }]),
        "the completed task must have been served from its stored record"
    );
}
