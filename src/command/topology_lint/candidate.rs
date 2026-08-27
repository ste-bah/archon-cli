//! Candidate-byte task-file evaluation for fixed-decomposition body landing.

use std::path::Path;

use anyhow::{Context, Result};

pub(crate) fn evaluate_task_file_candidate(
    cwd: &Path,
    path: &Path,
    candidate: &[u8],
    mode: archon_core::config::GateMode,
) -> Result<crate::command::workflow_gate::GateEvaluation> {
    let path = super::absolute(cwd, path);
    super::preflight::task_file_freeze(cwd, &path)?;
    let raw = std::str::from_utf8(candidate)
        .context("candidate TASK body is not UTF-8; return one complete UTF-8 TASK file")?;
    let lint = super::task_file::inspect_raw(cwd, &path, raw, mode);
    let subject = format!("task file {}", path.display());
    let findings = lint
        .blockers
        .into_iter()
        .map(|text| {
            let remediation_scope = if lint.inherited_blockers.contains(&text) {
                archon_workflow::RemediationScope::InheritedPredecessor
            } else {
                archon_workflow::RemediationScope::Body
            };
            crate::command::workflow_gate::GateFinding::new(
                crate::command::workflow_gate::GateId::WorkflowLintTaskFile,
                text.clone(),
                crate::command::workflow_gate::finding_subject(&text, &subject),
                Some(path.clone()),
                remediation_scope,
            )
        })
        .collect();
    Ok(crate::command::workflow_gate::GateEvaluation::new(
        lint.report,
        findings,
    ))
}
