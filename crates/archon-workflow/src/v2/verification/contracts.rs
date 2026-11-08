//! Host enforcement of declared deliverable contracts.
//!
//! The host collects evidence and evaluates the shared native predicates.
//! Only a passing floor can run its separately declared command. Every
//! finding reaches the branch outcome. Launch faults retain the bounded
//! operational retry; unavailable typed programs are recorded as unverified.

use crate::task_universe::WorkflowV2DeliverableContract;
use crate::v2::deliverable_contract::ContractRoots;
use crate::v2::{
    BranchFailureKind, FloorVerdict, WorkflowV2BranchOutcome, WorkflowV2Evidence,
    WorkflowV2EvidenceKind, WorkflowV2Status, collect_declarative_floor_facts,
    evaluate_floor_predicates,
};

/// Evaluate every accepted branch's declared contracts against host-collected
/// evidence, then run any authored commands whose predicates passed. Relative
/// references bind to the first resolution root holding their evidence.
pub async fn enforce_declared_contracts(
    outcomes: &mut [WorkflowV2BranchOutcome],
    contracts: &std::collections::BTreeMap<String, (ContractRoots, Vec<serde_json::Value>)>,
) {
    enforce_declared_contracts_watched(outcomes, contracts, None).await;
}

/// [`enforce_declared_contracts`], each branch's verifiers under the
/// project-input tripwire of the run at `run_root` (Batch G; Batch G2 for
/// what a failure of the environment does).
///
/// A verifier that could not start or did not finish, or a window in which
/// the project's inputs changed (the host puts them back), says nothing
/// about the branch: the branch's verifiers are re-run once, alone, from its
/// outcome as it was. Still unanswered, the branch fails as the host's own
/// operational error ([`mark_branch_operational`]) -- retried like a dropped
/// transport, never demoted as a contract violation, and no other branch's
/// verdict is touched.
pub async fn enforce_declared_contracts_watched(
    outcomes: &mut [WorkflowV2BranchOutcome],
    contracts: &std::collections::BTreeMap<String, (ContractRoots, Vec<serde_json::Value>)>,
    run_root: Option<&std::path::Path>,
) {
    enforce_declared_contracts_classified(outcomes, contracts, run_root, None).await;
}

/// The host supplies its existing program classifier using the command's actual
/// launch environment. An absent site program is recorded without asking the
/// implementation to provide it; a missing product still fails normally.
pub async fn enforce_declared_contracts_classified(
    outcomes: &mut [WorkflowV2BranchOutcome],
    contracts: &std::collections::BTreeMap<String, (ContractRoots, Vec<serde_json::Value>)>,
    run_root: Option<&std::path::Path>,
    classifier: Option<TypedProgramClassifier>,
) {
    if contracts.is_empty() {
        return;
    }
    for outcome in outcomes.iter_mut() {
        if !matches!(
            outcome.status,
            WorkflowV2Status::Accepted | WorkflowV2Status::Noop
        ) {
            continue;
        }
        let Some((roots, declared)) = contracts.get(&outcome.item_id) else {
            continue;
        };
        let pristine = outcome.clone();
        let label = format!("declared contract verifiers of {}", outcome.item_id);
        let mut unanswered = None;
        for _attempt in 0..2 {
            *outcome = pristine.clone();
            let (unavailable, violation) = crate::write_coordinator::input_tripwire::watch(
                run_root,
                &label,
                enforce_one(outcome, roots, declared, run_root, classifier),
            )
            .await;
            unanswered = match (violation, unavailable) {
                (Some(violation), _) => Some(violation.message()),
                (None, Some(reason)) => Some(reason),
                (None, None) => None,
            };
            if unanswered.is_none() {
                break;
            }
        }
        if let Some(reason) = unanswered {
            *outcome = pristine;
            mark_branch_operational(
                outcome,
                &format!("its declared contract verifiers gave no verdict twice: {reason}"),
            );
        }
    }
}

