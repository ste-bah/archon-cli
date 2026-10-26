//! The resume reconciliation: a live body that is not its current landing
//! is rebuilt from that landing's own candidate and restored only when the
//! rebuild hashes to its receipt; otherwise the resume is refused naming
//! the receipt.
use super::*;

#[path = "workflow_decompose_landing_restore_temp_tests.rs"]
mod restore_temp_tests;
use archon_workflow::{
    HostCommandRequest, HostCommandSubject, WorkflowV2CallRecord, WorkflowV2HostCall,
    WorkflowV2HostMethod, WorkflowV2HostOptions, WorkflowV2Result, WorkflowV2Status,
};

const TASK: &str = "TASK-X-010";
const FILE: &str = "TASK-X-010.md";

fn frozen() -> FrozenTask {
    FrozenTask {
        task_id: TASK.into(),
        file_name: FILE.into(),
        depends_on: Vec::new(),
        blocks: Vec::new(),
        implements: Vec::new(),
        deliverable_contracts: Vec::new(),
    }
}

fn skeleton() -> TaskSkeleton {
    TaskSkeleton {
        schema_version: 1,
        acceptance_digest: "digest".into(),
        tasks: vec![frozen()],
    }
}

/// An author's candidate for round `round`.
fn candidate(round: &str) -> String {
    format!(
        "```yaml\ntask_id: {TASK}\ntitle: {round}\ncomplexity: low\nstatus: ready\ndepends_on: []\nblocks: []\nimplements: []\nrequired_env_keys: []\nrequired_tools: []\ndeliverable_contracts: []\n```\n\n## Scope\n\nThe {round} body.\n\n## Focused Tests\n\n- `cargo test -p x`\n"
    )
}

/// A landing record of `round`, accepted or refused, whose receipt names
/// `blake3` for the task file.
fn landing(id: &str, at: &str, round: &str, refused: bool, blake3: &str) -> WorkflowV2CallRecord {
    let receipt = serde_json::json!({
        "call_id": id,
        "committed_at": at,
        "entries": [
            {"relative_path": FILE, "blake3": blake3, "byte_len": landed(round).len()},
            {"relative_path": "gate-envelope.json", "blake3": "e", "byte_len": 1}
        ]
    });
    record(id, at, round, refused, receipt)
}

fn record(
    id: &str,
    at: &str,
    round: &str,
    refused: bool,
    receipt: serde_json::Value,
) -> WorkflowV2CallRecord {
    let findings: Vec<serde_json::Value> = if refused {
        vec![serde_json::json!({"remediation_scope": "body", "text": "gap"})]
    } else {
        Vec::new()
    };
    WorkflowV2CallRecord {
        run_id: "wf-reconcile".into(),
        call: WorkflowV2HostCall {
            id: id.into(),
            method: WorkflowV2HostMethod::HostCommand,
            write_mode: None,
            options: WorkflowV2HostOptions {
                host_command: Some(HostCommandRequest {
                    command_id: "land-task-body".into(),
                    stdin: Some(candidate(round)),
                    target: Some(HostCommandSubject {
                        task_id: TASK.into(),
                        file_name: FILE.into(),
                    }),
                }),
                ..WorkflowV2HostOptions::default()
            },
        },
        attempt: 1,
        schema_version: "1".into(),
        admission_sequence: None,
        started_at: at.into(),
        finished_at: at.into(),
        input_hash: "in".into(),
        output_hash: "out".into(),
        status: if refused {
            WorkflowV2Status::NeedsReview
        } else {
            WorkflowV2Status::Accepted
        },
        result: WorkflowV2Result {
            data: serde_json::json!({
                "gateEnvelope": {"policy_findings": findings, "operational_error": null},
                "publicationReceipt": receipt,
            }),
            ..WorkflowV2Result::default()
        },
        depends_on: Vec::new(),
        invalidated_by: None,
        agent_session_id: None,
        agent_session_ids: Vec::new(),
        source_fingerprint: None,
        source_task_graph: None,
        completed_ids: Vec::new(),
        scaffold_hash: None,
        completion_evidence: Vec::new(),
        evidence_snapshot_hash: None,
        dispatched_items: Vec::new(),
        answered_by: None,
    }
}

/// The bytes the landing of `round` published.
fn landed(round: &str) -> Vec<u8> {
    let shell = record("shell", "t0", round, false, serde_json::Value::Null);
    rebuild(&shell, &frozen(), Path::new(FILE)).unwrap()
}

