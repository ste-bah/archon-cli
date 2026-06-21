use archon_workflow::WorkflowV2Result;

pub(super) fn attach_branch_evidence(
    aggregate: &mut WorkflowV2Result,
    branches: &[WorkflowV2Result],
) {
    for branch in branches {
        aggregate.evidence.extend(branch.evidence.clone());
        aggregate.artifacts.extend(branch.artifacts.clone());
        aggregate.commands_run.extend(branch.commands_run.clone());
        aggregate.files_read.extend(branch.files_read.clone());
        aggregate.files_changed.extend(branch.files_changed.clone());
        aggregate.task_coverage.extend(branch.task_coverage.clone());
        aggregate.residual_gaps.extend(branch.residual_gaps.clone());
    }
}

#[cfg(test)]
mod tests {
    use archon_workflow::{
        WorkflowV2CommandKind, WorkflowV2CommandRecord, WorkflowV2CommandStatus,
        WorkflowV2Evidence, WorkflowV2EvidenceKind, WorkflowV2FileRecord, WorkflowV2TaskCoverage,
        WorkflowV2TaskCoverageStatus,
    };

    use super::*;

    #[test]
    fn branch_evidence_is_lifted_to_aggregate_result() {
        let mut aggregate = WorkflowV2Result::accepted("fanout accepted");
        let mut branch = WorkflowV2Result::accepted("branch accepted");
        branch.evidence.push(WorkflowV2Evidence::new(
            WorkflowV2EvidenceKind::Implementation,
            "implemented concrete work",
        ));
        branch
            .files_read
            .push(WorkflowV2FileRecord::new("src/lib.rs"));
        branch
            .files_changed
            .push(WorkflowV2FileRecord::new("src/lib.rs"));
        branch.commands_run.push(WorkflowV2CommandRecord {
            kind: WorkflowV2CommandKind::Test,
            command: "cargo test focused".to_string(),
            status: WorkflowV2CommandStatus::Succeeded,
            exit_code: Some(0),
            output_summary: "passed".to_string(),
        });
        branch.task_coverage.push(WorkflowV2TaskCoverage {
            task_id: "T001".to_string(),
            status: WorkflowV2TaskCoverageStatus::Accepted,
            summary: "covered".to_string(),
            evidence: Vec::new(),
        });

        attach_branch_evidence(&mut aggregate, &[branch]);

        assert_eq!(aggregate.task_coverage[0].task_id, "T001");
        assert_eq!(aggregate.commands_run[0].command, "cargo test focused");
        assert_eq!(aggregate.files_changed[0].path, "src/lib.rs");
        assert_eq!(
            aggregate.evidence[0].kind,
            WorkflowV2EvidenceKind::Implementation
        );
    }
}
