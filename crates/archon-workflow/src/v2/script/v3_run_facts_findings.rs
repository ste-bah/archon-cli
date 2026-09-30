//! Batch O: the per-finding facts of remediation calls, for the terminal
//! rule's per-finding closure (`v3_run_outcome_closure`).

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use super::super::remediation_dispositions::{
    CHECK_FINDING_IDS_KEY, DispositionFact, FINDING_IDS_KEY, REFUTATION_KEY, contract_ids,
    disposition_of,
};
use super::super::remediation_plan::{asks_for_plan, planned_findings};
use super::*;
use crate::v2::review_finding_ids::finding_id_of;

/// What the host records of one call's part in per-finding remediation.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RemediationFact {
    /// The unit the contract names (a split part or a later cycle).
    pub unit: Option<String>,
    /// The finding ids the contract acts on.
    pub finding_ids: Vec<String>,
    /// Of those, the ones the script said concern a check.
    pub check_ids: BTreeSet<String>,
    /// A verifier judging a fix that landed nothing.
    pub refutation: bool,
    /// A verifier's record: what it said of each id.
    pub dispositions: BTreeMap<String, DispositionFact>,
    /// A remediation plan checkpoint: the blocked tasks it folded in, each
    /// with the ids of the findings standing for it.
    pub planned_blocked: BTreeMap<String, BTreeSet<String>>,
    /// A remediation plan checkpoint: every other finding it was handed
    /// (an unreviewed marker aside, which the review rule holds).
    pub planned_ids: BTreeSet<String>,
}

/// The fact for `call`, reading `record` when there is one.
pub fn remediation_fact(
    call: &WorkflowV2HostCall,
    record: Option<&WorkflowV2CallRecord>,
) -> RemediationFact {
    let mut fact = RemediationFact::default();
    if let Some(record) = record
        && asks_for_plan(record)
    {
        for finding in planned_findings(record) {
            if let Some(task) = finding.get("blocked_task").and_then(Value::as_str) {
                fact.planned_blocked
                    .entry(task.trim().to_string())
                    .or_default()
                    .insert(finding_id_of(&finding));
            } else if finding.get("review_outcome").and_then(Value::as_str)
                != Some(crate::v2::review_findings::UNREVIEWED_OUTCOME)
            {
                fact.planned_ids.insert(finding_id_of(&finding));
            }
        }
        return fact;
    }
    let Some(contract) = remediation_contract(call) else {
        return fact;
    };
    fact.unit = contract
        .get("unit")
        .and_then(Value::as_str)
        .map(str::to_string);
    fact.finding_ids = contract_ids(Some(contract), FINDING_IDS_KEY)
        .into_iter()
        .collect();
    fact.check_ids = contract_ids(Some(contract), CHECK_FINDING_IDS_KEY);
    fact.refutation = contract.get(REFUTATION_KEY) == Some(&Value::Bool(true));
    let verify = contract.get("stage").and_then(Value::as_str) == Some(REMEDIATION_STAGE_VERIFY)
        && call.method != WorkflowV2HostMethod::Checkpoint;
    if let (true, Some(record)) = (verify, record) {
        fact.dispositions = fact
            .finding_ids
            .iter()
            .map(|id| (id.clone(), disposition_of(record, id)))
            .collect();
    }
    fact
}
