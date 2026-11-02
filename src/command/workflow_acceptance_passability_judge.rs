use std::collections::{BTreeMap, BTreeSet};

use anyhow::Result;
use archon_workflow::WorkflowLlmClient;
use archon_workflow::task_set_contract::{AcceptanceContract, AcceptanceCriterion};

use super::{Candidate, Decision, INSTRUCTION, entries};
use crate::command::workflow_freeze_budget::{FreezeIncomplete, FreezeResume};

/// One batch over pending checks; preserve any verdicts from an incomplete reply.
pub(super) async fn ask(
    client: &dyn WorkflowLlmClient,
    model: &str,
    contract: &AcceptanceContract,
    pending: &[&Candidate],
) -> Result<(BTreeMap<String, Decision>, Option<anyhow::Error>)> {
    let ids: BTreeSet<&str> = pending
        .iter()
        .map(|candidate| candidate.id.as_str())
        .collect();
    let mut subset = contract.clone();
    subset
        .acceptance
        .retain(|entry| ids.contains(entry.id.as_str()));
    (subset.supplementary).retain(|entry| ids.contains(entry.id.as_str()));
    let prompt = |subset: &AcceptanceContract| {
        let subset_ids: BTreeSet<&str> = entries(subset).map(|entry| entry.id.as_str()).collect();
        let checks: Vec<&serde_json::Value> = pending
            .iter()
            .filter(|candidate| subset_ids.contains(candidate.id.as_str()))
            .map(|candidate| &candidate.shown)
            .collect();
        Ok(format!(
            "{INSTRUCTION} Checks: {}",
            serde_json::to_string(&checks)?
        ))
    };
    let decision = |entry: &AcceptanceCriterion| {
        let decision = Decision {
            verdict: entry.judgment.verdict,
            counterexample: entry.judgment.counterexample.clone(),
            reason: entry.judgment.reason.clone(),
            sampling: entry.judgment.sampling.clone(),
        };
        (entry.id.clone(), decision)
    };
    match super::super::judge::judge_refutations(client, subset, prompt, model, |answered| {
        entries(answered).any(|entry| {
            let Some(candidate) = pending.iter().find(|candidate| candidate.id == entry.id) else {
                return false;
            };
            let mut decision = decision(entry).1;
            super::evidence::ground_refutation(
                &mut decision,
                &candidate.evidence,
                &candidate.shown["check"],
            );
            decision.verdict != archon_workflow::task_set_contract::JudgeDecision::Accepted
        })
    })
    .await
    {
        Ok(judged) => Ok((entries(&judged).map(decision).collect(), None)),
        Err(error) => match super::super::judge::JudgeIncomplete::caused(&error) {
            Some(incomplete) => {
                let decided = (incomplete.decided.iter())
                    .filter(|entry| ids.contains(entry.id.as_str()))
                    .map(decision)
                    .collect();
                Ok((decided, Some(error)))
            }
            None => Err(error),
        },
    }
}

pub(in crate::command::workflow_task_set) fn incomplete_judge_error(
    error: anyhow::Error,
    resume: &FreezeResume,
    deferred: Vec<String>,
) -> anyhow::Error {
    let authored_reply = super::super::judge::JudgeIncomplete::caused(&error)
        .is_some_and(super::super::judge::JudgeIncomplete::is_authored_reply);
    let cause = archon_observability::redaction::redact_text(&format!("{error:#}"));
    let incomplete =
        FreezeIncomplete::awaiting_judge(&resume.budget, &resume.progress, deferred, cause)
            .with_fault_class(crate::command::workflow_host_fault::classify(&error));
    if authored_reply {
        incomplete.authored_judge_reply().into()
    } else {
        incomplete.into()
    }
}