/// Round 1 refused, round 2 accepted, then round 1 asked again and
/// published over round 2: the R8 shape.
fn damaged() -> Vec<WorkflowV2CallRecord> {
    let digest = |round: &str| content_digest(&landed(round));
    vec![
        landing("r1", "t1", "round-1", true, &digest("round-1")),
        landing("r2", "t2", "round-2", false, &digest("round-2")),
        landing("r1-again", "t3", "round-1", true, &digest("round-1")),
    ]
}

#[test]
fn a_body_a_rerun_overwrote_is_restored_to_its_current_landing() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join(FILE), landed("round-1")).unwrap();
    let plan = plan(&damaged(), temp.path(), &skeleton()).unwrap();
    assert_eq!(plan.restores.len(), 1, "{:?}", plan.lines);
    let restore = &plan.restores[0];
    assert_eq!(restore.call_id, "r2");
    assert_eq!(restore.bytes, landed("round-2"));
    assert_eq!(restore.receipt_blake3, content_digest(&landed("round-2")));
    assert!(plan.lines[0].contains("SETTLE"), "{}", plan.lines[0]);
}

#[test]
fn a_live_body_that_is_its_current_landing_is_left_alone() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join(FILE), landed("round-2")).unwrap();
    let plan = plan(&damaged(), temp.path(), &skeleton()).unwrap();
    assert!(plan.restores.is_empty());
    assert!(plan.lines[0].contains("matches its current landing"));
}

#[test]
fn a_missing_body_is_restored() {
    let temp = tempfile::tempdir().unwrap();
    let plan = plan(&damaged(), temp.path(), &skeleton()).unwrap();
    assert_eq!(plan.restores.len(), 1);
    assert_eq!(plan.restores[0].live_blake3, None);
}

#[test]
fn an_unrebuildable_receipt_leaves_existing_bytes_as_a_candidate() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join(FILE), landed("round-1")).unwrap();
    let mut records = damaged();
    // The receipt names bytes no stamping of its candidate gives.
    records[1].result.data["publicationReceipt"]["entries"][0]["blake3"] =
        serde_json::json!("0".repeat(64));
    let candidate_bytes = b"outside candidate";
    std::fs::write(temp.path().join(FILE), candidate_bytes).unwrap();
    let plan = plan(&records, temp.path(), &skeleton()).unwrap();
    assert!(plan.restores.is_empty());
    assert!(plan.lines.iter().any(|line| line.contains("candidate")));
    assert_eq!(
        std::fs::read(temp.path().join(FILE)).unwrap(),
        candidate_bytes
    );
}

#[test]
fn an_unrebuildable_missing_body_routes_to_the_author_path() {
    let temp = tempfile::tempdir().unwrap();
    let mut records = damaged();
    records[1].result.data["publicationReceipt"]["entries"][0]["blake3"] =
        serde_json::json!("0".repeat(64));
    let plan = plan(&records, temp.path(), &skeleton()).unwrap();
    assert!(plan.restores.is_empty());
    assert!(
        plan.lines
            .iter()
            .any(|line| line.contains("route to the normal author path"))
    );
    assert!(!temp.path().join(FILE).exists());
}

#[test]
fn a_task_with_no_accepted_landing_is_left_as_it_is() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join(FILE), b"operator text").unwrap();
    let records = vec![landing(
        "r1",
        "t1",
        "round-1",
        true,
        &content_digest(&landed("round-1")),
    )];
    let plan = plan(&records, temp.path(), &skeleton()).unwrap();
    assert!(plan.restores.is_empty());
    assert!(plan.lines[0].contains("no accepted landing"));
}

#[test]
fn a_restore_is_written_atomically_and_hashes_to_its_receipt() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path();
    let task_root = project.join("tasks").join("PRD-X");
    std::fs::create_dir_all(&task_root).unwrap();
    std::fs::write(task_root.join(FILE), landed("round-1")).unwrap();
    let pin = crate::command::workflow_task_set::acceptance_pin_path(project, &task_root);
    std::fs::create_dir_all(pin.parent().unwrap()).unwrap();
    let plan = plan(&damaged(), &task_root, &skeleton()).unwrap();
    apply(&pin, &task_root, &plan).unwrap();
    assert_eq!(
        std::fs::read(task_root.join(FILE)).unwrap(),
        landed("round-2")
    );
}

