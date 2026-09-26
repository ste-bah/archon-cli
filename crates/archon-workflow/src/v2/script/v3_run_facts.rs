//! The host's own record of an authored run, reduced to what the terminal
//! rule (`v3_run_outcome`) judges: one fact per executed or replayed call, in
//! script order, carrying the call's role and the per-task statuses its
//! record holds.
//!
//! WHICH task a branch is for comes from the host, never from the agent: the
//! call record's `dispatched_items` (the items the host built each branch
//! from, persisted when the call runs, so a call that failed before any branch
//! answered is still attributed), then the persisted source task graph
//! (verification waves). A branch's STATUS comes from the host-built outcome
//! view matched by branch id (`result.data.outcomes[]`, written by
//! `write::result` / `call_data::fanout_result`); a dispatched branch with no
//! view is charged the call's status. Records written before the host
//! persisted its items fall back to the ids in the outcome views, which the
//! agents reported, and the fact says so (`agent_attributed`).

use std::collections::BTreeMap;

use super::*;
use crate::v2::review_findings::task_ids_of;

/// What a call was for, as far as the terminal rule is concerned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthoredCallRole {
    /// Task write work outside review remediation.
    Write,
    /// A per-task verifier outside review remediation.
    TaskVerify,
    /// A mandatory review map or reducer.
    Review {
        kind: String,
        stage: String,
    },
    /// Review-remediation fix for one task.
    RemediationFix {
        task: String,
        round: u64,
    },
    /// Review-remediation verify for one task; `agent` is false for the
    /// no-patch checkpoint that stands in for a verifier that never ran.
    RemediationVerify {
        task: String,
        round: u64,
        agent: bool,
    },
    Acceptance,
    Other,
}

/// One task's status as a call's record holds it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuthoredTaskOutcome {
    pub status: WorkflowV2Status,
    /// The branch failed on execution (transport, timeout, rate limit).
    pub transport: bool,
    /// The branch produced no verdict at all: an execution, contract or
    /// safety failure, or no result. A reviewer that returned `failed` or
    /// `blocked` DID review; this is the case where nothing did.
    pub not_reviewed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthoredCallFact {
    pub id: String,
    pub role: AuthoredCallRole,
    /// The latest, non-invalidated record's status; `None` when there is none.
    pub status: Option<WorkflowV2Status>,
    /// The record's summary names a transport failure.
    pub transport: bool,
    pub tasks: BTreeMap<String, AuthoredTaskOutcome>,
    /// Acceptance rounds: the round record this call wrote or replayed.
    pub record_path: Option<String>,
    /// Task attribution fell back to what the branch agents reported: a
    /// record written before the host persisted its dispatched items.
    pub agent_attributed: bool,
    /// A remediation fix whose record carries the host's typed "nothing
    /// landed" marker (Issue-111).
    pub landed_nothing: bool,
    /// A remediation verifier dispatched on the host's re-verification plan
    /// (its contract carries the `reverify` key, which the host answers only
    /// on its own plan -- Issue-111).
    pub host_reverify: bool,
}

impl AuthoredCallFact {
    /// This call's status for `task`, when its record names the task.
    pub fn task(&self, task: &str) -> Option<AuthoredTaskOutcome> {
        self.tasks.get(task).copied()
    }

    /// The record's status as an outcome, for single-purpose calls.
    pub fn outcome(&self) -> Option<AuthoredTaskOutcome> {
        self.status.map(|status| AuthoredTaskOutcome {
            status,
            transport: self.transport,
            not_reviewed: false,
        })
    }
}

pub fn authored_call_role(call: &WorkflowV2HostCall) -> AuthoredCallRole {
    if is_acceptance_stage_call(call) {
        return AuthoredCallRole::Acceptance;
    }
    if let Some(stage) = review_contract_stage(call) {
        return AuthoredCallRole::Review {
            kind: review_contract_kind(call).unwrap_or_default().to_string(),
            stage: stage.to_string(),
        };
    }
    if let Some(contract) = remediation_contract(call) {
        let task = remediation_contract_string(call, "taskId")
            .unwrap_or_default()
            .to_string();
        let round = contract
            .get("round")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
        return match remediation_contract_string(call, "stage") {
            Some(REMEDIATION_STAGE_FIX) => AuthoredCallRole::RemediationFix { task, round },
            Some(REMEDIATION_STAGE_VERIFY) => AuthoredCallRole::RemediationVerify {
                task,
                round,
                agent: call.method != WorkflowV2HostMethod::Checkpoint,
            },
            _ => AuthoredCallRole::Other,
        };
    }
    let verify = call.options.item_kind.as_deref() == Some("focused_verification")
        || call.id.starts_with("verification-wave-");
    if verify {
        return AuthoredCallRole::TaskVerify;
    }
    let write = call.write_mode.is_some()
        && matches!(
            call.method,
            WorkflowV2HostMethod::Fanout | WorkflowV2HostMethod::Implementation
        );
    if write {
        AuthoredCallRole::Write
    } else {
        AuthoredCallRole::Other
    }
}

/// Facts for `calls` in script order. A call id seen more than once keeps
/// its LAST position: the store holds only its latest record, and ordering
/// checks ask what happened last.
pub fn authored_call_facts(
    calls: &[WorkflowV2HostCall],
    mut load: impl FnMut(&str) -> WorkflowResult<Option<WorkflowV2CallRecord>>,
) -> WorkflowResult<Vec<AuthoredCallFact>> {
    let mut facts: Vec<AuthoredCallFact> = Vec::new();
    for call in calls {
        facts.retain(|fact| fact.id != call.id);
        let record = load(&call.id)?.filter(|record| record.invalidated_by.is_none());
        facts.push(call_fact(call, record.as_ref()));
    }
    Ok(facts)
}

