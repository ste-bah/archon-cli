//! A generated V2 run with a persisted task universe and host-call manifest,
//! for driving the real restart entry points (`v2::restart`).
#![allow(dead_code)]

use archon_workflow::{
    WorkflowBundle, WorkflowBundleOrigin, WorkflowRun, WorkflowSpec, WorkflowStore,
    WorkflowV2CallRecord, WorkflowV2Evidence, WorkflowV2EvidenceKind, WorkflowV2HostCall,
    WorkflowV2HostMethod, WorkflowV2Result, WorkflowV2ResultStore, WorkflowV2Status,
    WorkflowV2TaskCompletionEvidence, WorkflowV2TaskCompletionEvidenceKind,
};

pub const T1: &str = "2026-10-02T11:48:49+00:00";
pub const T2: &str = "2026-10-03T04:31:04+00:00";

pub fn agent_call(id: &str) -> WorkflowV2HostCall {
    WorkflowV2HostCall {
        id: id.to_string(),
        method: WorkflowV2HostMethod::Agent,
        write_mode: None,
        options: Default::default(),
    }
}

/// A generated V2 run with tasks T-A, T-B (depends on T-A) and T-C, whose
/// manifest names `calls`.
pub fn generated_run(temp: &tempfile::TempDir, calls: &[&str]) -> (WorkflowStore, WorkflowRun) {
    let store = WorkflowStore::new(temp.path().join("workflows"));
    let run = store
        .create_run(WorkflowSpec {
            schema: archon_workflow::spec::WORKFLOW_SCHEMA.to_string(),
            name: "restart-control".to_string(),
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

pub fn v2_store(store: &WorkflowStore, run: &WorkflowRun) -> WorkflowV2ResultStore {
    WorkflowV2ResultStore::new(store.run_dir(&run.id).join("v2"))
}

fn timed(mut record: WorkflowV2CallRecord, at: &str) -> WorkflowV2CallRecord {
    record.started_at = at.to_string();
    record.finished_at = at.to_string();
    record
}

/// Attempt 1 completed `task` and was accepted.
pub fn accepted(id: &str, task: &str) -> WorkflowV2CallRecord {
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
pub fn interrupted(id: &str) -> WorkflowV2CallRecord {
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

/// The record in `id`'s slot, read from the file.
pub fn slot(store: &WorkflowV2ResultStore, id: &str) -> Option<WorkflowV2CallRecord> {
    let path = store.result_path(id);
    path.exists()
        .then(|| serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap())
}
