//! Pre-flight for the acceptance stage (Obs-32): an authored script must end
//! with `await acceptance(...)`, and the executed run must have reached it.
//!
//! Keyed on a schema marker the author emits (`meta.schema`), so scripts
//! persisted by older runs — which predate the stage — keep resuming, while
//! every newly authored script is held to it: the draft pre-flight requires
//! the marker itself, so a new author cannot dodge the stage by omitting it.

use super::*;
use crate::v2::acceptance_stage::{ACCEPTANCE_STAGE_CALL_PREFIX, ACCEPTANCE_STAGE_TOOL};

/// The first `meta.schema` that carries the acceptance stage requirement.
pub const AUTHORED_SCRIPT_SCHEMA_ACCEPTANCE: u32 = 2;

/// The `schema` the script's `meta` declaration states, if any. Read from the
/// meta statement's text: the reference shows a literal object and the dry
/// run never hands the evaluated object back.
pub fn authored_script_schema(source: &str) -> Option<u32> {
    let start = workflow_meta_marker_offset(source)?;
    let end = statement_end_offset(source, start);
    let meta = &source[start..end];
    let re = regex::Regex::new(r#"\bschema\s*:\s*(\d+)"#).expect("static regex");
    re.captures(meta)
        .and_then(|caps| caps.get(1))
        .and_then(|digits| digits.as_str().parse().ok())
}

/// Whether the schema marker puts a script under the acceptance-stage rule.
pub fn requires_acceptance_stage(source: &str) -> bool {
    authored_script_schema(source).is_some_and(|schema| schema >= AUTHORED_SCRIPT_SCHEMA_ACCEPTANCE)
}

pub fn is_acceptance_stage_call(call: &WorkflowV2HostCall) -> bool {
    call.method == WorkflowV2HostMethod::Tool
        && call
            .options
            .extra
            .get("tool")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|tool| tool == ACCEPTANCE_STAGE_TOOL)
        && call.id.starts_with(ACCEPTANCE_STAGE_CALL_PREFIX)
}

/// The defect a draft without the marker is rejected for.
pub fn schema_marker_defect(source: &str) -> Option<String> {
    (!requires_acceptance_stage(source)).then(|| {
        format!(
            "the `export const meta` declaration must carry `schema: {AUTHORED_SCRIPT_SCHEMA_ACCEPTANCE}` (found {}) — it marks a script authored under the acceptance-stage rule",
            authored_script_schema(source).map_or("no schema".to_string(), |s| s.to_string())
        )
    })
}

/// Ordering defects of the acceptance stage over a planned or executed call
/// sequence. Empty when the stage is present and final.
///
/// Rules: at least one acceptance round; the first round after every final
/// review reduce and after every task-work call; between rounds only
/// checkpoints and review-remediation calls (the fix + re-verify the stage
/// itself dispatches); after the last round nothing but checkpoints.
pub fn acceptance_stage_defects(calls: &[WorkflowV2HostCall]) -> Vec<String> {
    let mut defects = Vec::new();
    let rounds: Vec<usize> = calls
        .iter()
        .enumerate()
        .filter(|(_, call)| is_acceptance_stage_call(call))
        .map(|(index, _)| index)
        .collect();
    let (Some(&first), Some(&last)) = (rounds.first(), rounds.last()) else {
        defects.push(format!(
            "the script has no acceptance stage: after review remediation, as the FINAL stage, call `const acceptance_gate = await acceptance({{ taskFileFor, targetFilesFor }})` — it runs every check in the task set's frozen acceptance-contract.json against the finished repository (host call `{ACCEPTANCE_STAGE_TOOL}`), routes failing checks to the tasks that implement them, and the run cannot complete without it"
        ));
        return defects;
    };
    if let Some(last_reduce) = calls
        .iter()
        .rposition(|call| matches!(review_contract_stage(call), Some(REVIEW_REDUCE_FINAL_STAGE)))
        && last_reduce > first
    {
        defects.push(format!(
            "the acceptance stage `{}` runs BEFORE the final review reduce `{}` — acceptance is the last stage, after both mandatory reviews and their remediation",
            calls[first].id, calls[last_reduce].id
        ));
    }
    if let Some(last_work) = calls.iter().rposition(is_task_work_call)
        && last_work > first
    {
        defects.push(format!(
            "task work `{}` runs AFTER the acceptance stage `{}` started — every implement/verify/remediate call precedes acceptance",
            calls[last_work].id, calls[first].id
        ));
    }
    for call in &calls[first + 1..=last] {
        if is_acceptance_stage_call(call)
            || call.method == WorkflowV2HostMethod::Checkpoint
            || is_review_remediation_call(call)
        {
            continue;
        }
        defects.push(format!(
            "call `{}` (w.{}) runs between acceptance rounds — only the stage's own fix + re-verify remediation may run there",
            call.id,
            call.method.as_str()
        ));
    }
    for call in &calls[last + 1..] {
        if call.method == WorkflowV2HostMethod::Checkpoint {
            continue;
        }
        defects.push(format!(
            "call `{}` (w.{}) runs AFTER the final acceptance round `{}` — nothing but the accounting `return` follows acceptance",
            call.id,
            call.method.as_str(),
            calls[last].id
        ));
    }
    defects
}

/// The executed call sequence of a schema-marked script must contain the
/// stage in its final position; a live path that skipped it (a conditional
/// around the call) is a divergence from the plan the pre-flight accepted.
pub fn validate_executed_acceptance_stage(
    source: &str,
    calls: &[WorkflowV2HostCall],
) -> WorkflowResult<()> {
    if !requires_acceptance_stage(source) {
        return Ok(());
    }
    let defects = acceptance_stage_defects(calls);
    if defects.is_empty() {
        return Ok(());
    }
    Err(WorkflowError::SpecInvalid(format!(
        "the executed run did not end with the acceptance stage ({}); the live call sequence diverged from the pre-flight plan",
        defects.join("; AND ")
    )))
}

#[cfg(test)]
#[path = "v3_author_acceptance_tests.rs"]
mod tests;
