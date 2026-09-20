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
    let mut findings: Vec<_> = lint
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
    let mut report = lint.report;
    // Issue-55: the body's claims about repository paths, checked against the
    // repository recorded beside the task set. Deterministic and cheap, so it
    // runs before the critic and sends a false claim back while the author
    // still has attempts. A record that cannot be read is operational.
    let mut evaluation = None;
    if let Some(tasks_root) = path.parent() {
        let task_id = archon_workflow::task_universe::parsing::parse_task_file(&path, raw)
            .map(|task| task.canonical_task_id)
            .unwrap_or_else(|_| subject.clone());
        match super::repository_claims::inspect(cwd, tasks_root, &task_id, raw) {
            Ok(claims) => {
                report.push_str("\n## repository claims\n");
                if claims.is_empty() {
                    report.push_str("  no backticked repository path is said to exist or not exist against the recorded repository\n");
                }
                for text in claims {
                    report.push_str(&format!("  BLOCKING {text}\n"));
                    findings.push(crate::command::workflow_gate::GateFinding::new(
                        crate::command::workflow_gate::GateId::WorkflowLintTaskFile,
                        text,
                        task_id.clone(),
                        Some(path.clone()),
                        archon_workflow::RemediationScope::Body,
                    ));
                }
            }
            Err(error) => {
                evaluation = Some(
                    crate::command::workflow_gate::GateEvaluation::new(report.clone(), Vec::new())
                        .with_operational_error(format!("repository claim check failed: {error:#}")),
                );
            }
        }
    }
    Ok(match evaluation {
        Some(operational) => operational,
        None => crate::command::workflow_gate::GateEvaluation::new(report, findings),
    })
}
