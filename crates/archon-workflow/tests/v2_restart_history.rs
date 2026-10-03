//! Issue-250: an explicit restart is never undone by history reuse. A call
//! whose slot holds an interrupted attempt (which names no task) is still
//! selected by the tasks its archived records did, and those records are
//! marked invalidated too. Driven through the real restart entry points; every
//! check reads the files back.

use archon_workflow::v2::restart::{invalidate_generated_v2_call, restart_generated_v2_task};
use archon_workflow::{
    WorkflowBundle, WorkflowBundleOrigin, WorkflowRun, WorkflowSpec, WorkflowStore,
    WorkflowV2CallRecord, WorkflowV2Evidence, WorkflowV2EvidenceKind, WorkflowV2HostCall,
    WorkflowV2HostMethod, WorkflowV2Result, WorkflowV2ResultStore, WorkflowV2SourceTaskGraph,
    WorkflowV2SourceTaskItem, WorkflowV2Status, WorkflowV2TaskCompletionEvidence,
    WorkflowV2TaskCompletionEvidenceKind,
};

const T1: &str = "2026-10-02T11:48:49+00:00";
const T2: &str = "2026-10-03T04:31:04+00:00";

fn agent_call(id: &str) -> WorkflowV2HostCall {
    WorkflowV2HostCall {
        id: id.to_string(),
        method: WorkflowV2HostMethod::Agent,
        write_mode: None,
        options: Default::default(),
    }
}

/// A generated V2 run with tasks T-A, T-B (depends on T-A) and T-C.
fn generated_run(temp: &tempfile::TempDir, calls: &[&str]) -> (WorkflowStore, WorkflowRun) {
    let store = WorkflowStore::new(temp.path().join("workflows"));
    let run = store
        .create_run(WorkflowSpec {
            schema: archon_workflow::spec::WORKFLOW_SCHEMA.to_string(),
            name: "restart-history".to_string(),
            task: "test".to_string(),
            target_repository_root: None,
            max_parallelism: 1,
            max_agents: 1,
            stages: Vec::new(),
            permissions: Default::default(),
            learning_hooks: Vec::new(),
        })
        .expect("run");
    WorkflowBundle::create_for_run(
        &store,
        &run,
        "export default async function workflow(w) {}",
        WorkflowBundleOrigin::GeneratedHarness,
    )
    .expect("bundle");
    let task = |id: &str, deps: &[&str]| {
        serde_json::json!({
            "canonical_task_id": id,
            "aliases": [],
            "source_path": format!("tasks/{id}.md"),
            "dependency_ids": deps,
        })
    };
    let manifest = calls.iter().map(|id| agent_call(id)).collect::<Vec<_>>();
    let v2 = store.run_dir(&run.id).join("v2");
    std::fs::create_dir_all(&v2).unwrap();
    std::fs::write(
        v2.join("generated-metadata.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "schema_version": "workflow-generated-v2-metadata-v1",
            "task_universe": {
                "schema_version": "workflow-v2-task-universe-v1",
                "source_roots": ["tasks"],
                "tasks": [task("T-A", &[]), task("T-B", &["T-A"]), task("T-C", &[])],
            },
            "generated_scaffold": { "host_call_manifest": manifest },
        }))
        .unwrap(),
    )
    .unwrap();
    (store, run)
}

fn v2_store(store: &WorkflowStore, run: &WorkflowRun) -> WorkflowV2ResultStore {
    WorkflowV2ResultStore::new(store.run_dir(&run.id).join("v2"))
}

fn timed(mut record: WorkflowV2CallRecord, at: &str) -> WorkflowV2CallRecord {
    record.started_at = at.to_string();
    record.finished_at = at.to_string();
    record
}

