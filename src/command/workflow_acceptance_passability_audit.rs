use std::path::Path;

use anyhow::Result;
use archon_workflow::task_set_contract::JudgeDecision;
use serde::Serialize;

use super::Decision;

#[derive(Serialize)]
struct AuditDecision<'a> {
    input_digest: &'a str,
    check_id: &'a str,
    verdict: JudgeDecision,
    reason: &'a str,
    counterexample: &'a str,
    baseline_exit_code: Option<i32>,
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
