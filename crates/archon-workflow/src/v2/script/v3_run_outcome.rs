//! The authored (v3) run's terminal status, decided from HOST records.
//!
//! The script host accumulates the worst status any call returned. For an
//! authored script that is the wrong question — a review map that finds
//! something reports `needs_review` by design, and a verifier that rejects
//! round 1 and accepts round 2 reports `needs_review` once — so it pinned
//! every authored run to `NeedsReview` however the run ended.
//!
//! This rule asks where the run ENDED, and it asks the host, not the script.
//! Every list the script returns is a claim to be checked against the call
//! records (`v3_run_facts`), never a verdict:
//!
//! - hard stops keep their status: a cancelled call, a failure the host
//!   recorded as terminal (`failed_call`), a script that never returned;
//! - ids are the universe's: every id the script reports is trimmed and
//!   matched case-insensitively against the task universe (`TaskKeys`);
//! - a task reported `accepted` needs its latest pre-review write accepted or
//!   noop for that task, and a later pre-review verify accepted for it, where
//!   the task a branch served is the host's dispatched item, not the agent's
//!   report;
//! - every mandatory review call must have run: a review call whose record is
//!   missing, failed, blocked or cancelled, or a map branch that produced no
//!   verdict (execution/contract/safety failure, no result), means no review
//!   happened — a reviewer that returned `failed` DID review;
//! - `blocked` tasks hold the run unless review remediation verifiably
//!   finished them; `resolved` needs, for the key's LAST remediation round, an
//!   accepted fix followed by an accepted verifier AGENT (a no-patch
//!   checkpoint is not one) for every task the key names; the same holds for
//!   remediation the acceptance stage dispatched; every other open outcome
//!   holds the run, and `not_task_actionable` only stands for universe tasks
//!   with no writable file;
//! - every task a finding names needs a remediation outcome, and a finding
//!   that names tasks but opts out of single-task attribution needs one for
//!   its cross-task key; a finding naming no universe task blocks unless its
//!   severity is on an explicit low-impact allow-list and it is no uncovered
//!   requirement;
//! - the acceptance round record bound to the last acceptance call this run
//!   executed or replayed must pass.
//!
//! Anything open is `NeedsReview`, or `Blocked` when an open item failed on
//! transport. The rule is a pure function of host records and the final
//! accounting, so a resumed process reaches the same verdict.

use std::collections::BTreeSet;

use super::*;
use crate::v2::AuthoredAcceptanceGateV1;

/// The one remediation outcome that can stand without a fix: the findings
/// name nothing the task may write.
pub const NOT_TASK_ACTIONABLE_OUTCOME: &str = "not_task_actionable";
/// A `review_outcome` marking a review branch that never completed. The host
/// attaches one to a review map for each branch left without a verdict
/// (`review_findings::UNREVIEWED_OUTCOME`), naming its task; it holds the run
/// whatever remediation reports for that task.
pub const UNREVIEWED_REVIEW_OUTCOME: &str = crate::v2::review_findings::UNREVIEWED_OUTCOME;

/// What the host knows about the acceptance stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthoredAcceptanceGateFact<'a> {
    /// No acceptance call ran and the script predates the rule.
    NotRequired,
    /// Under the rule, yet no acceptance call ran or its record is missing.
    Missing,
    /// The round record bound to the last acceptance call this run executed
    /// or replayed (the path that call's own record names).
    Recorded {
        gate: &'a AuthoredAcceptanceGateV1,
        record_call_id: &'a str,
        last_call_id: &'a str,
        last_call_status: Option<WorkflowV2Status>,
    },
}

#[derive(Debug, Clone, Copy)]
pub struct AuthoredRunFacts<'a> {
    /// The accumulator's worst-call status.
    pub accumulated_status: WorkflowV2Status,
    /// The call the host recorded as the run's terminal failure, if any.
    pub host_terminal_failure: Option<&'a str>,
    /// The script's return value; `None` when it never returned.
    pub script_result: Option<&'a str>,
    pub acceptance_gate: AuthoredAcceptanceGateFact<'a>,
    /// Every executed or replayed call, in script order.
    pub calls: &'a [AuthoredCallFact],
    /// Tasks the task universe declares writable files for.
    pub writable_tasks: &'a BTreeSet<String>,
    /// Every task id in the universe, in its canonical spelling.
    pub universe_tasks: &'a BTreeSet<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthoredRunOutcome {
    pub status: WorkflowV2Status,
    /// True when the host records decided; false when a hard stop kept the
    /// accumulated status.
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

