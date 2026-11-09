//! The claim rule shared by the trace and the body landing gate.
use std::path::Path;

use crate::command::workflow_gate::{GateFinding, GateId};
use archon_workflow::RemediationScope;

/// Claim ids absent from non-metadata body sentences, with their trace reasons.
/// Constraints and obligations use exactly the same rule.
pub(super) fn untestable_claims(body: &str, implements: &[String]) -> Vec<(String, String)> {
    let lines = super::observe::body_lines(body);
    implements.iter().filter(|id| {
        !lines.iter().any(|line| !line.metadata && super::observe::mentions(&line.text, id))
    }).map(|id| (id.clone(), format!(
        "the body never names {id} outside its implements list, so no file, symbol or command it promises is tied to the claim"
    ))).collect()
}

pub(super) fn no_file_reason(task_id: &str) -> String {
    format!(
        "{task_id} declares no file it may change (Files Expected to Change and deliverable_contracts name nothing outside its forbidden files), so no declared file can serve its claims"
    )
}

pub(super) fn outside_reason(requirement: &str) -> String {
    format!(
        "nothing the body ties to {requirement} is inside the recorded repository, so the index cannot check it"
    )
}

pub(super) fn outside_declarations_reason(task_id: &str) -> String {
    format!(
        "{task_id} declares no changeable file inside the recorded repository; declarations outside the repository cannot ground a claim; declare a changeable file inside the recorded repository"
    )
}

/// Parsing belongs to the mechanical gate: never pile claim findings onto an
/// unreadable candidate. Use the same declarations/forbidden-file reader as
/// the trace; repository observables remain the whole set's responsibility.
#[cfg(test)]
pub(crate) fn body_claim_findings(root: &Path, path: &Path, body: &str) -> Vec<GateFinding> {
    let site = crate::command::requirement_trace::check_policy::command_path(None);
    body_claim_findings_at_path(root, path, body, &site)
}

pub(crate) fn body_claim_findings_at_path(
    root: &Path,
    path: &Path,
    body: &str,
    site_path: &std::ffi::OsStr,
) -> Vec<GateFinding> {
    let Ok(binding) = archon_knowledge::traceability::tasks::parse_task_binding_with_path(
        body,
        &path.to_string_lossy(),
        site_path,
    ) else {
        return Vec::new();
    };
    let root = crate::command::topology_lint::scope_declarations::repository_root(
        root,
        path.parent().unwrap_or_else(|| Path::new(".")),
    );
    let task = super::task_inputs(&root, binding, body.to_string());
    if task.unread.is_some() {
        return Vec::new();
    }
    let mut findings = Vec::new();
    for id in &task.binding.implements {
        let mut missing = untestable_claims(body, std::slice::from_ref(id));
        if missing.is_empty() && task.changeable.is_empty() && !task.outside.is_empty() {
            missing.push((
                id.clone(),
                outside_declarations_reason(&task.binding.task_id),
            ));
        }
        let text = if let Some((_, reason)) = missing.first() {
            format!(
                "task {}: its claim of '{id}' cannot be tested: {reason}; re-author the task body so a sentence naming {id} also names the file, symbol or verifier command that makes it observable",
                task.binding.task_id
            )
        } else if task.changeable.is_empty()
            && task.outside.is_empty()
            && task.forbidden_conflicts.is_empty()
        {
            format!(
                "task {}: its claim of '{id}' is refuted: {}; declare the files this task will change",
                task.binding.task_id,
                no_file_reason(&task.binding.task_id)
            )
        } else {
            continue;
        };
        findings.push(GateFinding::new(
            GateId::WorkflowLintTaskFile,
            text,
            task.binding.task_id.clone(),
            Some(path.to_path_buf()),
            RemediationScope::Body,
        ));
    }
    findings
}
