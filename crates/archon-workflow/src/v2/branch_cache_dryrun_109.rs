//! Issue-109 dry run: which review-remediation calls the next resumes of a
//! live run would replay or run, against a COPY of its records (they are
//! written: each dry session saves what the host would) and the live target
//! repository (read only). Ignored by default; run with
//!   ARCHON_DRY_RUN_RUN=<copied run dir> ARCHON_DRY_RUN_REPO=<repository>
//!   cargo test -p archon-workflow --lib dry_run_109 -- --ignored --nocapture
//!
//! The calls are the ones the run's latest session answered, in its order.
//! A fix's branch is judged by the production checks (`tree_holds_landing`,
//! `landing_receipt_holds`, `refiled`, `note_fix_lineage`); its item is
//! rebuilt from the record, so its identity is taken as the one its own
//! outcome was filed under -- the resumed prelude issues the same item.

use std::path::{Path, PathBuf};

use super::*;
use crate::v2::script::resume_drift::superseded_remediation_record;
use crate::v2::script::resume_verdict::{
    is_remediation_fix, is_remediation_verdict, verdict_vouches_for_session_fix,
};

fn env(name: &str) -> Option<PathBuf> {
    std::env::var_os(name).map(PathBuf::from)
}

/// The review-remediation calls the latest session answered, in order.
fn latest_session_calls(run: &Path) -> Vec<String> {
    let events = std::fs::read_to_string(run.join("events.jsonl")).expect("events");
    let lines: Vec<&str> = events.lines().collect();
    let resumed = lines
        .iter()
        .rposition(|line| line.contains("\"kind\":\"resumed\""))
        .unwrap_or(0);
    let mut calls = Vec::new();
    for line in &lines[resumed..] {
        let Ok(event) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let Some(id) = event["detail"]["call_id"].as_str() else {
            continue;
        };
        let review = id.starts_with("review-remediate-")
            || id.starts_with("verification-wave-review-verify-");
        if review && !calls.iter().any(|call| call == id) {
            calls.push(id.to_string());
        }
    }
    calls
}

fn item(record: &WorkflowV2CallRecord, repo: &Path) -> WorkflowV2FanoutItem {
    let mut call = record.call.clone();
    call.id = format!("{}-0", record.call.id);
    WorkflowV2FanoutItem::read_only(
        call.id.clone(),
        "coder",
        call,
        serde_json::json!({ "item": { "target_repository_root": repo.display().to_string() } }),
    )
}

/// The host's next attempt of `record`, recorded now.
fn resaved(record: &WorkflowV2CallRecord) -> WorkflowV2CallRecord {
    let mut next = record.clone();
    next.attempt += 1;
    next.started_at = chrono::Utc::now().to_rfc3339();
    next.finished_at = next.started_at.clone();
    next.answered_by = None;
    next
}

