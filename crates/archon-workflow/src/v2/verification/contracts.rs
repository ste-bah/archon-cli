//! Host-executed declared-deliverable-contract enforcement.
//!
//! Split from `normalize` to hold the 500-line source ceiling, along the seam
//! the module already had. `normalize` re-reads a verifier's SELF-REPORT
//! against evidence already in the envelope — commands_run, gap text, coverage
//! — and never leaves the process. Everything here does the opposite: the HOST
//! runs the declared contract's own verifier itself and judges the branch on
//! what that subprocess printed. Different input, different trust model, no
//! shared state; the two sides call nothing of each other's.

use crate::task_universe::WorkflowV2DeliverableContract;
use crate::v2::deliverable_contract::ContractRoots;
use crate::v2::{
    BranchFailureKind, DeclarativeFloorEvaluation, WorkflowV2BranchOutcome, WorkflowV2Evidence,
    WorkflowV2EvidenceKind, WorkflowV2Status, collect_declarative_floor_facts,
    declarative_floor_deferral_reason, evaluate_declarative_floor,
};

/// HOST-EXECUTED deliverable-contract enforcement.
///
/// The contract verifier was previously only handed to the agent as the item's
/// `focused_verification` text and the agent SELF-REPORTED the outcome — so an
/// agent that skipped it (or ran only a weaker typed command) could report
/// "verified" over fabricated artifacts, and every declared contract predicate
/// went unexecuted. A gate the audited party may decline to run is not a gate.
///
/// This runs the SAME host-generated verifier ourselves for every accepted
/// branch whose item declared a contract, and demotes the outcome when it fails.
/// Fail-closed: a verifier that cannot be executed, times out, or emits
/// unparseable output demotes too — "we could not check" is never a pass.
///
/// Domain-agnostic: the contract declares its own artifact paths and predicates;
/// this only runs the command and reads the JSON verdicts from its stdout.
/// Each item's contracts resolve under its [`ContractRoots`] — project artifact
/// root first, then the target repository root (Issue-22).
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
                enforce_one(outcome, roots, declared, run_root),
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
) -> Option<String> {
    // A task may declare several contracts and a v3 verification item covers
    // the whole task, so every one has to hold: stop at the first failure,
    // since one violated contract already sinks the branch.
    let mut passed = 0usize;
    let mut shared_floor_count = 0usize;
    let mut generated_count = 0usize;
    for contract in declared {
        let verification = match run_shared_declarative_floor(roots, contract) {
            Some(verification) => {
                shared_floor_count += 1;
                verification
            }
            None => {
                generated_count += 1;
                let command =
                    crate::v2::deliverable_contract::verification_command(roots, contract);
                run_contract_verifier_for(&command, CONTRACT_VERIFIER_TIMEOUT, run_root).await
            }
        };
        match verification {
            ContractVerification::Passed => passed += 1,
            ContractVerification::Unavailable(reason) => return Some(reason),
            ContractVerification::Failed(detail) => {
                stamp_contract_evaluator(outcome, shared_floor_count, generated_count);
                demote_failed_contract(outcome, &detail, run_root);
                return None;
            }
        }
    }
    stamp_contract_evaluator(outcome, shared_floor_count, generated_count);
    stamp_passed_contracts(outcome, passed);
    None
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
) -> Option<ContractVerification> {
    let contract =
        serde_json::from_value::<WorkflowV2DeliverableContract>(raw_contract.clone()).ok()?;
    if crate::v2::deliverable_contract::contract_defect(raw_contract).is_some()
        || declarative_floor_deferral_reason(&contract).is_some()
    {
        return None;
    }
    let facts = match collect_declarative_floor_facts(roots, &contract) {
        Ok(facts) => facts,
        Err(error) => {
            return Some(ContractVerification::Failed(vec![format!(
                "host could not collect declared contract facts: {error}"
            )]));
        }
    };
    Some(match evaluate_declarative_floor(&contract, &facts) {
        DeclarativeFloorEvaluation::Passed => ContractVerification::Passed,
        DeclarativeFloorEvaluation::Failed { findings } => floor_failed(&findings),
        DeclarativeFloorEvaluation::Deferred { .. } => return None,
    })
}

/// Issue 219: every floor finding reaches the branch's demotion, none
/// dropped (only the first five used to).
fn floor_failed(findings: &[String]) -> ContractVerification {
    ContractVerification::Failed(findings.to_vec())
}

