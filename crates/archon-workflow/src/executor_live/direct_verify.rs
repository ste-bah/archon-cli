use serde_json::json;

use crate::acceptance;
use crate::error::WorkflowResult;
use crate::run::WorkflowRun;
use crate::source_context;
use crate::spec::{ProviderTier, StageSpec};
use crate::store::WorkflowStore;

pub(super) fn should_run(stage: &StageSpec) -> bool {
    stage.provider_tier == Some(ProviderTier::Local)
        && stage
            .verify_command
            .as_deref()
            .is_some_and(|command| !command.trim().is_empty())
}

pub(super) fn root(
    store: &WorkflowStore,
    run: &WorkflowRun,
    stage: &StageSpec,
) -> WorkflowResult<std::path::PathBuf> {
    source_context::implementation_root_for_payload_targets(
        store,
        run,
        &stage.input,
        &stage.expected_target_files,
    )
    .or_else(|_| Ok(source_context::effective_root(store, run)))
}

pub(super) fn body(
    stage: &StageSpec,
    root: &std::path::Path,
    report: &acceptance::VerifyCommandReport,
    accepted: bool,
) -> String {
    serde_json::to_string_pretty(&json!({
        "schema": "archon.workflow.verify_command.v1",
        "stage": stage.id,
        "status": if accepted { "verified" } else { "failed" },
        "target_repository_root": root.display().to_string(),
        "command": &report.command,
        "exit_code": report.exit_code,
        "commands_run": [{
            "role": "verification",
            "command": &report.command,
            "exit_status": report.exit_code,
            "result": if accepted { "passed" } else { "failed" },
        }],
        "stdout_excerpt": truncate(&report.stdout),
        "stderr_excerpt": truncate(&report.stderr),
        "residual_gaps": [],
    }))
    .unwrap_or_else(|_| "{}".to_string())
}

fn truncate(text: &str) -> String {
    const LIMIT: usize = 12_000;
    if text.len() <= LIMIT {
        return text.to_string();
    }
    let mut cut = LIMIT;
    while !text.is_char_boundary(cut) {
        cut -= 1;
    }
    format!(
        "{}\n...[truncated {} bytes]",
        &text[..cut],
        text.len() - cut
    )
}
