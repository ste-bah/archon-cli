use std::path::Path;

use anyhow::Result;
use archon_workflow::task_set_contract::JudgeDecision;
use serde::{Deserialize, Serialize};

use super::Decision;

pub(crate) fn audited_verdict(
    client: &dyn super::WorkflowLlmClient,
    model: &str,
    contract: &super::AcceptanceContract,
    id: &str,
    runs: &super::BaselineRuns,
    prd_text: &str,
    directory: &Path,
) -> Option<(String, String)> {
    let requirements =
        archon_workflow::v2::acceptance_stage::coverage::prd_requirement_texts(prd_text);
    let redactor = super::evidence::Redactor::for_runs(runs);
    let candidate = super::candidates(
        client,
        model,
        contract,
        &std::collections::BTreeSet::from([id.to_string()]),
        runs,
        &requirements,
        &redactor,
    )
    .into_iter()
    .next()?;
    let decision = load(directory, &candidate.key, id)?;
    Some((format!("{:?}", decision.verdict), decision.reason))
}

#[derive(Serialize)]
struct AuditDecision<'a> {
    input_digest: &'a str,
    check_id: &'a str,
    verdict: JudgeDecision,
    reason: &'a str,
    counterexample: &'a str,
    baseline_exit_code: Option<i32>,
}

#[derive(Deserialize)]
struct SavedAudit {
    input_digest: String,
    check_id: String,
    verdict: JudgeDecision,
    reason: String,
    counterexample: String,
}

pub(super) fn load(directory: &Path, input_digest: &str, check_id: &str) -> Option<Decision> {
    for entry in std::fs::read_dir(directory).ok()? {
        let path = entry.ok()?.path();
        let Ok(bytes) = std::fs::read(path) else {
            continue;
        };
        let Ok(saved) = serde_json::from_slice::<SavedAudit>(&bytes) else {
            continue;
        };
        if saved.input_digest == input_digest && saved.check_id == check_id {
            return Some(Decision {
                verdict: saved.verdict,
                counterexample: saved.counterexample,
                reason: saved.reason,
                sampling: Some(serde_json::json!({"provenance": "audit"})),
            });
        }
    }
    None
}

pub fn verdict_key(instruction: &str, judgment_input: serde_json::Value) -> String {
    archon_workflow::task_set_contract::content_digest(
        serde_json::json!(["acceptance-passability-v4", instruction, judgment_input])
            .to_string()
            .as_bytes(),
    )
}

pub(super) fn write(
    directory: &Path,
    input_digest: &str,
    check_id: &str,
    decision: &Decision,
    baseline_exit_code: Option<i32>,
) -> Result<()> {
    let audit = AuditDecision {
        input_digest,
        check_id,
        verdict: decision.verdict,
        reason: &decision.reason,
        counterexample: &decision.counterexample,
        baseline_exit_code,
    };
    let name = format!(
        "{input_digest}.{}.{}.json",
        archon_workflow::task_set_contract::content_digest(check_id.as_bytes()),
        uuid::Uuid::new_v4()
    );
    crate::command::workflow_task_set::reauthor::cache::write_atomic(
        &directory.join(name),
        &serde_json::to_vec(&audit)?,
    )
}