/// Attempt 1 completed `task` and was accepted.
fn accepted(id: &str, task: &str) -> WorkflowV2CallRecord {
    let mut result = WorkflowV2Result::accepted("done");
    result.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Inspection,
        "criterion checked",
    ));
    let evidence = WorkflowV2TaskCompletionEvidence::new(
        task,
        WorkflowV2TaskCompletionEvidenceKind::ImplementationCandidate,
        id,
        "item",
        WorkflowV2Status::Accepted,
    );
    let record =
        WorkflowV2CallRecord::new("wf", agent_call(id), 1, format!("in-{id}"), result, vec![])
            .with_completion_evidence(vec![evidence]);
    timed(record, T1)
}

/// Attempt 2, stopped by a pause: no task ids, no source graph.
fn interrupted(id: &str) -> WorkflowV2CallRecord {
    let result = WorkflowV2Result {
        status: WorkflowV2Status::NeedsReview,
        summary: "paused and produced no result".to_string(),
        data: serde_json::json!({ "interrupted": "paused" }),
        ..WorkflowV2Result::default()
    };
    let record =
        WorkflowV2CallRecord::new("wf", agent_call(id), 2, "drifted".into(), result, vec![]);
    timed(record, T2)
}

fn graph(item_tasks: &[&str], deps: &[&str]) -> WorkflowV2SourceTaskGraph {
    let strings = |ids: &[&str]| ids.iter().map(|id| id.to_string()).collect::<Vec<_>>();
    WorkflowV2SourceTaskGraph::new(
        strings(&["T-A", "T-B", "T-C"]),
        vec![WorkflowV2SourceTaskItem {
            item_id: "item".to_string(),
            canonical_task_ids: strings(item_tasks),
            dependency_ids: strings(deps),
            target_files: vec![],
            declared_target_files: vec![],
            target_file_expansions: vec![],
            acceptance_criteria: vec![],
            focused_verification: vec![],
            expected_evidence: vec![],
            artifact_requirements: vec![],
            required_tools: vec![],
        }],
        strings(item_tasks),
    )
}

/// Every archived record of `id`, read from the files.
fn archived(store: &WorkflowV2ResultStore, id: &str) -> Vec<WorkflowV2CallRecord> {
    let stem = store
        .result_path(id)
        .file_stem()
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    let dir = store.root().join("results/superseded");
    std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .filter(|entry| {
                    entry
                        .file_name()
                        .to_string_lossy()
                        .starts_with(&format!("{stem}-"))
                })
                .map(|entry| serde_json::from_slice(&std::fs::read(entry.path()).unwrap()).unwrap())
                .collect()
        })
        .unwrap_or_default()
}

fn slot(store: &WorkflowV2ResultStore, id: &str) -> Option<WorkflowV2CallRecord> {
    let path = store.result_path(id);
    path.exists()
        .then(|| serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap())
}

fn assert_never_reused(store: &WorkflowV2ResultStore, id: &str, reason: &str) {
    let accepted_copies = archived(store, id)
        .into_iter()
        .filter(|record| record.status == WorkflowV2Status::Accepted)
        .collect::<Vec<_>>();
    assert!(
        !accepted_copies.is_empty(),
        "{id}: accepted record archived"
    );
    for record in accepted_copies {
        assert_eq!(record.invalidated_by.as_deref(), Some(reason), "{id}");
    }
    let input = format!("in-{id}");
    assert!(
        store
            .last_accepted_call_record(id, &input)
            .unwrap()
            .is_none(),
        "{id}"
    );
    let candidate = store
        .call_record_for_reuse(&agent_call(id), &input)
        .unwrap();
    assert!(
        candidate.is_none_or(|candidate| !candidate.from_history),
        "{id}"
    );
}

