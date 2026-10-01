//! The scripted verifier answers of the escalation harness.
use archon_workflow::*;
use serde_json::{Value, json};

use super::{Verdict, payload_item};

/// A verifier branch's answer, in the shape the verification wave records.
pub(super) fn verdict_result(
    execution: &WorkflowV2CallExecution,
    verdict: &Verdict,
    judged: &str,
) -> WorkflowV2Result {
    let item = payload_item(execution);
    // A disposition naming a file is a plain disposition whose open ids'
    // evidence names the file.
    let (owned, naming) = match verdict {
        Verdict::DisposeNaming(said, file) => (Verdict::Dispose(said.clone()), Some(*file)),
        other => (other.clone(), None),
    };
    let verdict = &owned;
    let gaps: Vec<Value> = match verdict {
        Verdict::AcceptWith(gaps) | Verdict::AcceptDisposing(gaps, _) | Verdict::RefuseWith(gaps) => gaps
            .iter()
            .map(|(id, severity, description)| json!({"id": id, "severity": severity, "description": description}))
            .collect(),
        _ => Vec::new(),
    };
    let gaps = match verdict {
        Verdict::RefuseRed(..) => vec![
            json!({"id": "baseline_red_test_verification", "severity": "review",
            "description": "the accepted verdict is refused by the base-commit rule"}),
        ],
        _ => gaps,
    };
    let (status, summary, evidence) = match verdict {
        Verdict::RefuseRed(..) => ("needs_review", "refused by the base-commit rule", json!([
            {"kind": "review", "summary": "accepted verification demoted"}])),
        Verdict::Accept | Verdict::AcceptWith(_) | Verdict::AcceptDisposing(..) => ("accepted", "every finding resolved; baselines green", json!([
            {"kind": "test", "summary": "focused tests pass"}])),
        Verdict::RefuseWith(_) => ("needs_review", "NOT accepted: a regression stands", json!([
            {"kind": "review", "summary": "a regression the round could not fix stands"}])),
        Verdict::Dispose(said) if said.iter().all(|(_, d)| *d != "open") => ("accepted", "every finding judged", json!([
            {"kind": "test", "summary": "focused tests pass"}])),
        Verdict::DisposeNaming(..) => unreachable!("normalized to Dispose above"),
        Verdict::Dispose(_) => ("needs_review", "NOT accepted: a finding still holds", json!([
            {"kind": "review", "summary": "a finding still holds"}])),
        Verdict::Refuse(sources) => (
            "needs_review",
            "NOT accepted: must-pass baseline tests fail in another task's file",
            json!(sources.iter().map(|source| json!({"kind": "blocker",
                "summary": format!("{source} fails: the fix needs a change there"), "source": source}))
                .collect::<Vec<_>>()),
        ),
    };
    let mut branch = json!({"status": status, "summary": summary, "evidence": evidence,
        "commands_run": [{"kind": "test", "command": "cargo test", "status": "succeeded", "exit_code": 0}],
        "residual_gaps": gaps, "data": {"judged_commit": judged}});
    if let Verdict::RefuseRed(red, command) = verdict {
        branch["data"]["baseline_red_tests"] = json!(red);
        branch["commands_run"] = json!([{"kind": "test", "command": command, "status": "failed",
            "exit_code": 101, "output_summary": "red tests outside this round", "pre_existing": true}]);
    }
    // The agent's own dispositions ride in its result's data, as the
    // adapter leaves them.
    if let Verdict::AcceptDisposing(_, dispositions) = verdict {
        branch["data"]["gap_dispositions"] = dispositions
            .iter()
            .map(|(id, status)| json!({"gap_id": id, "status": status}))
            .collect();
    }
    // Batch O: a verifier judges every finding id its contract names. A
    // scripted accept closes each with evidence and a refusal leaves each
    // open, as a compliant verifier reports them; `Dispose` says exactly.
    let named: Vec<String> = execution
        .call
        .options
        .extra
        .get("remediationContract")
        .and_then(|contract| contract.get("findingIds"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect();
    let said: Vec<(String, &str)> = match verdict {
        Verdict::Dispose(said) => said.iter().map(|(id, d)| (id.clone(), *d)).collect(),
        _ => named
            .iter()
            .map(|id| {
                (
                    id.clone(),
                    if status == "accepted" {
                        "resolved"
                    } else {
                        "open"
                    },
                )
            })
            .collect(),
    };
    if !said.is_empty() {
        branch["data"]["finding_dispositions"] = said
            .iter()
            .map(|(id, disposition)| {
                let evidence = match naming {
                    Some(file) if *disposition == "open" => {
                        format!("still open: the fix needs a change in {file}")
                    }
                    _ => format!("scripted verdict: {disposition}"),
                };
                json!({"finding_id": id, "disposition": disposition, "evidence": evidence})
            })
            .collect();
    }
    let branch_id = format!("{}-0", execution.call.id);
    let tasks = item["canonical_task_ids"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let completion: Vec<Value> = tasks
        .iter()
        .map(|task| {
            json!({"task_id": task,
        "evidence_kind": "focused_verification", "call_id": execution.call.id,
        "item_id": item["item_id"], "status": status, "evidence_refs": ["scripted"]})
        })
        .collect();
    serde_json::from_value(json!({
        "status": status, "summary": summary, "evidence": evidence, "residual_gaps": gaps,
        // Filed under the branch id the host dispatched (`dispatched_items`).
        "data": {"outcomes": [{"item_id": branch_id, "id": branch_id, "status": status,
            "canonical_task_ids": item["canonical_task_ids"], "result": branch,
            "completion_evidence": completion}],
            "items": [branch]},
    }))
    .unwrap()
}
