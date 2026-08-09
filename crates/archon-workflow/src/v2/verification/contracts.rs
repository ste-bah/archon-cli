//! Host-executed declared-deliverable-contract enforcement.
//!
//! Split from `normalize` to hold the 500-line source ceiling, along the seam
//! the module already had. `normalize` re-reads a verifier's SELF-REPORT
//! against evidence already in the envelope — commands_run, gap text, coverage
//! — and never leaves the process. Everything here does the opposite: the HOST
//! runs the declared contract's own verifier itself and judges the branch on
//! what that subprocess printed. Different input, different trust model, no
//! shared state; the two sides call nothing of each other's.

use crate::v2::{
    BranchFailureKind, WorkflowV2BranchOutcome, WorkflowV2Evidence, WorkflowV2EvidenceKind,
    WorkflowV2Status,
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
pub async fn enforce_declared_contracts(
    outcomes: &mut [WorkflowV2BranchOutcome],
    contracts: &std::collections::BTreeMap<String, (String, Vec<serde_json::Value>)>,
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
        let Some((root, declared)) = contracts.get(&outcome.item_id) else {
            continue;
        };
        // A task may declare several contracts and a v3 verification item
        // covers the whole task, so every one has to hold: stop at the first
        // failure, since one violated contract already sinks the branch.
        let mut passed = 0usize;
        let mut failed = false;
        for contract in declared {
            let command = crate::v2::deliverable_contract::verification_command(root, contract);
            match run_contract_verifier(&command).await {
                ContractVerification::Passed => passed += 1,
                ContractVerification::Failed(detail) => {
                    demote_failed_contract(outcome, &detail);
                    failed = true;
                    break;
                }
            }
        }
        if !failed {
            stamp_passed_contracts(outcome, passed);
        }
    }
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
    Failed(String),
}

pub(super) async fn run_contract_verifier(command: &str) -> ContractVerification {
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
    let mut child = match tokio::process::Command::new(archon_shell::resolve_posix_shell())
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
    {
        Ok(child) => child,
        Err(error) => {
            return ContractVerification::Failed(format!(
                "host could not execute the declared contract verifier: {error}"
            ));
        }
    };
    if let Some(mut stdin) = child.stdin.take() {
        use tokio::io::AsyncWriteExt as _;
        let script = command.to_string();
        // Ignore write failures: the child may have exited already, and the
        // output/exit status below is what decides the verdict either way.
        let _ = stdin.write_all(script.as_bytes()).await;
        let _ = stdin.shutdown().await;
    }
    let run = child.wait_with_output();
    let output = match tokio::time::timeout(CONTRACT_VERIFIER_TIMEOUT, run).await {
        Ok(Ok(output)) => output,
        // Fail closed: an unrunnable verifier is not evidence of success.
        Ok(Err(error)) => {
            return ContractVerification::Failed(format!(
                "host could not execute the declared contract verifier: {error}"
            ));
        }
        Err(_) => {
            return ContractVerification::Failed(format!(
                "declared contract verifier did not finish within {}s; treating as unverified",
                CONTRACT_VERIFIER_TIMEOUT.as_secs()
            ));
        }
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    let verdicts = verifier_verdicts(&stdout);
    // Any stage that reported a failure demotes the branch, whichever one it
    // was. Returning on the FIRST verdict instead would let the typed
    // pre-check's permissive `{"status":"verified"}` mask the contract
    // verifier's own failure printed after it.
    if let Some(detail) = verdicts.iter().find_map(verdict_failure) {
        return ContractVerification::Failed(detail);
    }
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return ContractVerification::Failed(format!(
            "declared contract verifier exited non-zero: {}",
            stderr.trim().chars().take(300).collect::<String>()
        ));
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
    ContractVerification::Failed(
        "declared contract verifier produced no parseable status; treating as unverified"
            .to_string(),
    )
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

/// The failure text a verdict carries, if it reports one.
///
/// A verdict fails either by saying `status: failed` or by carrying a non-empty
/// `failures` array — the verifier's early exits print the latter with no
/// `status` field at all, and their text is the only account of what broke.
pub(super) fn verdict_failure(verdict: &serde_json::Value) -> Option<String> {
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
    let detail = failures
        .map(|items| {
            items
                .iter()
                .filter_map(serde_json::Value::as_str)
                .take(5)
                .collect::<Vec<_>>()
                .join("; ")
        })
        .unwrap_or_default();
    Some(if detail.is_empty() {
        "declared deliverable contract verification failed".to_string()
    } else {
        detail
    })
}

pub(super) fn demote_failed_contract(outcome: &mut WorkflowV2BranchOutcome, detail: &str) {
    let truncated: String = detail.chars().take(500).collect();
    if let Some(result) = outcome.result.as_mut() {
        result.status = WorkflowV2Status::NeedsReview;
        result.residual_gaps.push(crate::WorkflowV2ResidualGap {
            id: "declared_contract_verification_failed".to_string(),
            description: format!(
                "host-executed declared deliverable contract verification failed: {truncated}"
            ),
            severity: Some("review".to_string()),
        });
        result.evidence.push(WorkflowV2Evidence::new(
            WorkflowV2EvidenceKind::Blocker,
            "accepted branch demoted: the host ran the declared deliverable contract verifier and it failed",
        ));
        let mut data = result.data.as_object().cloned().unwrap_or_default();
        data.insert(
            "declared_contract_verification".to_string(),
            serde_json::json!("failed"),
        );
        data.insert(
            "verification_failure_class".to_string(),
            serde_json::json!("declared_contract_violation"),
        );
        result.data = serde_json::Value::Object(data);
    }
    outcome.status = WorkflowV2Status::NeedsReview;
    outcome.failure_kind = Some(BranchFailureKind::Semantic);
}