pub fn call_fact(
    call: &WorkflowV2HostCall,
    record: Option<&WorkflowV2CallRecord>,
) -> AuthoredCallFact {
    let role = authored_call_role(call);
    let Some(record) = record else {
        return AuthoredCallFact {
            id: call.id.clone(),
            role,
            status: None,
            transport: false,
            tasks: BTreeMap::new(),
            record_path: None,
            agent_attributed: false,
            landed_nothing: false,
            host_reverify: false,
        };
    };
    let transport = record.status == WorkflowV2Status::Failed
        && is_transport_failure_text(&record.result.summary);
    let record_path = (role == AuthoredCallRole::Acceptance)
        .then(|| record.result.data.get("record_path"))
        .flatten()
        .and_then(serde_json::Value::as_str)
        .map(str::to_string);
    let (tasks, agent_attributed) = task_outcomes(record, transport);
    let landed_nothing = matches!(role, AuthoredCallRole::RemediationFix { .. })
        && super::remediation_escalation::landed_nothing(&record.result.data);
    let host_reverify = matches!(
        role,
        AuthoredCallRole::RemediationVerify { agent: true, .. }
    ) && remediation_contract(call).is_some_and(|contract| {
        contract
            .get(super::remediation_escalation::REVERIFY_CONTRACT_KEY)
            .is_some()
    });
    AuthoredCallFact {
        id: call.id.clone(),
        role,
        status: Some(record.status),
        transport,
        tasks,
        record_path,
        agent_attributed,
        landed_nothing,
        host_reverify,
    }
}

/// Per-task outcomes of one record, and whether attribution had to fall back
/// to what the agents reported; see the module doc for the sources.
pub fn task_outcomes(
    record: &WorkflowV2CallRecord,
    record_transport: bool,
) -> (BTreeMap<String, AuthoredTaskOutcome>, bool) {
    let mut tasks: BTreeMap<String, AuthoredTaskOutcome> = BTreeMap::new();
    let views = record
        .result
        .data
        .get("outcomes")
        .and_then(serde_json::Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    // A branch the host dispatched that never reported: charged with the
    // call's own status, and never counted as a review.
    let silent = AuthoredTaskOutcome {
        status: record.status,
        transport: record_transport,
        not_reviewed: true,
    };
    if !record.dispatched_items.is_empty() {
        for item in &record.dispatched_items {
            let outcome = views
                .iter()
                .find(|view| view_item_id(view) == Some(item.item_id.as_str()))
                .map_or(silent, view_outcome);
            for task in &item.canonical_task_ids {
                merge_task(&mut tasks, task.clone(), outcome);
            }
        }
        return (tasks, false);
    }
    if let Some(graph) = record
        .source_task_graph
        .as_ref()
        .filter(|g| !g.items.is_empty())
    {
        let outcome = match (graph.items.len(), views) {
            (1, [view]) => view_outcome(view),
            _ => silent,
        };
        for task in graph.items.iter().flat_map(|item| &item.canonical_task_ids) {
            merge_task(&mut tasks, task.clone(), outcome);
        }
        return (tasks, false);
    }
    for view in views {
        let outcome = view_outcome(view);
        for task in task_ids_of(view) {
            merge_task(&mut tasks, task, outcome);
        }
    }
    let agent_attributed = !tasks.is_empty();
    (tasks, agent_attributed)
}

fn view_item_id(view: &serde_json::Value) -> Option<&str> {
    view.get("item_id")
        .or_else(|| view.get("id"))
        .and_then(serde_json::Value::as_str)
}

/// One branch outcome view as the host wrote it.
fn view_outcome(view: &serde_json::Value) -> AuthoredTaskOutcome {
    let mut status = view
        .get("status")
        .cloned()
        .and_then(|value| serde_json::from_value::<WorkflowV2Status>(value).ok())
        .unwrap_or(WorkflowV2Status::NeedsReview);
    if view.get("contract_valid") == Some(&serde_json::Value::Bool(false)) {
        status = merge_v2_status(status, WorkflowV2Status::NeedsReview);
    }
    let failure = view.get("failure_kind").and_then(serde_json::Value::as_str);
    let no_result = view.get("result").is_none_or(serde_json::Value::is_null);
    AuthoredTaskOutcome {
        status,
        transport: failure == Some("execution"),
        not_reviewed: no_result || matches!(failure, Some("execution" | "contract" | "safety")),
    }
}

/// Several branches naming one task: the worst one stands.
fn merge_task(
    tasks: &mut BTreeMap<String, AuthoredTaskOutcome>,
    task: String,
    outcome: AuthoredTaskOutcome,
) {
    let entry = tasks.entry(task).or_insert(outcome);
    let not_reviewed = entry.not_reviewed || outcome.not_reviewed;
    if merge_v2_status(entry.status, outcome.status) != entry.status {
        *entry = outcome;
    }
    entry.not_reviewed = not_reviewed;
}

/// Tasks the universe declares at least one writable file for (exclusive or
/// shared-append). A `not_task_actionable` remediation outcome is credible
/// only for a task outside this set.
pub fn writable_task_ids(
    universe: Option<&crate::task_universe::WorkflowV2TaskUniverse>,
) -> std::collections::BTreeSet<String> {
    universe
        .into_iter()
        .flat_map(|universe| &universe.tasks)
        .filter(|task| {
            task.files_expected_to_change
                .iter()
                .chain(&task.shared_append_target_files)
                .any(|entry| declared_path(entry).is_some())
        })
        .map(|task| task.canonical_task_id.clone())
        .collect()
}

#[cfg(test)]
#[path = "v3_run_facts_tests.rs"]
mod tests;
