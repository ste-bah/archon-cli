//! Skeleton-owned defects have the same route in file and candidate lint.
use crate::command::workflow_gate::GateFinding;
use std::path::Path;

pub(super) fn route(finding: &mut GateFinding, root: Option<&Path>) {
    if finding
        .deterministic_defect
        .as_ref()
        .is_some_and(skeleton_owned_defect)
    {
        if let Some(id) = &finding.deterministic_defect
            && id.subject.starts_with("TASK-")
        {
            finding.subject.clone_from(&id.subject);
        }
        finding.remediation_scope = archon_workflow::RemediationScope::Skeleton;
        finding.source_path =
            root.map(|root| root.join(archon_workflow::task_set_contract::TASK_SKELETON_FILE));
    }
}

pub(super) fn skeleton_owned_defect(id: &archon_workflow::defect::DeterministicDefect) -> bool {
    let field = id.location.split('/').next().unwrap_or_default();
    crate::command::workflow_host_command_binding::HOST_STAMPED_FIELDS.contains(&field)
}

#[cfg(test)]
mod tests {
    use super::*;
    use archon_workflow::{RemediationScope, defect::DeterministicDefect};

    #[test]
    fn stamped_locations_route_by_ownership_and_keep_the_task() {
        for field in crate::command::workflow_host_command_binding::HOST_STAMPED_FIELDS {
            for location in [
                field.to_string(),
                format!("{field}/0/typed_verifier_command"),
            ] {
                let mut finding = GateFinding::new(
                    crate::command::workflow_gate::GateId::WorkflowLintTaskFile,
                    "opaque diagnostic",
                    "task file",
                    None,
                    RemediationScope::Body,
                )
                .with_defect(DeterministicDefect::new(
                    "any_code",
                    "TASK-X-010",
                    location,
                ));
                route(&mut finding, Some(Path::new("tasks")));
                assert_eq!(finding.remediation_scope, RemediationScope::Skeleton);
                assert_eq!(finding.subject, "TASK-X-010");
                assert_eq!(
                    finding.source_path,
                    Some(Path::new("tasks/task-skeleton.json").into())
                );
            }
        }
        for location in ["focused_tests/0", "body", "deliverable_contracts_extra/0"] {
            let mut finding = GateFinding::new(
                crate::command::workflow_gate::GateId::WorkflowLintTaskFile,
                "deliverable_contracts: opaque diagnostic",
                "TASK-X-010",
                None,
                RemediationScope::Body,
            )
            .with_defect(DeterministicDefect::new(
                "invalid_verifier",
                "TASK-X-010",
                location,
            ));
            route(&mut finding, Some(Path::new("tasks")));
            assert_eq!(finding.remediation_scope, RemediationScope::Body);
            assert!(finding.source_path.is_none());
        }
    }
}
