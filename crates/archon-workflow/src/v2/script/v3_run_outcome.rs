//! The authored (v3) run's terminal status, derived from its FINAL accounting.
//!
//! The script host accumulates a status over every call it executes, keeping
//! the worst one it ever saw. For an authored script that is the wrong
//! question: the script's own loops exist to turn intermediate verdicts into
//! final ones. A review map call that FINDS something reports `needs_review` —
//! that is its job, and its findings flow on to remediation. A verifier that
//! rejects round 1 and accepts round 2 reports `needs_review` once. Either one
//! pinned the whole run to `NeedsReview` however the run actually ended, so no
//! authored run could ever complete.
//!
//! This module decides the status from where the run ENDED instead:
//!
//! - hard stops dominate unchanged: a cancelled call, a failure the host
//!   recorded as terminal (`failed_call`: a terminal gate, a script error, the
//!   repository audit), or a script that never returned its accounting;
//! - otherwise the run is `Accepted` iff no task is left blocked, every task a
//!   review finding names reached an outcome, no remediation outcome is open
//!   other than `not_task_actionable`, and the acceptance stage's final round
//!   (the host's own round record, never the script's copy) passed;
//! - anything else is `NeedsReview`, or `Blocked` when an open item failed on
//!   transport rather than on its merits.
//!
//! Script-reported claims are cross-checked against host records where the
//! host has one: a task reported `resolved` by review remediation must have a
//! host-recorded accepted remediation verify call, or it counts as open.
//! Findings naming no task (`unassigned`) are reported and do not block.
//!
//! Everything here is a pure function of its inputs, so a resumed process —
//! which replays cached calls and re-runs the rest from the top — reaches the
//! same verdict from the same final accounting.

use std::collections::BTreeSet;

use super::*;
use crate::v2::AuthoredAcceptanceGateV1;

/// The one remediation outcome that does not hold a run open: the findings
/// name nothing a task owns, so no task could have acted on them.
pub const NOT_TASK_ACTIONABLE_OUTCOME: &str = "not_task_actionable";

/// What the host knows about the acceptance stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthoredAcceptanceGateFact<'a> {
    /// No acceptance call ran and the script predates the rule.
    NotRequired,
    /// The script is under the rule, yet no acceptance call ran or the host
    /// wrote no round record for it.
    Missing,
    /// The last acceptance call THIS run executed or replayed, beside the
    /// latest round record on disk. Binding the two means a record left by an
    /// earlier process for a round this run never reached cannot pass the
    /// gate.
    Recorded {
        gate: &'a AuthoredAcceptanceGateV1,
        record_call_id: &'a str,
        last_call_id: &'a str,
        last_call_status: Option<WorkflowV2Status>,
    },
}

/// Inputs to [`authored_run_terminal_status`], all host-sourced except the
/// script's accounting, whose shape the host validators already checked.
#[derive(Debug, Clone, Copy)]
pub struct AuthoredRunFacts<'a> {
    /// The accumulator's worst-call status.
    pub accumulated_status: WorkflowV2Status,
    /// The call the host recorded as the run's terminal failure, if any.
    pub host_terminal_failure: Option<&'a str>,
    /// The script's return value; `None` when it never returned.
    pub script_result: Option<&'a str>,
    pub acceptance_gate: AuthoredAcceptanceGateFact<'a>,
    /// Tasks with a host-recorded accepted review-remediation verify call.
    pub verified_remediation_tasks: &'a BTreeSet<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthoredRunOutcome {
    pub status: WorkflowV2Status,
    /// True when the final accounting decided; false when a hard stop kept
    /// the accumulated status.
    pub from_accounting: bool,
    /// What holds the run open, one clause each; empty when accepted.
    pub blocking: Vec<String>,
    /// Non-blocking facts an operator should still see.
    pub notes: Vec<String>,
}

impl AuthoredRunOutcome {
    /// One line stating why the run got its status.
    pub fn explanation(&self) -> String {
        let mut line = format!("authored run {:?}", self.status);
        if !self.blocking.is_empty() {
            line.push_str(&format!(": {}", self.blocking.join("; ")));
        }
        if !self.notes.is_empty() {
            line.push_str(&format!(" [{}]", self.notes.join("; ")));
        }
        line
    }
}