/// How this session answers the fix `record`: its own record, a drifted
/// sibling's refile, or a run. Saves what the host would.
fn answer_fix(v2: &WorkflowV2ResultStore, record: &WorkflowV2CallRecord, repo: &Path) -> String {
    let call_id = record.call.id.as_str();
    let records = v2.load_call_records().expect("records");
    let item = item(record, repo);
    let label_written = lineage::label_last_written(v2, call_id, &record.call);
    let persisted = v2.load_branch_outcome(call_id, &item.id).expect("outcome");
    let own = persisted.as_ref().is_some_and(|outcome| {
        (reusable_branch_outcome(outcome)
            || (history_eligible(outcome) && superseded_remediation_record(record, &records)))
            && tree_holds_landing(v2, call_id, outcome, &item)
    });
    let (mut sources, mut refiled_from, mut how) = (vec![], vec![], "RUNS".to_string());
    if own {
        sources.push(call_id.to_string());
        how = "own record".into();
    } else if let Some((token, own_ordinal)) = ordinal_token(call_id) {
        for sibling in drift_candidates(call_id, &records, |id| v2.in_session(id)) {
            let Some((_, ordinal)) = ordinal_token(&sibling.call.id) else {
                continue;
            };
            if !same_remediation_contract(&sibling.call, &record.call) {
                continue;
            }
            let branch = rebase_text(&item.id, token, own_ordinal, ordinal);
            let Some(outcome) = v2.load_branch_outcome(&sibling.call.id, &branch).unwrap() else {
                continue;
            };
            if !reusable_branch_outcome(&outcome)
                || !landing_receipt_holds(v2, &item, &sibling.call.id, &outcome)
            {
                continue;
            }
            let mut again = refiled(&outcome, &item, call_id, &sibling.call.id);
            again.item_input_hash = persisted.as_ref().and_then(|o| o.item_input_hash.clone());
            v2.note_session_call(&sibling.call.id);
            if v2.filed_unchanged(persisted.as_ref(), &again) {
                sources.push(call_id.to_string());
                refiled_from.push(sibling.call.id.clone());
                how = format!("refile of {} as already filed", sibling.call.id);
            } else {
                v2.save_branch_outcome(call_id, &again).expect("refile");
                sources.push(sibling.call.id.clone());
                how = format!("refile of {}", sibling.call.id);
            }
            break;
        }
    }
    let answered = Replayed {
        sources: &sources,
        refiled_from: &refiled_from,
        none_pending: !sources.is_empty(),
    };
    note_fix_lineage(v2, call_id, &record.call, answered, label_written);
    v2.save_call_record(&resaved(record)).expect("fix record");
    let key = crate::v2::script::resume_verdict::remediation_round_key(&record.call).unwrap();
    // The rebuilt item's identity is the filed one; a changed identity would
    // have archived the earlier outcome under `superseded/`.
    let branch_dir = v2.branch_outcome_path(call_id, &item.id);
    let superseded = branch_dir.with_file_name("superseded").exists();
    format!(
        "{how}; identity ever changed: {superseded}; verdicts pair with {:?}",
        v2.fix_replayed(&key)
    )
}

/// One dry session over `calls`: what each would do.
fn session(v2: &WorkflowV2ResultStore, calls: &[String], repo: &Path) -> Vec<String> {
    let mut report = Vec::new();
    for id in calls {
        let records = v2.load_call_records().expect("records");
        let Some(record) = records.iter().find(|record| &record.call.id == id) else {
            report.push(format!("??      {id}: no record"));
            continue;
        };
        if is_remediation_fix(&record.call) {
            let how = answer_fix(v2, record, repo);
            report.push(format!("fix     {id}: {how}"));
        } else if is_remediation_verdict(&record.call) {
            let vouches = verdict_vouches_for_session_fix(record, &records, v2);
            if vouches {
                v2.note_session_call(id);
            } else {
                let mut fresh = resaved(record);
                fresh.status = WorkflowV2Status::Accepted;
                v2.save_call_record(&fresh).expect("verdict record");
            }
            let verdict = if vouches { "REPLAYS" } else { "RUNS AGAIN" };
            report.push(format!("verdict {id}: {verdict}"));
        }
    }
    report
}

#[test]
#[ignore = "needs a copied live run: ARCHON_DRY_RUN_RUN and ARCHON_DRY_RUN_REPO"]
fn dry_run_109_next_resumes() {
    let (Some(run), Some(repo)) = (env("ARCHON_DRY_RUN_RUN"), env("ARCHON_DRY_RUN_REPO")) else {
        eprintln!("ARCHON_DRY_RUN_RUN / ARCHON_DRY_RUN_REPO unset; nothing to do");
        return;
    };
    let calls = latest_session_calls(&run);
    for resume in 1..=2 {
        println!("== next resume {resume}");
        let v2 = WorkflowV2ResultStore::new(run.join("v2"));
        for line in session(&v2, &calls, &repo) {
            println!("{line}");
        }
    }
}
