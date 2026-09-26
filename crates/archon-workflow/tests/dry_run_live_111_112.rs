//! A dry run of the Issue-111 and Issue-112 host rules against a COPY of a
//! live run's records and the live target repository (read-only git).
//! Ignored by default; run with
//!   ARCHON_DRY_RUN_RUN=<copied run dir> ARCHON_DRY_RUN_REPO=<repository>
//!   cargo test -p archon-workflow --test dry_run_live_111_112 -- --ignored --nocapture
//! It answers what a resume would do with each 012-unit call and what the
//! audit concludes about the shared deliverable, and writes nothing to the
//! repository.

use std::path::PathBuf;

use archon_workflow::repository_audit::runtime::AuditState;
use archon_workflow::task_universe::WorkflowV2TaskUniverse;
use archon_workflow::v2::branch_cache::landing::landing_holds;
use archon_workflow::v2::branch_cache::replayed_fix;
use archon_workflow::v2::script::remediation_escalation::{
    buys_escalation, escalation_refusal, reverify_plan,
};
use archon_workflow::v2::script::resume_drift::superseded_remediation_record;
use archon_workflow::v2::script::resume_verdict::{
    is_remediation_fix, is_remediation_verdict, remediation_round_key,
    verdict_vouches_for_session_fix,
};
use archon_workflow::write_coordinator::PatchManifest;
use archon_workflow::*;
use serde_json::json;

fn env(name: &str) -> Option<PathBuf> {
    std::env::var_os(name).map(PathBuf::from)
}

fn manifest(run: &std::path::Path, stage: &str) -> Option<PatchManifest> {
    let dir = run
        .join("write-coordination/stages")
        .join(stage)
        .join("manifests");
    let entry = std::fs::read_dir(dir).ok()?.flatten().next()?;
    serde_json::from_slice(&std::fs::read(entry.path()).ok()?).ok()
}

#[test]
#[ignore = "needs a copied live run: ARCHON_DRY_RUN_RUN and ARCHON_DRY_RUN_REPO"]
fn dry_run_the_live_records() {
    let (Some(run), Some(repo)) = (env("ARCHON_DRY_RUN_RUN"), env("ARCHON_DRY_RUN_REPO")) else {
        eprintln!("ARCHON_DRY_RUN_RUN / ARCHON_DRY_RUN_REPO unset; nothing to do");
        return;
    };
    let unit = std::env::var("ARCHON_DRY_RUN_UNIT").unwrap_or_else(|_| "task-trading-012".into());
    let metadata: serde_json::Value =
        serde_json::from_slice(&std::fs::read(run.join("v2/generated-metadata.json")).unwrap())
            .unwrap();
    let universe: WorkflowV2TaskUniverse =
        serde_json::from_value(metadata["task_universe"].clone()).unwrap();
    // A new session over the records: what a resume starts from.
    let store = WorkflowV2ResultStore::new(run.join("v2"));
    let records = store.load_call_records().unwrap();
    let ordinal = |id: &str| {
        id.rsplit('-')
            .next()
            .and_then(|n| n.parse::<u64>().ok())
            .unwrap_or(0)
    };
    // The unit's calls of the latest review pass, in ordinal order: the ids
    // the deployed prelude issued (ordinals 47..51 for the live 012 unit).
    let mut calls: Vec<&WorkflowV2CallRecord> = records
        .iter()
        .filter(|r| r.call.id.contains(&unit) && r.call.id.contains("review-"))
        .filter(|r| ordinal(&r.call.id) >= 40 && ordinal(&r.call.id) < 60)
        .collect();
    calls.sort_by_key(|r| ordinal(&r.call.id));
    println!("== Issue-111: unit {unit}, a resumed session");
    for record in &calls {
        let id = &record.call.id;
        if is_remediation_fix(&record.call) {
            let holds = manifest(&run, id).map(|m| landing_holds(&repo, &m));
            let lineage = replayed_fix(&store, record);
            if let Some(key) = remediation_round_key(&record.call) {
                store.note_fix_lineage(&key, lineage.clone());
            }
            store.note_session_call(id);
            println!(
                "fix     {id}: status={:?} attempt={} record_finish={} landing_holds={:?} replayed_execution_finish={:?}",
                record.status,
                record.attempt,
                record.finished_at,
                holds.map(|h| h.map_err(|e| e.chars().take(120).collect::<String>())),
                lineage.map(|l| l.finished_at)
            );
            if let Some(plan) = reverify_plan(record, &store, Some(&universe), Some(&repo)) {
                println!("        REVERIFY PLAN: {plan:#}");
            }
        } else if is_remediation_verdict(&record.call) {
            let vouches = verdict_vouches_for_session_fix(record, &records, &store);
            let history = superseded_remediation_record(record, &records)
                || buys_escalation(record, Some(&universe), Some(&repo));
            let replays = vouches
                && (history
                    || matches!(
                        record.status,
                        WorkflowV2Status::Accepted | WorkflowV2Status::Noop
                    ));
            store.note_session_call(id);
            println!(
                "verdict {id}: status={:?} finish={} vouches={vouches} history={history} => {}",
                record.status,
                record.finished_at,
                if replays { "REPLAYS" } else { "RUNS AGAIN" }
            );
        }
    }
    // The escalated round's no-patch checkpoint, as the prelude files it.
    if let Some(esc) = calls
        .iter()
        .find(|r| r.call.id.contains("-esc-") && is_remediation_fix(&r.call))
    {
        let mut contract = esc.call.options.extra["remediationContract"].clone();
        contract["stage"] = json!("verify");
        let mut options = WorkflowV2HostOptions::default();
        options.extra.insert("remediationContract".into(), contract);
        let checkpoint = WorkflowV2CallExecution {
            call: WorkflowV2HostCall {
                id: format!("review-verify-{unit}-3-no-patch"),
                method: WorkflowV2HostMethod::Checkpoint,
                write_mode: None,
                options,
            },
            input: json!({"options": {}}),
            depends_on: vec![],
        };
        println!(
            "checkpoint {}: escalation refusal = {:?}",
            checkpoint.call.id,
            escalation_refusal(&checkpoint, &store, Some(&universe), Some(&repo))
        );
    }

    println!("== Issue-112: the repository audit at its last snapshot");
    let state: AuditState =
        serde_json::from_slice(&std::fs::read(run.join("v2/repository-audit/state.json")).unwrap())
            .unwrap();
    let mut state = state;
    let snapshot = state.ledger.history.last().unwrap().snapshot.clone();
    println!(
        "before: unresolved={:?} discharges={} contests={}",
        state.ledger.unresolved(&snapshot).unwrap(),
        state.ledger.discharges.len(),
        state.ledger.contests.len()
    );
    state.rejudge(&run, &repo);
    println!(
        "after rejudge: unresolved={:?}",
        state.ledger.describe_unresolved(&snapshot).unwrap()
    );
    for contest in state.ledger.contested(&snapshot) {
        println!("contest: {contest:#?}");
    }
    let open = state.ledger.unresolved(&snapshot).unwrap();
    for path in &open {
        println!(
            "cache admission of a write targeting {path}: {:?}",
            archon_workflow::repository_audit::reuse::admits(&state, std::slice::from_ref(path))
        );
    }
}
