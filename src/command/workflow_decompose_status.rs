//! Status detail extension for fixed decomposition runs.

use anyhow::{Context, Result};
use archon_workflow::{
    DecompositionPhase, FixedDecompositionStateV1, SubjectDisposition, WorkflowStore,
    WorkflowV2ResultStore,
};

pub(crate) fn render(store: &WorkflowStore, run_id: &str) -> Result<Option<String>> {
    let path = store
        .run_dir(run_id)
        .join(crate::command::workflow_decompose_state::FIXED_STATE_PATH);
    if !path.exists() {
        return Ok(None);
    }
    let state: FixedDecompositionStateV1 = serde_json::from_slice(
        &std::fs::read(&path)
            .with_context(|| format!("reading fixed decomposition status {}", path.display()))?,
    )
    .with_context(|| format!("parsing fixed decomposition status {}", path.display()))?;
    let checkpoint = WorkflowV2ResultStore::new(store.run_dir(run_id).join("v2"))
        .load_checkpoint()?
        .unwrap_or_default();
    let mut out = String::from("\nfixed decomposition:\n");
    out.push_str("run_kind: fixed_decomposition_v1\n");
    out.push_str(&format!(
        "template_version: {}\nstarting_binary_revision: {}\nscript_digest: {}\ncatalog_digest: {}\n",
        state.identity.template_version,
        state.identity.starting_binary_revision,
        state.identity.script_digest,
        state.identity.catalog_digest,
    ));
    out.push_str(&format!(
        "project_root: {}\nprd: {}\ntask_root: {}\nphase: {}\nlog_path: {}\nresume_eligible_calls: {}\n",
        state.identity.project_root_identity,
        state.identity.prd_identity,
        state.identity.task_root_identity,
        phase_label(state.phase),
        state.log_path,
        checkpoint.completed_call_ids.len(),
    ));
    if !state.attempts.is_empty() {
        out.push_str("attempts:\n");
        for (subject, attempt) in state.attempts {
            out.push_str(&format!(
                "- {subject} attempt={} interrupted={} last_error={}\n",
                attempt.logical_attempt,
                attempt.interrupted,
                attempt.last_error.as_deref().unwrap_or("none")
            ));
        }
    }
    if !state.dispositions.is_empty() {
        out.push_str("dispositions:\n");
        for (subject, disposition) in state.dispositions {
            out.push_str(&format!("- {subject}={}\n", disposition_label(disposition)));
        }
    }
    Ok(Some(out))
}

fn phase_label(phase: DecompositionPhase) -> &'static str {
    match phase {
        DecompositionPhase::Identity => "identity",
        DecompositionPhase::Acceptance => "acceptance",
        DecompositionPhase::Skeleton => "skeleton",
        DecompositionPhase::Bodies => "bodies",
        DecompositionPhase::SetGates => "set_gates",
        DecompositionPhase::Reconciliation => "reconciliation",
        DecompositionPhase::Completed => "completed",
    }
}

fn disposition_label(disposition: SubjectDisposition) -> &'static str {
    match disposition {
        SubjectDisposition::Pending => "pending",
        SubjectDisposition::Accepted => "accepted",
        SubjectDisposition::AcceptedWithShadowFindings => "accepted_with_shadow_findings",
        SubjectDisposition::Failed => "failed",
        SubjectDisposition::Blocked => "blocked",
        SubjectDisposition::Interrupted => "interrupted",
    }
}