#[derive(Default)]
pub(super) struct Verdict {
    pub(super) blocking: Vec<String>,
    pub(super) notes: Vec<String>,
    pub(super) transport: bool,
}

impl Verdict {
    pub(super) fn block(&mut self, clause: String, transport: bool) {
        self.blocking.push(clause);
        self.transport |= transport;
    }
}

impl AuthoredRunOutcome {
    /// Issue-117: the residual gate's clauses folded in. A clause holds only
    /// a verdict the host records decided; a hard stop keeps its status.
    pub fn with_residual_gate(mut self, blocking: Vec<String>, notes: Vec<String>) -> Self {
        if self.from_accounting
            && !blocking.is_empty()
            && matches!(
                self.status,
                WorkflowV2Status::Accepted | WorkflowV2Status::Noop
            )
        {
            self.status = WorkflowV2Status::NeedsReview;
        }
        self.blocking.extend(blocking);
        self.notes.extend(notes);
        self
    }
}

/// The authored run's terminal status; see the module doc for the rule.
pub fn authored_run_terminal_status(facts: &AuthoredRunFacts<'_>) -> AuthoredRunOutcome {
    authored_run_terminal_status_with(facts, &BTreeSet::new())
}

/// [`authored_run_terminal_status`], where each key of `discharged` is a
/// review remediation unit a host-planned review round completed
/// (Issue-117, `residual_plan`): its open outcome is a note, not a hold.
pub fn authored_run_terminal_status_with(
    facts: &AuthoredRunFacts<'_>,
    discharged: &BTreeSet<String>,
) -> AuthoredRunOutcome {
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
    let mut verdict = Verdict::default();
    match serde_json::from_str::<serde_json::Value>(raw) {
        Ok(accounting) => judge(&accounting, facts, discharged, &mut verdict),
        Err(error) => verdict.block(format!("the accounting is not JSON: {error}"), false),
    }
    let status = if verdict.blocking.is_empty() {
        WorkflowV2Status::Accepted
    } else if verdict.transport {
        WorkflowV2Status::Blocked
    } else {
        WorkflowV2Status::NeedsReview
    };
    AuthoredRunOutcome {
        status,
        from_accounting: true,
        blocking: verdict.blocking,
        notes: verdict.notes,
    }
}

fn judge(
    accounting: &serde_json::Value,
    facts: &AuthoredRunFacts<'_>,
    discharged: &BTreeSet<String>,
    v: &mut Verdict,
) {
    let calls = facts.calls;
    let review_start = calls
        .iter()
        .position(|call| {
            matches!(
                call.role,
                AuthoredCallRole::Review { .. } | AuthoredCallRole::Acceptance
            )
        })
        .unwrap_or(calls.len());
    let acceptance_start = calls
        .iter()
        .position(|call| call.role == AuthoredCallRole::Acceptance)
        .unwrap_or(calls.len());
    let keys = TaskKeys {
        universe: facts.universe_tasks,
    };
    for task in array(accounting.get("accepted"))
        .iter()
        .filter_map(serde_json::Value::as_str)
    {
        check_accepted_task(&keys.key(task), &calls[..review_start], v);
    }
    check_reviews(calls, v);
    note_unattributed_failures(&calls[..review_start], v);
    let legacy = calls.iter().filter(|call| call.agent_attributed).count();
    if legacy > 0 {
        v.notes.push(format!(
            "{legacy} call record(s) predate host item attribution; their tasks were read from the branch outcome views"
        ));
    }
    let outcomes = check_remediation(
        accounting,
        facts,
        &keys,
        &calls[..acceptance_start],
        discharged,
        v,
    );
    let gate_clean = matches!(
        facts.acceptance_gate,
        AuthoredAcceptanceGateFact::Recorded { gate, record_call_id, last_call_id, last_call_status }
            if !gate.blocks_completion()
                && record_call_id == last_call_id
                && last_call_status.is_some_and(is_reusable_status)
    );
    check_acceptance_remediation(&calls[acceptance_start..], &keys, gate_clean, v);
    check_findings(accounting, &outcomes, &keys, v);
    acceptance_verdict(facts.acceptance_gate, v);
}

