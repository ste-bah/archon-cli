//! Evidence loss BEFORE re-freeze must refuse ordinary publication, by name,
//! and the run then stops resumable for review, never failed (Issue 305).
use super::super::repair_tests::{Run, run_fixture_with};
use super::round_six::{metadata, recover};
use super::*;

/// How the pending recovery evidence is damaged, and the refusal it names.
#[derive(Clone, Copy, Debug)]
enum Damage {
    MissingLog,
    EmptyLog,
    UnboundLog,
    ForeignTaskRoot,
}

impl Damage {
    fn named(self) -> &'static str {
        match self {
            Self::MissingLog => "is missing; restore the recovery evidence",
            Self::EmptyLog | Self::UnboundLog => "not bound to a durable unfreeze log entry",
            Self::ForeignTaskRoot => "names task root",
        }
    }
}

fn damaged(damage: Damage) -> Run {
    let run = run_fixture_with(&[("AC-F-001", "test -f present", true)]);
    let launch = run.set.pin().identity();
    let metadata_path = metadata(&run, &run.run_id, &launch);
    let mut value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&metadata_path).unwrap()).unwrap();
    value["schema_version"] = serde_json::json!("workflow-generated-v2-metadata-v1");
    std::fs::write(metadata_path, serde_json::to_vec(&value).unwrap()).unwrap();
    recover(&run);
    let log = run.set.pin_path().with_extension("publish-recovery.log");
    match damage {
        Damage::MissingLog => std::fs::remove_file(&log).unwrap(),
        Damage::EmptyLog => std::fs::write(&log, b"").unwrap(),
        Damage::UnboundLog => std::fs::write(
            &log,
            b"{\"event\":\"task_set_publish_recovered\",\"authority\":{}}\n",
        )
        .unwrap(),
        Damage::ForeignTaskRoot => {
            // The only pending record names another task root. Before
            // round 4 a task-root filter skipped it into publication.
            let path =
                crate::command::workflow_task_set::recovery_lineage::path(&run.set.pin_path());
            let mut records: serde_json::Value =
                serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            records[0]["task_root"] = serde_json::json!("/elsewhere/tasks");
            std::fs::write(path, serde_json::to_vec(&records).unwrap()).unwrap();
        }
    }
    run
}

fn chain_files(run: &Run) -> Vec<std::path::PathBuf> {
    use archon_workflow::task_set_contract::*;
    vec![
        run.set.pin_path(),
        run.set.tasks.join(ACCEPTANCE_CONTRACT_FILE),
        run.set.tasks.join(ACCEPTANCE_LOCK_FILE),
        run.set.tasks.join(TASK_SKELETON_FILE),
        run.set.tasks.join(TASK_SKELETON_LOCK_FILE),
    ]
}

fn pending_evidence(damage: Damage) {
    let run = damaged(damage);
    let files = chain_files(&run);
    let before = files.iter().map(std::fs::read).collect::<Vec<_>>();
    let context = super::super::super::exec::resolve_context(
        &run.store,
        &run.run_id,
        run.runtime.target_repository_root.as_deref(),
        Some(&run.universe),
    )
    .unwrap();
    let result = super::super::super::author::publish_fresh(
        &context,
        &run.set.prd,
        &run.set.contract(),
        None,
    );
    let error = result
        .err()
        .expect("pending recovery without authenticated evidence must refuse before publication")
        .error;
    assert!(error.contains(damage.named()), "{damage:?}: {error}");
    let after = files.iter().map(std::fs::read).collect::<Vec<_>>();
    for (before, after) in before.into_iter().zip(after) {
        assert_eq!(before.ok(), after.ok(), "live chain bytes must not change");
    }
}
#[test]
fn round3_305_missing_pending_recovery_log() {
    pending_evidence(Damage::MissingLog);
}
#[test]
fn round3_305_empty_pending_recovery_log() {
    pending_evidence(Damage::EmptyLog);
}
#[test]
fn round3_305_unbound_pending_recovery_log() {
    pending_evidence(Damage::UnboundLog);
}
#[test]
fn round4_305_pending_record_of_another_task_root_refuses_by_name() {
    pending_evidence(Damage::ForeignTaskRoot);
}

/// The acceptance stage meets the refusal as an operational error; the run
/// end holds the run to review with the evidence remedy, and it is not failed.
async fn pending_run_outcome(damage: Damage) {
    let run = damaged(damage);
    let (_result, record) = stage(&run, &accepting()).await;
    assert!(record.blocks_completion(), "{damage:?}");
    assert!(
        record
            .operational_errors
            .iter()
            .any(|error| error.contains(damage.named())),
        "{damage:?}: the refusal is named: {:?}",
        record.operational_errors
    );
    let v2 = archon_workflow::WorkflowV2ResultStore::new(run.store.run_dir(&run.run_id).join("v2"));
    let summary = crate::command::workflow_live::workflow_live_v2::workflow_live_v2_script::WorkflowV2ScriptSummary {
        status: WorkflowV2Status::Accepted,
        completed: 1,
        executed: 1,
        reused: 0,
        calls: Vec::new(),
        failed_call: None,
        failed_result_path: None,
        next_action: None,
        script_error: None,
        script_result: None,
    };
    let (gated, _) = super::super::super::super::workflow_live_v3_run_end::apply_acceptance_gate(
        &run.store,
        &run.run_id,
        &v2,
        summary,
    )
    .unwrap();
    assert_eq!(gated.status, WorkflowV2Status::NeedsReview, "{damage:?}");
    let next = gated.next_action.expect("a remedy");
    assert!(next.contains(damage.named()), "{next}");
    assert!(next.contains("/workflow resume --live"), "{next}");
    assert!(
        !next.contains("fix the named tasks' implementation"),
        "missing evidence is not a task defect: {next}"
    );
    assert_ne!(
        run.store.load_state(&run.run_id).unwrap().status,
        archon_workflow::RunStatus::Failed,
        "the run stays resumable"
    );
}
#[tokio::test]
async fn round4_305_missing_log_holds_the_run_for_review() {
    pending_run_outcome(Damage::MissingLog).await;
}
#[tokio::test]
async fn round4_305_unbound_log_holds_the_run_for_review() {
    pending_run_outcome(Damage::UnboundLog).await;
}
#[tokio::test]
async fn round4_305_foreign_task_root_holds_the_run_for_review() {
    pending_run_outcome(Damage::ForeignTaskRoot).await;
}