fn stamp_contract_evaluator(
    outcome: &mut WorkflowV2BranchOutcome,
    shared_floor_count: usize,
    generated_count: usize,
) {
    let Some(result) = outcome.result.as_mut() else {
        return;
    };
    let evaluator = match (shared_floor_count > 0, generated_count > 0) {
        (true, false) => "shared_declarative_floor",
        (true, true) => "shared_declarative_floor_and_generated_verifier",
        (false, true) => "generated_verifier",
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

/// Upper bound on a single declared-contract verification. The verifier only
/// reads JSON/JSONL, so overrunning this means it is wedged rather than slow;
/// the branch is then demoted as unverified instead of stalling the fanout.
const CONTRACT_VERIFIER_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);

pub(super) enum ContractVerification {
    Passed,
    /// Every finding the verifier or floor reported, none dropped (Issue 219).
    Failed(Vec<String>),
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
    run_contract_verifier_for(command, timeout, None).await
}

/// Batch G2: for a run (`run_root`), under the host's OS write boundary
/// (`write_coordinator::host_sandbox`): every host root sealed, nothing
/// re-opened -- a contract verifier only reads.
pub(super) async fn run_contract_verifier_for(
    command: &str,
    timeout: std::time::Duration,
    run_root: Option<&std::path::Path>,
) -> ContractVerification {
    // Fed to the shell on stdin rather than as `-c <command>`.
    //
    // The generated deliverable verifier embeds a ~29 KB Python program, and
    // Windows caps a whole command line at 32,767 characters. Passed as an
    // argument it was silently truncated mid-script by CreateProcess, so the
    // heredoc never met its terminator and Python died on a severed statement
    // ("here-document delimited by end-of-file", then a SyntaxError). Linux
    // allows roughly 2 MB, which is why this only ever failed on Windows.
    //
    // stdin has no such limit, and for a generated script the semantics are
    // the same -- nothing here depends on `$0` or positional arguments.
    // Issue-227/234: `boundary` lives until the verifier has been reaped, then
    // `finish` restores and names any change to a sealed root on a host with no
    // kernel boundary (a no-op where the kernel refused the write live).
    let (mut process, boundary) = match crate::write_coordinator::host_sandbox::command(
        archon_shell::resolve_posix_shell(),
        run_root,
        &[],
    ) {
        Ok((command, boundary)) => (tokio::process::Command::from(command), boundary),
        Err(reason) => return ContractVerification::Unavailable(reason),
    };
    process
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    // Batch G2: its own process group, so a timeout reaps what it started.
    #[cfg(unix)]
    process.process_group(0);
    let mut child = match process.spawn() {
        Ok(child) => child,
        Err(error) => {
            return ContractVerification::Unavailable(format!(
                "host could not execute the declared contract verifier: {error}"
            ));
        }
    };
    let pid = child.id();
    // Issue-134: dropped mid-run, the whole group goes with it.
    let _group = crate::v2::write::test_baseline_run::GroupKillOnDrop(pid);
    if let Some(mut stdin) = child.stdin.take() {
        use tokio::io::AsyncWriteExt as _;
        let script = command.to_string();
        // Ignore write failures: the child may have exited already, and the
        // output/exit status below is what decides the verdict either way.
        let _ = stdin.write_all(script.as_bytes()).await;
        let _ = stdin.shutdown().await;
    }
    let run = bounded_output(child);
    let output = match tokio::time::timeout(timeout, run).await {
        Ok(Ok(output)) => output,
        // Fail closed: an unrunnable verifier is not evidence of success --
        // and not evidence of failure either (Batch G2).
        Ok(Err(error)) => {
            crate::v2::write::test_baseline_run::kill_group(pid);
            return ContractVerification::Unavailable(format!(
                "host could not execute the declared contract verifier: {error}"
            ));
        }
        Err(_) => {
            crate::v2::write::test_baseline_run::kill_group(pid);
            return ContractVerification::Unavailable(format!(
                "declared contract verifier did not finish within {}s; treating as unverified",
                timeout.as_secs()
            ));
        }
    };
    // Reap anything the verifier left running behind it.
    crate::v2::write::test_baseline_run::kill_group(pid);
    // Issue-234: on a host with no kernel boundary, restore any sealed root the
    // verifier changed. A change means the tree it was judged against is not the
    // one the host sealed, so there is no trusted verdict: re-run it.
    if let Err(reason) = boundary.finish("the declared contract verifier for this run") {
        return ContractVerification::Unavailable(reason);
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let verdicts = verifier_verdicts(&stdout);
    // Issue 219: past the read cap the verdict cannot be read whole. What
    // was read still counts; a pass never does.
    let cut = (output.stdout_total > output.stdout.len() as u64).then(|| {
        format!(
            "declared contract verifier output cut at {} of {} bytes [output cut]: any failure printed after the cut is unread, and its verdict is not trusted as a pass",
            output.stdout.len(),
            output.stdout_total
        )
    });
    // Any stage that reported a failure demotes the branch, whichever one it
    // was. Returning on the FIRST verdict instead would let the typed
    // pre-check's permissive `{"status":"verified"}` mask the contract
    // verifier's own failure printed after it.
    let failures: Vec<String> = verdicts
        .iter()
        .filter_map(verdict_failure)
        .flatten()
        .collect();
    if !failures.is_empty() || cut.is_some() {
        return ContractVerification::Failed(failures.into_iter().chain(cut).collect());
    }
    if !output.status.success() {
        // Its end and its failure lines (stdout when stderr is silent),
        // well within one finding's 4 KB gap bound (`contracts_demote`).
        let said = if output.stderr.trim_ascii().is_empty() {
            &output.stdout
        } else {
            &output.stderr
        };
        return ContractVerification::Failed(vec![format!(
            "declared contract verifier exited non-zero: {}",
            crate::failure_evidence::failure_evidence(said, 420)
        )]);
    }
    // The contract verifier is appended last, so the final status-bearing
    // object is its verdict. Requiring one keeps silence from counting as a
    // pass.
    if verdicts.iter().rev().any(|verdict| {
        verdict
            .get("status")
            .and_then(serde_json::Value::as_str)
            .is_some()
    }) {
        return ContractVerification::Passed;
    }
    ContractVerification::Failed(vec![
        "declared contract verifier produced no parseable status; treating as unverified"
            .to_string(),
    ])
}

/// Every JSON object the verification command printed, in emission order.
///
/// `verification_command` may chain a typed pre-check ahead of the contract
/// verifier, so stdout routinely carries more than one verdict and they must
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