/// A task the script reports accepted, held to the host's pre-review record.
fn check_accepted_task(task: &str, pre_review: &[AuthoredCallFact], v: &mut Verdict) {
    let last = |role: AuthoredCallRole| {
        pre_review
            .iter()
            .enumerate()
            .rev()
            .find(|(_, call)| call.role == role && call.task(task).is_some())
    };
    let Some((write_at, write)) = last(AuthoredCallRole::Write) else {
        v.block(
            format!("task {task} is reported accepted but no host write record names it"),
            false,
        );
        return;
    };
    let written = write.task(task).expect("filtered on the task");
    if !is_reusable_status(written.status) {
        v.block(
            format!(
                "task {task} is reported accepted but its latest write `{}` is {:?}",
                write.id, written.status
            ),
            written.transport,
        );
    }
    let Some((verify_at, verify)) = last(AuthoredCallRole::TaskVerify) else {
        v.block(
            format!("task {task} is reported accepted but no host verify record names it"),
            false,
        );
        return;
    };
    let verified = verify.task(task).expect("filtered on the task");
    if !is_reusable_status(verified.status) {
        v.block(
            format!(
                "task {task} is reported accepted but its latest verify `{}` is {:?}",
                verify.id, verified.status
            ),
            verified.transport,
        );
    } else if verify_at < write_at {
        v.block(
            format!(
                "task {task} is reported accepted but nothing verified it after its latest write `{}`",
                write.id
            ),
            false,
        );
    }
}

/// Every mandatory review call ran, for every task it was given.
fn check_reviews(calls: &[AuthoredCallFact], v: &mut Verdict) {
    for call in calls {
        let AuthoredCallRole::Review { kind, stage } = &call.role else {
            continue;
        };
        match call.status {
            None => v.block(
                format!(
                    "review call `{}` ({kind} {stage}) has no host record",
                    call.id
                ),
                false,
            ),
            Some(
                status @ (WorkflowV2Status::Failed
                | WorkflowV2Status::Blocked
                | WorkflowV2Status::Cancelled),
            ) => v.block(
                format!(
                    "review call `{}` ({kind} {stage}) is {status:?}: that review did not happen",
                    call.id
                ),
                call.transport,
            ),
            Some(_) => {}
        }
        // A reviewer's own verdict (`failed`, `blocked`) is a review; only a
        // branch that produced none did not review its task.
        for (task, outcome) in &call.tasks {
            if outcome.not_reviewed {
                v.block(
                    format!(
                        "review call `{}` did not review task {task} (branch {:?})",
                        call.id, outcome.status
                    ),
                    outcome.transport,
                );
            }
        }
    }
}

/// Pre-review work that failed before naming its tasks cannot be charged to
/// one; each task's later host verify decides, and the operator is told.
fn note_unattributed_failures(pre_review: &[AuthoredCallFact], v: &mut Verdict) {
    for call in pre_review {
        if matches!(
            call.role,
            AuthoredCallRole::Write | AuthoredCallRole::TaskVerify
        ) && call.tasks.is_empty()
            && !call.status.is_some_and(is_reusable_status)
        {
            v.notes.push(format!(
                "`{}` failed before naming its tasks ({})",
                call.id,
                call.status
                    .map_or("no record".to_string(), |status| format!("{status:?}"))
            ));
        }
    }
}

pub(super) fn array(value: Option<&serde_json::Value>) -> &[serde_json::Value] {
    value
        .and_then(serde_json::Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
}

pub(super) fn task_id(entry: &serde_json::Value) -> Option<&str> {
    entry
        .get("taskId")
        .or_else(|| entry.get("task_id"))
        .and_then(serde_json::Value::as_str)
}

pub(super) fn text(value: Option<&serde_json::Value>) -> &str {
    value.and_then(serde_json::Value::as_str).unwrap_or("")
}

pub(super) fn clip(text: &str) -> String {
    const LIMIT: usize = 200;
    if text.chars().count() <= LIMIT {
        return text.to_string();
    }
    format!("{}...", text.chars().take(LIMIT).collect::<String>())
}

#[path = "v3_run_outcome_findings.rs"]
mod findings;
use findings::check_findings;
#[path = "v3_run_outcome_gates.rs"]
mod gates;
use gates::acceptance_verdict;
#[path = "v3_run_outcome_keys.rs"]
mod keys;
use keys::TaskKeys;
pub use keys::{CROSS_TASK_KEY_PREFIX, cross_key};
#[path = "v3_run_outcome_remediation.rs"]
mod remediation;
use remediation::{check_acceptance_remediation, check_remediation};

#[cfg(test)]
#[path = "v3_run_outcome_tests.rs"]
mod tests;
