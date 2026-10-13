//! #284: the gate findings a fixed decomposition's host commands reported,
//! as learning lessons.
use archon_workflow::{StageStatus, Verification, WorkflowLearningRecord};
use serde_json::Value;

use super::{digest, redact_lesson};

/// The findings a host command's gate reported, one lesson each.
///
/// A trusted host command (an acceptance freeze, a requirements trace, a
/// task-set lint, a body landing) reports what it refused as
/// `gateEnvelope.policy_findings[]`, each with the finding's `text`, its
/// `subject` and, for a deterministic defect, a `code`. That text carries the
/// judge's reason and counterexample for a refuted check, the refuted claim of
/// a task, and every other gate finding -- the lessons a later author most
/// needs. A lesson is keyed by its content, so a finding repeated over
/// attempts is one lesson, however many calls report it.
pub(crate) fn gate_finding_records(
    run_id: &str,
    command_id: &str,
    source: &Value,
    hooks: &[String],
) -> Vec<WorkflowLearningRecord> {
    let findings = source
        .get("gateEnvelope")
        .and_then(|envelope| envelope.get("policy_findings"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten();
    let mut records = Vec::new();
    for finding in findings {
        let Some(text) = finding.get("text").and_then(Value::as_str) else {
            continue;
        };
        if text.trim().is_empty() {
            continue;
        }
        let subject = finding
            .get("subject")
            .and_then(Value::as_str)
            .unwrap_or("the set");
        let code = finding
            .get("deterministic_defect")
            .and_then(|defect| defect.get("code"))
            .and_then(Value::as_str);
        let lower = text.to_ascii_lowercase();
        let kind = if lower.contains("refuted by the host judge") {
            "refutation"
        } else if lower.contains("cannot pass as written") {
            "cannot_pass"
        } else if lower.contains("refuted") {
            "claim_refutation"
        } else {
            "gate_finding"
        };
        let name = redact_lesson(&format!(
            "{command_id} {kind} on {subject}{}: {text}",
            code.map(|code| format!(" ({code})")).unwrap_or_default()
        ));
        if name.trim().is_empty() {
            continue;
        }
        records.push(WorkflowLearningRecord {
            run_id: run_id.to_string(),
            name,
            stage_id: format!(
                "decomposition-{}-finding-{}",
                digest(run_id),
                digest(&format!("{command_id}\n{subject}\n{text}"))
            ),
            phase: "decomposition_lesson".into(),
            agent: None,
            status: StageStatus::Failed,
            verification: Verification::Failed,
            durable: false,
            quality_score: None,
            artifact_refs: vec![kind.to_string()],
            telemetry: archon_workflow::StageTelemetry {
                attempt: 1,
                error_class: None,
                artifact_count: 0,
            },
            trace_ref: None,
            hooks: hooks.to_vec(),
            ts: chrono::Utc::now(),
        });
    }
    records
}
