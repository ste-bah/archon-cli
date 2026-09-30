//! A repository audit's findings and receipts decide terminal success.

use super::*;

#[tokio::test]
async fn repository_audit_open_finding_prevents_terminal_success() {
    use archon_workflow::repository_audit::{
        AuditContract, AuditReport,
        budget::{AuditPolicy, Limit},
        runtime::{AuditRuntime, Snapshot},
    };
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let run = store.create_run(spec()).unwrap();
    let v2 = WorkflowV2ResultStore::new(store.run_dir(&run.id).join("v2"));
    seed_call(&v2, WorkflowV2Status::Accepted);
    let audit = AuditRuntime::initialize(
        store.clone(),
        run.id.clone(),
        AuditPolicy {
            attempt_timeout_secs: Limit::Unlimited,
            total_time_secs: Limit::Unlimited,
            unexpected_change_refreshes: Limit::Unlimited,
        },
    )
    .unwrap();
    audit.update(|s| {
        s.snapshot = Some(Snapshot{identity:"sealed".into(),root:temp.path().into(),paths:vec![]});
        let report: AuditReport = serde_json::from_value(serde_json::json!({"schema_version":1,"snapshot":"sealed","records":[{
            "declared_path":"new.txt","verdict":"exists_elsewhere","equivalents":["old.txt"],"required_action":"wire_or_migrate","reason":"existing implementation"
        }]})).unwrap();
        s.ledger.accept(AuditContract{schema_version:1,snapshot:"sealed".into(),declared_paths:vec!["new.txt".into()]},report)
    }).unwrap();
    finalize_summary(
        &store,
        &run.id,
        WorkflowRunKind::FixedOrSavedScript,
        None,
        &summary(WorkflowV2Status::Accepted),
        &v2,
        None,
        None,
    )
    .await
    .unwrap();
    assert_eq!(store.load_state(&run.id).unwrap().status, RunStatus::Failed);
    assert!(
        !events(&store, &run.id)
            .iter()
            .any(|e| e.detail["event"] == "terminal_status" && e.detail["status"] == "accepted")
    );
}

#[tokio::test]
async fn repository_audit_clean_dispatch_is_not_a_final_assessment_receipt() {
    use archon_workflow::repository_audit::{
        AuditContract, AuditReport,
        budget::{AuditPolicy, Limit},
        runtime::{AuditRuntime, Snapshot},
    };
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let run = store.create_run(spec()).unwrap();
    let v2 = WorkflowV2ResultStore::new(store.run_dir(&run.id).join("v2"));
    seed_call(&v2, WorkflowV2Status::Accepted);
    let audit = AuditRuntime::initialize(
        store.clone(),
        run.id.clone(),
        AuditPolicy {
            attempt_timeout_secs: Limit::Unlimited,
            total_time_secs: Limit::Unlimited,
            unexpected_change_refreshes: Limit::Unlimited,
        },
    )
    .unwrap();
    audit
        .update(|state| {
            state.snapshot = Some(Snapshot {
                identity: "dispatch".into(),
                root: temp.path().into(),
                paths: vec![],
            });
            state.ledger.accept(
                AuditContract {
                    schema_version: 1,
                    snapshot: "dispatch".into(),
                    declared_paths: vec![],
                },
                AuditReport {
                    schema_version: 1,
                    snapshot: "dispatch".into(),
                    records: vec![],
                },
            )
        })
        .unwrap();
    finalize_summary(
        &store,
        &run.id,
        WorkflowRunKind::FixedOrSavedScript,
        None,
        &summary(WorkflowV2Status::Accepted),
        &v2,
        None,
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        store.load_state(&run.id).unwrap().status,
        RunStatus::Failed,
        "an earlier clean assessment was promoted to final acceptance without a final snapshot receipt"
    );
}
