//! Skeleton-owned defects have the same route in file and candidate lint.
use crate::command::workflow_gate::GateFinding;
use std::path::Path;

pub(super) fn route(finding: &mut GateFinding, root: Option<&Path>) {
    if finding
        .deterministic_defect
        .as_ref()
        .is_some_and(skeleton_owned_defect)
    {
        finding.remediation_scope = archon_workflow::RemediationScope::Skeleton;
        finding.source_path =
            root.map(|root| root.join(archon_workflow::task_set_contract::TASK_SKELETON_FILE));
    }
}

pub(super) fn skeleton_owned_defect(id: &archon_workflow::defect::DeterministicDefect) -> bool {
    matches!(
        id.code.as_str(),
        "uncited_section"
            | "unknown_section"
            | "unowned_constraint"
            | "duplicate_entry"
            | "untrimmed_entry"
    )
}