/// One branch's declared contracts: the first failure demotes it. `Some`
/// when a verifier could not answer at all (it is then left for the caller).
async fn enforce_one(
    outcome: &mut WorkflowV2BranchOutcome,
    roots: &ContractRoots,
    declared: &[serde_json::Value],
    run_root: Option<&std::path::Path>,
    classifier: Option<TypedProgramClassifier>,
) -> Option<String> {
    // A task may declare several contracts and a v3 verification item covers
    // the whole task, so every one has to hold: stop at the first failure,
    // since one violated contract already sinks the branch.
    let mut passed = 0usize;
    let mut shared_floor_count = 0usize;
    let mut typed_command_count = 0usize;
    let mut unverified = Vec::new();
    for contract in declared {
        shared_floor_count += 1;
        let mut verification = run_shared_declarative_floor(roots, contract);
        if matches!(verification, ContractVerification::Passed)
            && let Some(command) =
                crate::v2::deliverable_contract::typed_verification_command(roots, contract)
        {
            typed_command_count += 1;
            verification = run_contract_verifier_for(
                &command,
                CONTRACT_VERIFIER_TIMEOUT,
                run_root,
                false,
                classifier,
            )
            .await;
        }
        match verification {
            ContractVerification::Passed => passed += 1,
            ContractVerification::Unverified(reason) => {
                unverified.push(reason);
                stamp_unverified_contracts(outcome, &unverified);
            }
            ContractVerification::Unavailable(reason) => return Some(reason),
            ContractVerification::Failed(detail, note) => {
                stamp_contract_evaluator(outcome, shared_floor_count, typed_command_count);
                demote_failed_contract(outcome, &detail, run_root);
                if let Some(note) = note {
                    demote::attach_environment_note(outcome, &note);
                }
                return None;
            }
        }
    }
    stamp_contract_evaluator(outcome, shared_floor_count, typed_command_count);
    stamp_passed_contracts(outcome, passed);
    stamp_unverified_contracts(outcome, &unverified);
    None
}

fn stamp_unverified_contracts(outcome: &mut WorkflowV2BranchOutcome, reasons: &[String]) {
    if !reasons.is_empty()
        && let Some(result) = outcome.result.as_mut()
    {
        let mut data = result.data.as_object().cloned().unwrap_or_default();
        data.insert(
            "declared_contract_verification".into(),
            serde_json::json!("unverified"),
        );
        data.insert(
            "declared_contracts_unverified".into(),
            serde_json::json!(reasons),
        );
        result.data = serde_json::Value::Object(data);
    }
}

/// Batch G2: fail a branch as the host's own operational error -- not a
/// verdict on its work. Typed `Execution` with the host's marker, so it is
/// retried and refunded like a dropped transport and never routed to a task
/// as a finding; its untrusted result is dropped.
pub fn mark_branch_operational(outcome: &mut WorkflowV2BranchOutcome, reason: &str) {
    outcome.status = WorkflowV2Status::Failed;
    outcome.failure_kind = Some(BranchFailureKind::Execution);
    outcome.error = Some(format!(
        "{} {reason}",
        crate::error::HOST_OPERATIONAL_ERROR_MARKER
    ));
    outcome.result = None;
}

fn run_shared_declarative_floor(
    roots: &ContractRoots,
    raw_contract: &serde_json::Value,
) -> ContractVerification {
    let contract =
        match serde_json::from_value::<WorkflowV2DeliverableContract>(raw_contract.clone()) {
            Ok(contract) => contract,
            Err(error) => {
                return floor_failed(&[format!(
                    "declared deliverable contract does not parse: {error}"
                )]);
            }
        };
    if let Some(defect) = crate::v2::deliverable_contract::contract_defect(raw_contract) {
        return floor_failed(&[defect]);
    }
    let facts = match collect_declarative_floor_facts(roots, &contract) {
        Ok(facts) => facts,
        Err(error) => {
            return floor_failed(&[format!(
                "host could not collect declared contract facts: {error}"
            )]);
        }
    };
    match evaluate_floor_predicates(&contract, &facts) {
        FloorVerdict::Passed => ContractVerification::Passed,
        FloorVerdict::Failed { findings } => floor_failed(&findings),
    }
}

/// Issue 219: every floor finding reaches the branch's demotion, none
/// dropped (only the first five used to).
fn floor_failed(findings: &[String]) -> ContractVerification {
    ContractVerification::Failed(findings.to_vec(), None)
}

fn stamp_contract_evaluator(
    outcome: &mut WorkflowV2BranchOutcome,
    shared_floor_count: usize,
    typed_command_count: usize,
) {
    let Some(result) = outcome.result.as_mut() else {
        return;
    };
    let evaluator = match (shared_floor_count > 0, typed_command_count > 0) {
        (true, false) => "shared_declarative_floor",
        (true, true) => "shared_declarative_floor_and_typed_command",
        (false, true) => "shared_declarative_floor_and_typed_command",
        (false, false) => return,
    };
    let mut data = result.data.as_object().cloned().unwrap_or_default();
    data.insert(
        "declared_contract_evaluator".to_string(),
        serde_json::json!(evaluator),
    );
    result.data = serde_json::Value::Object(data);
}