/// The authored run's terminal status from its final accounting; see the
/// module doc for the rule.
pub fn authored_run_terminal_status(facts: &AuthoredRunFacts<'_>) -> AuthoredRunOutcome {
    let hard = |reason: String| AuthoredRunOutcome {
        status: facts.accumulated_status,
        from_accounting: false,
        blocking: vec![reason],
        notes: Vec::new(),
    };
    if facts.accumulated_status == WorkflowV2Status::Cancelled {
        return hard("a call was cancelled".to_string());
    }
    if let Some(call_id) = facts.host_terminal_failure {
        return hard(format!(
            "the host recorded a terminal failure at `{call_id}`"
        ));
    }
    let Some(raw) = facts.script_result else {
        return hard("the script stopped before returning its accounting".to_string());
    };
    let mut blocking = Vec::new();
    let mut notes = Vec::new();
    let mut transport = false;
    let accounting: serde_json::Value = match serde_json::from_str(raw) {
        Ok(value) => value,
        Err(error) => {
            return AuthoredRunOutcome {
                status: WorkflowV2Status::NeedsReview,
                from_accounting: true,
                blocking: vec![format!("the accounting is not JSON: {error}")],
                notes,
            };
        }
    };
    let remediation = accounting.get("review_remediation");
    let resolved = resolved_tasks(remediation, facts.verified_remediation_tasks);
    for unbacked in resolved.reported.difference(&resolved.backed) {
        blocking.push(format!(
            "task {unbacked} is reported resolved but the host has no accepted remediation verify call for it"
        ));
    }
    for entry in array(accounting.get("blocked")) {
        let task = task_id(entry).unwrap_or("<unnamed>");
        if resolved.backed.contains(task) {
            notes.push(format!(
                "blocked task {task} was resolved by review remediation"
            ));
            continue;
        }
        let reason = text(entry.get("reason"));
        transport |= is_transport_failure_text(reason);
        blocking.push(format!("task {task} is blocked: {}", clip(reason)));
    }
    match remediation {
        Some(remediation) if remediation.is_object() => {
            let mut outcomes = resolved.reported.clone();
            for entry in array(remediation.get("unresolved")) {
                let task = task_id(entry).unwrap_or("<unnamed>");
                outcomes.insert(task.to_string());
                let outcome = text(entry.get("outcome"));
                if outcome == NOT_TASK_ACTIONABLE_OUTCOME {
                    notes.push(format!("task {task}: findings not task-actionable"));
                    continue;
                }
                let reason = text(entry.get("reason"));
                transport |= is_transport_failure_text(reason);
                blocking.push(format!(
                    "task {task} review remediation is {}: {}",
                    if outcome.is_empty() { "open" } else { outcome },
                    clip(reason)
                ));
            }
            let unassigned = array(remediation.get("unassigned")).len();
            if unassigned > 0 {
                notes.push(format!(
                    "{unassigned} finding(s) name no task (non-blocking)"
                ));
            }
            for task in finding_tasks(&accounting).difference(&outcomes) {
                blocking.push(format!(
                    "review findings name task {task} but review remediation reports no outcome for it"
                ));
            }
        }
        _ => {
            if !finding_tasks(&accounting).is_empty() {
                blocking.push(
                    "review findings name tasks but the accounting carries no review_remediation"
                        .to_string(),
                );
            }
        }
    }
    acceptance_verdict(facts.acceptance_gate, &mut blocking, &mut notes);
    let status = if blocking.is_empty() {
        WorkflowV2Status::Accepted
    } else if transport {
        WorkflowV2Status::Blocked
    } else {
        WorkflowV2Status::NeedsReview
    };
    AuthoredRunOutcome {
        status,
        from_accounting: true,
        blocking,
        notes,
    }
}