#[test]
fn a_live_body_no_landing_published_is_left_as_a_candidate() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join(FILE), b"edited by hand during the pause").unwrap();
    let plan = plan(&damaged(), temp.path(), &skeleton()).unwrap();
    assert!(plan.restores.is_empty());
    assert!(
        plan.lines
            .iter()
            .any(|line| line.contains("fresh candidate"))
    );
    assert_eq!(
        std::fs::read(temp.path().join(FILE)).unwrap(),
        b"edited by hand during the pause"
    );
}

/// The lineage reads the occurrence the host stamps under its own key.
#[test]
fn the_lineage_reads_the_hosts_occurrence_key() {
    assert_eq!(
        archon_workflow::v2::script::landing_lineage::OCCURRENCE_KEY,
        crate::command::workflow_host_command_occurrence::OCCURRENCE_KEY
    );
}

/// A stop between publication and record save leaves a newer landing's
/// body on disk with no receipt: it is the run's own, left for the resume.
#[test]
fn a_newer_body_published_without_a_receipt_is_left_for_the_resume() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join(FILE), landed("round-3")).unwrap();
    let mut records = damaged();
    records.push(record(
        "r3",
        "t4",
        "round-3",
        false,
        serde_json::Value::Null,
    ));
    let plan = plan(&records, temp.path(), &skeleton()).unwrap();
    assert!(plan.restores.is_empty());
    assert!(
        plan.lines[0].contains("candidate of call r3"),
        "{}",
        plan.lines[0]
    );
    // The same body from a landing older than the current one is a
    // regression and is put back.
    let mut records = damaged();
    records.push(record("r0", "t0", "round-3", true, serde_json::Value::Null));
    let older = super::plan(&records, temp.path(), &skeleton()).unwrap();
    assert_eq!(older.restores.len(), 1);
}

#[test]
fn reconcile_takes_the_chain_lock_before_planning_and_defers_to_live_settle() {
    let temp = tempfile::tempdir().unwrap();
    let context = crate::command::workflow_host_command_exec_tests::context(temp.path());
    let project = context.project_root.clone();
    let task_root = context.task_root.clone();
    let task_path = task_root.join(FILE);
    let prd_path = project.join("prds/PRD-X.md");
    std::fs::create_dir_all(prd_path.parent().unwrap()).unwrap();
    std::fs::write(
        &prd_path,
        "# PRD X\n\n## Acceptance Criteria\n\n| ID | Criterion |\n|---|---|\n| AC-X-001 | The fixture is valid. |\n",
    )
    .unwrap();
    let acceptance_digest =
        crate::command::workflow_decompose_frozen_chain::test_support::freeze_acceptance(
            &project, &prd_path, &task_root,
        );
    crate::command::workflow_decompose_frozen_chain::test_support::freeze_skeleton(
        &project,
        &task_root,
        &acceptance_digest,
        &[TASK],
    );
    std::fs::write(&task_path, landed("round-1")).unwrap();
    let pin = crate::command::workflow_task_set::acceptance_pin_path(&project, &task_root);
    let held = crate::command::workflow_task_set::ChainLock::acquire(&pin, &task_root).unwrap();
    let run_dir = temp.path().join("run");
    let store = archon_workflow::WorkflowV2ResultStore::new(run_dir.join("v2"));
    let mut initial = damaged();
    for record in &initial {
        store.save_call_record(record).unwrap();
    }

    let project_for_thread = project.clone();
    let task_root_for_thread = task_root.clone();
    let run_dir_for_thread = run_dir.clone();
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        started_tx.send(()).unwrap();
        reconcile_task_root(
            &project_for_thread,
            &run_dir_for_thread,
            &task_root_for_thread,
            true,
        )
        .unwrap()
    });
    started_rx.recv().unwrap();
    std::thread::sleep(std::time::Duration::from_millis(150));

    // A publisher completes a newer landing while reconciliation is waiting
    // for this same chain lock. Reconciliation must plan from this state.
    std::fs::write(&task_path, landed("round-3")).unwrap();
    initial.push(landing(
        "r3",
        "t4",
        "round-3",
        false,
        &content_digest(&landed("round-3")),
    ));
    store.save_call_record(initial.last().unwrap()).unwrap();
    drop(held);

    let lines = worker.join().unwrap();
    let actual = String::from_utf8(std::fs::read(&task_path).unwrap()).unwrap();
    let expected = String::from_utf8(landed("round-3")).unwrap();
    assert_eq!(
        actual, expected,
        "the completed newer landing must remain on disk"
    );
    assert!(lines[0].contains("queued 0 of 1"), "{lines:?}");
}
