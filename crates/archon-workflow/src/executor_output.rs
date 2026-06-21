//! Stage output helpers split out of `executor.rs` to keep that file within the
//! 500-line module budget. These are pure functions over a stage spec / output
//! body with no executor state.

use crate::context;
use crate::context_output;
use crate::error::{WorkflowError, WorkflowResult};
use crate::spec::{StageKind, StageSpec};

/// Render the deterministic (no-live-runner) artifact body for a stage.
///
/// A stage that declares itself a structured fan-out items producer must emit a
/// parseable `items:` document even in the deterministic path, otherwise
/// downstream `foreach` fan-outs would fail-fast with no items.
pub(crate) fn deterministic_stage_output(stage: &StageSpec) -> String {
    if crate::spec::stage_declares_items_producer(stage) {
        if deterministic_empty_items(stage) {
            return r#"{"items":[]}"#.to_string();
        }
        return format!(
            r#"{{"items":[{{"stage":"{}","deterministic":true}}]}}"#,
            stage.id
        );
    }
    format!(
        "# Stage {}\n\nKind: `{:?}`\nAgent: `{}`\n",
        stage.id,
        stage.kind,
        stage.agent.as_deref().unwrap_or("none")
    )
}

fn deterministic_empty_items(stage: &StageSpec) -> bool {
    stage
        .extra
        .get("deterministic_empty_items")
        .or_else(|| stage.input.get("deterministic_empty_items"))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
}

/// Reject a stage output body that self-reports blocked, failed, or
/// unverifiable status before it can be accepted as a usable artifact.
pub(crate) fn ensure_output_usable(body: &str) -> WorkflowResult<()> {
    ensure_output_usable_for_contract(OutputContract::Implementation, body)
}

pub(crate) fn ensure_stage_output_usable(stage: &StageSpec, body: &str) -> WorkflowResult<()> {
    ensure_declared_items_output(stage, body)?;
    ensure_output_usable_for_contract(stage_output_contract(stage), body)
}

pub(crate) fn ensure_fanout_item_output_usable(
    stage: &StageSpec,
    body: &str,
) -> WorkflowResult<()> {
    let contract = if stage.effective_item_kind() == StageKind::Implementation {
        OutputContract::Implementation
    } else if review_like_stage(stage) {
        OutputContract::ReviewEvidence
    } else if stage.kind == StageKind::Fanout {
        OutputContract::EvidenceFanout
    } else {
        stage_output_contract(stage)
    };
    ensure_declared_items_output(stage, body)?;
    ensure_output_usable_for_contract(contract, body)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OutputContract {
    Implementation,
    Verification,
    Evidence,
    EvidenceFanout,
    ReviewEvidence,
}

fn ensure_output_usable_for_contract(contract: OutputContract, body: &str) -> WorkflowResult<()> {
    let blocked = match contract {
        OutputContract::ReviewEvidence => {
            context_output::output_reports_invalid_review_evidence(body)
        }
        _ => context::output_reports_blocked(body),
    };
    if let Some(reason) = blocked {
        return Err(WorkflowError::StageFailed(reason));
    }
    let failed = match contract {
        OutputContract::Implementation => context::output_reports_failed_verification(body),
        OutputContract::Verification => context_output::output_reports_failed_execution(body),
        OutputContract::Evidence => context_output::output_reports_failed_execution(body),
        OutputContract::EvidenceFanout => {
            context_output::output_reports_failed_execution_without_test_counts(body)
        }
        OutputContract::ReviewEvidence => None,
    };
    if let Some(reason) = failed {
        return Err(WorkflowError::StageFailed(reason));
    }
    Ok(())
}

fn stage_output_contract(stage: &StageSpec) -> OutputContract {
    match stage.kind {
        StageKind::Implementation => OutputContract::Implementation,
        StageKind::Agent | StageKind::Tool if review_like_stage(stage) => {
            OutputContract::ReviewEvidence
        }
        StageKind::Agent | StageKind::Tool if verification_like_stage(stage) => {
            OutputContract::Verification
        }
        _ => OutputContract::Evidence,
    }
}

fn review_like_stage(stage: &StageSpec) -> bool {
    let text = stage_search_text(stage);
    ["review", "audit", "critic", "adversarial"]
        .iter()
        .any(|needle| text.contains(needle))
}

fn verification_like_stage(stage: &StageSpec) -> bool {
    if stage.verify_command.is_some() {
        return true;
    }
    let text = stage_search_text(stage);
    !review_like_stage(stage)
        && [
            "test",
            "tests",
            "verification",
            "verify",
            "clippy",
            "lint",
            "fmt",
            "build",
            "check",
        ]
        .iter()
        .any(|needle| text.contains(needle))
}

fn stage_search_text(stage: &StageSpec) -> String {
    format!("{} {}", stage.id, stage.task.as_deref().unwrap_or_default()).to_ascii_lowercase()
}

fn ensure_declared_items_output(stage: &StageSpec, body: &str) -> WorkflowResult<()> {
    if !crate::spec::stage_declares_items_producer(stage) {
        return Ok(());
    }
    for doc in candidate_documents(body) {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(doc)
            .or_else(|_| serde_yaml_ng::from_str::<serde_json::Value>(doc))
            && (value
                .get("items")
                .and_then(serde_json::Value::as_array)
                .is_some()
                || value
                    .get("completed_items")
                    .and_then(serde_json::Value::as_array)
                    .is_some())
        {
            if let Some(reason) =
                crate::completion_proof::invalid_completed_items_reason(&stage.id, doc)
            {
                return Err(WorkflowError::StageFailed(reason));
            }
            return Ok(());
        }
    }
    Err(WorkflowError::StageFailed(format!(
        "stage '{}' declares outputs: [items] but emitted no parseable items or completed_items structure",
        stage.id
    )))
}

fn candidate_documents(body: &str) -> Vec<&str> {
    let mut docs = vec![body.trim()];
    let mut rest = body;
    while let Some(start) = rest.find("```") {
        rest = &rest[start + 3..];
        if let Some(newline) = rest.find('\n') {
            rest = &rest[newline + 1..];
        }
        let Some(end) = rest.find("```") else {
            break;
        };
        docs.push(rest[..end].trim());
        rest = &rest[end + 3..];
    }
    docs
}
