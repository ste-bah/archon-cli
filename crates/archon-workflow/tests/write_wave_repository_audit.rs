//! Write-wave repository-audit gate: the pre-apply and post-apply audit
//! checks driven through the production seam.
use archon_workflow::*;
use serde_json::json;
use std::{path::PathBuf, sync::Mutex, time::Duration};

#[path = "support/write_wave_seam.rs"]
mod seam;
use seam::*;

#[tokio::test]
async fn repository_audit_duplicate_is_rejected_before_apply_without_expanding_scope() {
    use archon_workflow::repository_audit::{
        AuditContract, AuditReport,
        budget::{AuditPolicy, Limit},
        runtime::AuditRuntime,
    };
    let f = Fixture::new();
    let audit = AuditRuntime::initialize(
        f.store.clone(),
        f.run.clone(),
        AuditPolicy {
            attempt_timeout_secs: Limit::Finite(60),
            total_time_secs: Limit::Unlimited,
            unexpected_change_refreshes: Limit::Unlimited,
        },
    )
    .unwrap();
    let contract = AuditContract {
        schema_version: 1,
        snapshot: "fixture".into(),
        declared_paths: vec!["added.txt".into()],
    };
    let report: AuditReport =
        serde_json::from_value(json!({"schema_version":1,"snapshot":"fixture","records":[{
        "declared_path":"added.txt","verdict":"exists_elsewhere","equivalents":["owned.txt"],
        "required_action":"wire_or_migrate","reason":"existing behavior"}]}))
        .unwrap();
    audit.update(|s| s.ledger.accept(contract, report)).unwrap();
    let (out, dispatch) = f.wave("audit-duplicate", Reply::Accepted).await;
    let prompts = dispatch.prompts.lock().unwrap();
    assert!(
        prompts[0].contains("Host repository audit") && prompts[0].contains("wire_or_migrate"),
        "audit injection is disconnected"
    );
    assert_ne!(
        out.status,
        WorkflowV2Status::Accepted,
        "audit obligation ignored: {out:#?}"
    );
    assert_eq!(
        git(&f.repo, &["rev-parse", "HEAD"]),
        f.base,
        "unexplained duplicate applied"
    );
}

#[path = "support/write_wave_audit_cache.rs"]
mod audit_cache;

#[tokio::test]
async fn repository_audit_snapshot_waiver_is_honored_by_preapply_gate() {
    use archon_workflow::repository_audit::{
        AuditContract, AuditReport,
        budget::{AuditPolicy, Limit},
        ledger::Waiver,
        runtime::{AuditRuntime, Snapshot},
    };
    let fixture = Fixture::new();
    let audit = AuditRuntime::initialize(
        fixture.store.clone(),
        fixture.run.clone(),
        AuditPolicy {
            attempt_timeout_secs: Limit::Unlimited,
            total_time_secs: Limit::Unlimited,
            unexpected_change_refreshes: Limit::Unlimited,
        },
    )
    .unwrap();
    audit.update(|state| {
        state.declared_paths.insert("added.txt".into());
        state.snapshot=Some(Snapshot{identity:"one".into(),root:fixture.repo.clone(),paths:vec!["owned.txt".into()]});
        let report:AuditReport=serde_json::from_value(json!({"schema_version":1,"snapshot":"one","records":[{
            "declared_path":"added.txt","verdict":"exists_elsewhere","equivalents":["owned.txt"],"required_action":"wire_or_migrate","reason":"equivalent implementation"
        }]})).unwrap();
        state.ledger.accept(AuditContract{schema_version:1,snapshot:"one".into(),declared_paths:vec!["added.txt".into()]},report)?;
        state.ledger.waivers.push(Waiver{declared_path:"added.txt".into(),snapshot:"one".into(),action_id:"human-confirmed".into(),reason:"accepted exception".into(),assessment_count:1});
        Ok(())
    }).unwrap();
    let (result, dispatch) = fixture.wave("waived", Reply::Accepted).await;
    assert_eq!(
        result.status,
        WorkflowV2Status::Accepted,
        "confirmed exception ignored by preapply: {result:#?}"
    );
    assert!(dispatch.prompts.lock().unwrap()[0].contains("operator_waived"));
    assert_eq!(
        audit.state().unwrap().ledger.history[0].records[0].required_action,
        archon_workflow::repository_audit::RequiredAction::WireOrMigrate
    );
}
