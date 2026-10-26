use std::path::Path;

use anyhow::Context;
use archon_workflow::WorkflowLlmClient;
use archon_workflow::task_set_contract::{AcceptanceContract, JudgeDecision};

use super::super::{BaselineRuns, CANNOT_PASS, Decision, evidence::Redactor};

/// A stall-only host finding for a failed check that gives its owner no
/// output to diagnose. The decision is persisted through the usual audit
/// gate with base-tree evidence.
pub(crate) fn silent_stall_finding(
    client: &dyn WorkflowLlmClient,
    model: &str,
    contract: &AcceptanceContract,
    id: &str,
    runs: &BaselineRuns,
    prd_text: &str,
    audit_dir: &Path,
    trailing_rounds: usize,
    implemented_commit: &str,
) -> anyhow::Result<Option<String>> {
    let requirements =
        archon_workflow::v2::acceptance_stage::coverage::prd_requirement_texts(prd_text);
    let redactor = Redactor::for_runs(runs);
    let Some(candidate) = super::super::candidates(
        client,
        model,
        contract,
        &std::collections::BTreeSet::from([id.to_string()]),
        runs,
        &requirements,
        &redactor,
    )
    .into_iter()
    .next() else {
        return Ok(None);
    };
    let Some(result) = runs.failures.get(id) else {
        return Ok(None);
    };
    let exit = result
        .result
        .exit_code
        .map_or_else(|| "unknown".to_string(), |code| code.to_string());
    // The finding starts with the passability marker so the router sends it
    // to re-authoring; what follows states only what both runs showed.
    let reason = format!(
        "it fails without output: on the implemented tree ({implemented_commit}) it failed in {trailing_rounds} trailing round(s) without progress, and on the base tree ({}) it failed with exit {exit}; both runs printed nothing on stdout or stderr, so no owner can see what fails; rewrite the check so every failing path prints what it found, and keep every assertion of its criterion",
        runs.commit
    );
    let decision = Decision {
        verdict: JudgeDecision::Refuted,
        counterexample: "the check printed nothing on stdout or stderr".into(),
        reason: reason.clone(),
        sampling: Some(serde_json::json!({"provenance": "host"})),
    };
    super::super::audit::write(
        audit_dir,
        &candidate.key,
        id,
        &decision,
        result.result.exit_code,
    )
    .with_context(|| format!("writing silent-stall passability audit for check {id}"))?;
    Ok(Some(format!("check '{id}': {CANNOT_PASS}: {reason}")))
}