/// Record that the host ran this branch's contracts and they held.
///
/// Recording only failures makes the gate unobservable when it works: "no
/// mentions of declared_contract_verification" reads identically whether every
/// contract passed or the verifier never ran at all. That ambiguity is exactly
/// how this enforcement sat dead through several runs while looking fine — so
/// a pass leaves a trace too, and absence of the field now means the host did
/// not check.
pub(super) fn stamp_passed_contracts(outcome: &mut WorkflowV2BranchOutcome, passed: usize) {
    let Some(result) = outcome.result.as_mut() else {
        return;
    };
    let mut data = result.data.as_object().cloned().unwrap_or_default();
    data.insert(
        "declared_contract_verification".to_string(),
        serde_json::json!("passed"),
    );
    data.insert(
        "declared_contracts_verified".to_string(),
        serde_json::json!(passed),
    );
    result.data = serde_json::Value::Object(data);
}

/// Upper bound on one command execution. Failure to finish gets one
/// operational retry, then a host error rather than stalling the fanout.
const CONTRACT_VERIFIER_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);

/// Return the unavailable-program diagnostic only when the existing site
/// classifier says the command could not run, rather than finding a defect.
pub type TypedProgramClassifier = fn(
    &str,
    &crate::acceptance_scratch::CheckResult,
    &std::collections::BTreeMap<String, String>,
) -> Option<String>;

pub(super) enum ContractVerification {
    Unverified(String),
    Passed,
    /// Every finding the verifier or floor reported, none dropped (Issue 219).
    Failed(Vec<String>, Option<String>),
    /// Batch G2: no verdict at all -- the verifier could not be started or
    /// waited on, or did not finish. The environment's, never the branch's.
    Unavailable(String),
}

#[cfg(test)]
pub(super) async fn run_contract_verifier(command: &str) -> ContractVerification {
    run_contract_verifier_within(command, CONTRACT_VERIFIER_TIMEOUT).await
}

#[cfg(test)]
pub(super) async fn run_contract_verifier_within(
    command: &str,
    timeout: std::time::Duration,
) -> ContractVerification {
    run_contract_verifier_for(command, timeout, None, true, None).await
}

#[path = "contracts_run.rs"]
mod run;
use run::run_contract_verifier_for;

/// Every JSON object the verification command printed, in emission order.
///
/// `verification_command` may chain a typed command after the predicates,
/// so stdout can carry more than one verdict and they must
/// all be considered.
pub(super) fn verifier_verdicts(stdout: &str) -> Vec<serde_json::Value> {
    let mut verdicts = Vec::new();
    let mut offset = 0usize;
    while let Some(open) = stdout[offset..].find('{') {
        let start = offset + open;
        let mut stream =
            serde_json::Deserializer::from_str(&stdout[start..]).into_iter::<serde_json::Value>();
        match stream.next() {
            Some(Ok(value)) => {
                // Skip past the object just parsed rather than rescanning the
                // braces nested inside it.
                offset = start + stream.byte_offset().max(1);
                verdicts.push(value);
            }
            // Not the start of a well-formed object; try the next brace.
            _ => offset = start + 1,
        }
    }
    verdicts
}

/// The failures a verdict carries, every one, if it reports any.
///
/// A verdict fails either by saying `status: failed` or by carrying a non-empty
/// `failures` array — the verifier's early exits print the latter with no
/// `status` field at all, and their text is the only account of what broke.
pub(super) fn verdict_failure(verdict: &serde_json::Value) -> Option<Vec<String>> {
    let failed_status = verdict
        .get("status")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|status| status.eq_ignore_ascii_case("failed"));
    let failures = verdict
        .get("failures")
        .and_then(serde_json::Value::as_array)
        .filter(|items| !items.is_empty());
    if !failed_status && failures.is_none() {
        return None;
    }
    let detail: Vec<String> = failures
        .into_iter()
        .flatten()
        .filter_map(serde_json::Value::as_str)
        .map(str::to_string)
        .collect();
    Some(if detail.is_empty() {
        vec!["declared deliverable contract verification failed".to_string()]
    } else {
        detail
    })
}

#[path = "contracts_output.rs"]
mod output;
use output::bounded_output;
#[path = "contracts_demote.rs"]
mod demote;
pub(crate) use demote::demote_failed_contract;

#[cfg(test)]
#[path = "contracts_env_tests.rs"]
mod env_tests;

#[cfg(test)]
#[path = "contracts_native_tests.rs"]
mod native_tests;