#[test]
fn restart_task_invalidates_the_history_behind_an_interrupted_slot() {
    let temp = tempfile::tempdir().unwrap();
    let (store, run) = generated_run(&temp, &["author-a", "author-b", "author-c"]);
    let v2 = v2_store(&store, &run);
    for (id, task) in [
        ("author-a", "T-A"),
        ("author-b", "T-B"),
        ("author-c", "T-C"),
    ] {
        v2.save_call_record(&accepted(id, task)).unwrap();
        v2.save_call_record(&interrupted(id)).unwrap();
        assert!(
            v2.last_accepted_call_record(id, &format!("in-{id}"))
                .unwrap()
                .is_some()
        );
    }

    let invalidation = restart_generated_v2_task(&store, &run, "T-A")
        .unwrap()
        .unwrap();

    let ids = invalidation.invalidated_call_ids;
    assert!(ids.contains(&"author-a".to_string()), "{ids:?}");
    assert!(
        ids.contains(&"author-b".to_string()),
        "downstream T-B: {ids:?}"
    );
    assert!(!ids.contains(&"author-c".to_string()), "{ids:?}");
    for id in ["author-a", "author-b"] {
        let reason = "restart-task:T-A";
        assert_eq!(
            slot(&v2, id).unwrap().invalidated_by.as_deref(),
            Some(reason)
        );
        assert_never_reused(&v2, id, reason);
    }
    // The unrelated task's accepted record still answers its resume.
    assert!(
        v2.last_accepted_call_record("author-c", "in-author-c")
            .unwrap()
            .is_some()
    );
    assert!(
        archived(&v2, "author-c")
            .iter()
            .all(|r| r.invalidated_by.is_none())
    );
}

#[test]
fn restart_task_marks_the_history_when_a_crash_left_the_slot_empty() {
    let temp = tempfile::tempdir().unwrap();
    let (store, run) = generated_run(&temp, &["author-a"]);
    let v2 = v2_store(&store, &run);
    v2.save_call_record(&accepted("author-a", "T-A")).unwrap();
    // Killed between the archive and the write of attempt 2: the record was
    // renamed into the archive, exactly as `archive_superseded_json` names it.
    let path = v2.result_path("author-a");
    let stem = path.file_stem().unwrap().to_str().unwrap().to_string();
    let dir = v2.root().join("results/superseded");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::rename(&path, dir.join(format!("{stem}-1-1-0.json"))).unwrap();
    assert!(slot(&v2, "author-a").is_none());
    assert!(
        v2.last_accepted_call_record("author-a", "in-author-a")
            .unwrap()
            .is_some()
    );

    let invalidation = restart_generated_v2_task(&store, &run, "T-A")
        .unwrap()
        .unwrap();

    assert!(
        invalidation
            .invalidated_call_ids
            .contains(&"author-a".to_string())
    );
    assert!(slot(&v2, "author-a").is_none());
    assert_never_reused(&v2, "author-a", "restart-task:T-A");
}

#[test]
fn restart_agent_invalidates_downstream_history_behind_an_interrupted_slot() {
    let temp = tempfile::tempdir().unwrap();
    // No manifest edge links the two: only the source graphs do.
    let (store, run) = generated_run(&temp, &["wave-a", "author-b"]);
    let v2 = v2_store(&store, &run);
    let wave = accepted("wave-a", "T-A")
        .with_source_metadata(Some("fp".into()), Some(graph(&["T-A"], &[])));
    v2.save_call_record(&wave).unwrap();
    let downstream = accepted("author-b", "T-B")
        .with_source_metadata(Some("fp".into()), Some(graph(&["T-B"], &["T-A"])));
    v2.save_call_record(&downstream).unwrap();
    v2.save_call_record(&interrupted("author-b")).unwrap();
    assert!(
        v2.last_accepted_call_record("author-b", "in-author-b")
            .unwrap()
            .is_some()
    );

    let invalidated = invalidate_generated_v2_call(&store, &run, "wave-a").unwrap();

    assert!(
        invalidated.contains(&"author-b".to_string()),
        "{invalidated:?}"
    );
    assert_eq!(
        slot(&v2, "author-b").unwrap().invalidated_by.as_deref(),
        Some("wave-a")
    );
    assert_never_reused(&v2, "author-b", "wave-a");
}