fn acceptance_verdict(
    fact: AuthoredAcceptanceGateFact<'_>,
    blocking: &mut Vec<String>,
    notes: &mut Vec<String>,
) {
    let (gate, record_call_id, last_call_id, last_call_status) = match fact {
        AuthoredAcceptanceGateFact::NotRequired => {
            notes.push("no acceptance stage (script predates the rule)".to_string());
            return;
        }
        AuthoredAcceptanceGateFact::Missing => {
            blocking.push("the acceptance stage recorded no round".to_string());
            return;
        }
        AuthoredAcceptanceGateFact::Recorded {
            gate,
            record_call_id,
            last_call_id,
            last_call_status,
        } => (gate, record_call_id, last_call_id, last_call_status),
    };
    if record_call_id != last_call_id {
        blocking.push(format!(
            "the latest acceptance record belongs to `{record_call_id}`, not to `{last_call_id}`, the last round this run executed"
        ));
    } else if !last_call_status.is_some_and(is_reusable_status) {
        blocking.push(format!(
            "acceptance call `{last_call_id}` recorded {}",
            last_call_status.map_or("no result".to_string(), |status| format!("{status:?}"))
        ));
    }
    if gate.blocks_completion() {
        blocking.push(if gate.failing_check_ids.is_empty() {
            format!(
                "acceptance round {} could not evaluate: {}",
                gate.final_round,
                gate.operational_errors.join("; ")
            )
        } else {
            format!(
                "acceptance round {} has failing checks: {}",
                gate.final_round,
                gate.failing_check_ids.join(", ")
            )
        });
    } else {
        notes.push(format!(
            "acceptance round {} passed{}",
            gate.final_round,
            if gate.contract_present {
                ""
            } else {
                " (no contract)"
            }
        ));
    }
}

/// Tasks with an accepted review-remediation verify call before the
/// acceptance stage, read from the host's own call records. Calls after the
/// first acceptance round belong to the stage's remediation, which its round
/// record judges.
pub fn review_remediation_verified_tasks(
    calls: &[WorkflowV2HostCall],
    mut status_of: impl FnMut(&str) -> WorkflowResult<Option<WorkflowV2Status>>,
) -> WorkflowResult<BTreeSet<String>> {
    let mut verified = BTreeSet::new();
    for call in calls
        .iter()
        .take_while(|call| !is_acceptance_stage_call(call))
    {
        if remediation_contract_string(call, "stage") != Some(REMEDIATION_STAGE_VERIFY) {
            continue;
        }
        let Some(task) = remediation_contract_string(call, "taskId") else {
            continue;
        };
        if status_of(&call.id)?.is_some_and(is_reusable_status) {
            verified.insert(task.to_string());
        }
    }
    Ok(verified)
}

struct ResolvedTasks {
    reported: BTreeSet<String>,
    backed: BTreeSet<String>,
}

fn resolved_tasks(
    remediation: Option<&serde_json::Value>,
    verified: &BTreeSet<String>,
) -> ResolvedTasks {
    let reported = array(remediation.and_then(|value| value.get("resolved")))
        .iter()
        .filter_map(task_id)
        .map(str::to_string)
        .collect::<BTreeSet<_>>();
    let backed = reported.intersection(verified).cloned().collect();
    ResolvedTasks { reported, backed }
}

/// Tasks the review findings name, grouped the way the prelude's
/// `findingsByTask` groups them: an explicit non-attributable finding and one
/// naming no id are unassigned.
fn finding_tasks(accounting: &serde_json::Value) -> BTreeSet<String> {
    let mut tasks = BTreeSet::new();
    for field in MANDATED_RESULT_FIELDS {
        for finding in array(accounting.get(field)) {
            if finding.get("attributable_to_task") == Some(&serde_json::Value::Bool(false)) {
                continue;
            }
            let named = ["canonical_task_ids", "task_ids", "taskIds", "task_id"]
                .iter()
                .find_map(|key| finding.get(*key).filter(|value| !value.is_null()));
            match named {
                Some(serde_json::Value::Array(ids)) => tasks.extend(
                    ids.iter()
                        .filter_map(serde_json::Value::as_str)
                        .filter(|id| !id.is_empty())
                        .map(str::to_string),
                ),
                Some(serde_json::Value::String(id)) if !id.is_empty() => {
                    tasks.insert(id.clone());
                }
                _ => {}
            }
        }
    }
    tasks
}

fn array(value: Option<&serde_json::Value>) -> &[serde_json::Value] {
    value
        .and_then(serde_json::Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
}

fn task_id(entry: &serde_json::Value) -> Option<&str> {
    entry
        .get("taskId")
        .or_else(|| entry.get("task_id"))
        .and_then(serde_json::Value::as_str)
}

fn text(value: Option<&serde_json::Value>) -> &str {
    value.and_then(serde_json::Value::as_str).unwrap_or("")
}

fn clip(text: &str) -> String {
    const LIMIT: usize = 200;
    if text.chars().count() <= LIMIT {
        return text.to_string();
    }
    format!("{}...", text.chars().take(LIMIT).collect::<String>())
}

#[cfg(test)]
#[path = "v3_run_outcome_tests.rs"]
mod tests;
