use serde_json::Value;

use crate::error::WorkflowResult;
use crate::fanout;
use crate::persistence;
use crate::run::{StageStatus, WorkflowRun};
use crate::runner::StageRunOutput;
use crate::spec::StageSpec;
use crate::store::WorkflowStore;

pub(super) fn is_blocked_evidence(stage: &StageSpec, body: &str) -> bool {
    is_repair_stage(stage) && blocked_body_has_required_evidence(body)
}

pub(super) fn is_accepted_report_evidence(stage: &StageSpec, body: &str) -> bool {
    is_repair_stage(stage) && accepted_report_body_has_required_evidence(body)
}

pub(super) fn record_evidence(
    store: &WorkflowStore,
    run: &mut WorkflowRun,
    stage: &StageSpec,
    item_id: String,
    output: StageRunOutput,
) -> WorkflowResult<()> {
    let artifact = persistence::write_attached_stage_artifact(
        store,
        run,
        stage,
        &item_id,
        &output.extension,
        output.body.clone(),
        false,
    )?;
    persistence::record_blocked_agent_output(
        store,
        &run.id,
        &stage.id,
        &item_id,
        Some(&output),
        Some(&artifact),
        Some("blocked evidence supplied"),
    )?;
    fanout::record_item(
        run,
        stage,
        item_id,
        StageStatus::Blocked,
        Some(artifact),
        Some("blocked evidence supplied".to_string()),
    );
    Ok(())
}

fn is_repair_stage(stage: &StageSpec) -> bool {
    stage
        .extra
        .get("artifact_self_heal")
        .and_then(Value::as_bool)
        .unwrap_or(false)
        && stage.id.starts_with("repair-required-artifacts")
}

fn blocked_body_has_required_evidence(body: &str) -> bool {
    let Ok(value) = serde_json::from_str::<Value>(body) else {
        return blocked_text_has_required_evidence(body);
    };
    status_is(&value, "blocked")
        && has_non_empty(&value, &["artifact_path", "resolved_path", "artifact"])
        && has_non_empty(&value, &["reason", "missing_evidence", "action_required"])
        && has_non_empty(
            &value,
            &[
                "commands_run",
                "attempted_commands",
                "generation_attempts",
                "command_discovery",
            ],
        )
}

fn accepted_report_body_has_required_evidence(body: &str) -> bool {
    let Ok(value) = serde_json::from_str::<Value>(body) else {
        return false;
    };
    if !status_is(&value, "accepted") {
        return false;
    }
    if idempotent_noop_with_evidence(&value) {
        return true;
    }
    has_non_empty(&value, &["artifact", "artifact_path", "resolved_path"])
        && has_non_empty(&value, &["evidence", "reason", "created", "summary"])
}

fn idempotent_noop_with_evidence(value: &Value) -> bool {
    value
        .get("idempotent_noop")
        .and_then(Value::as_bool)
        .unwrap_or(false)
        && has_non_empty(value, &["evidence", "reason", "summary"])
}

fn status_is(value: &Value, expected: &str) -> bool {
    value
        .get("status")
        .and_then(Value::as_str)
        .map(str::trim)
        .is_some_and(|status| status.eq_ignore_ascii_case(expected))
}

fn has_non_empty(value: &Value, fields: &[&str]) -> bool {
    fields
        .iter()
        .any(|field| value.get(*field).is_some_and(value_has_content))
}

fn value_has_content(value: &Value) -> bool {
    match value {
        Value::Array(values) => !values.is_empty(),
        Value::Object(fields) => !fields.is_empty(),
        Value::String(value) => !value.trim().is_empty(),
        Value::Bool(_) | Value::Number(_) => true,
        Value::Null => false,
    }
}

fn blocked_text_has_required_evidence(body: &str) -> bool {
    let lower = body.to_ascii_lowercase();
    lower.contains("status: blocked")
        && (lower.contains("artifact_path:") || lower.contains("resolved_path:"))
        && (lower.contains("reason:") || lower.contains("missing_evidence:"))
        && (lower.contains("commands_run:")
            || lower.contains("attempted_commands:")
            || lower.contains("generation_attempts:")
            || lower.contains("command_discovery:"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    use crate::spec::{RetryPolicy, StageKind, StageSpec};

    #[test]
    fn blocked_repair_evidence_requires_artifact_and_reason() {
        let stage = repair_stage();
        assert!(is_blocked_evidence(
            &stage,
            r#"{"status":"blocked","artifact_path":"a.json","reason":"missing data","commands_run":[{"command":"make artifact","exit_status":1}]}"#
        ));
        assert!(!is_blocked_evidence(
            &stage,
            r#"{"status":"blocked","artifact_path":"a.json","reason":"missing data"}"#
        ));
        assert!(!is_blocked_evidence(
            &stage,
            r#"{"status":"blocked","reason":"missing data","commands_run":[{"command":"make artifact","exit_status":1}]}"#
        ));
        assert!(!is_blocked_evidence(
            &stage,
            "status: blocked\nartifact_path: a.json\nreason: missing data"
        ));
        assert!(is_blocked_evidence(
            &stage,
            "status: blocked\nartifact_path: a.json\nreason: missing data\ncommands_run:\n- command: make artifact\n  exit_status: 1"
        ));
    }

    #[test]
    fn accepted_report_evidence_allows_report_artifacts_only() {
        let stage = repair_stage();
        assert!(is_accepted_report_evidence(
            &stage,
            r#"{"status":"accepted","artifact":"report.md","evidence":"created from inventory"}"#
        ));
        assert!(is_accepted_report_evidence(
            &stage,
            r#"{"status":"accepted","idempotent_noop":true,"evidence":"target already exists"}"#
        ));
        assert!(!is_accepted_report_evidence(
            &stage,
            r#"{"status":"accepted","artifact":"report.md"}"#
        ));
        assert!(!is_accepted_report_evidence(
            &stage,
            r#"{"status":"accepted","idempotent_noop":true}"#
        ));
    }

    fn repair_stage() -> StageSpec {
        let mut stage = StageSpec {
            id: "repair-required-artifacts".into(),
            kind: StageKind::Fanout,
            task: None,
            agent: None,
            foreach: None,
            reducer: None,
            tool: None,
            condition: None,
            depends_on: Vec::new(),
            provider_tier: None,
            retry: RetryPolicy::default(),
            input: Value::Null,
            model: None,
            provider: None,
            expected_target_files: Vec::new(),
            verify_command: None,
            max_parallelism: None,
            item_kind: None,
            filter: None,
            extra: BTreeMap::new(),
        };
        stage
            .extra
            .insert("artifact_self_heal".into(), Value::Bool(true));
        stage
    }
}
