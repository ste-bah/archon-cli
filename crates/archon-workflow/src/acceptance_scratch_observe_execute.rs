//! Evaluate declarative prerequisites before executing an authorized check.
use super::*;

/// One authorized check at a site: a nested verifier's declarative
/// prerequisites first in-process, then the pinned command itself.
pub async fn execute_check_at(
    site: &super::super::process::CommandSite<'_>,
    contract: &AcceptanceContract,
    reference: &FrozenCommandRef,
    command: &crate::acceptance_world::AuthorizedCommand,
    cancel: Arc<AtomicBool>,
) -> WorkflowResult<CheckResult> {
    if reference.kind == crate::acceptance_world::AcceptanceCommandKind::NestedVerifier {
        let entry = contract
            .acceptance
            .iter()
            .chain(&contract.supplementary)
            .find(|e| e.id == reference.acceptance_id)
            .expect("authorized entry");
        if let crate::task_set_contract::AcceptanceCheck::Floor { contract: floor } = &entry.check {
            let mut prerequisites = floor.clone();
            prerequisites.typed_verifier_command = None;
            let facts = super::super::control::Control::new(site.timeout_secs, cancel.clone())
                .run(|| {
                    crate::collect_declarative_floor_facts(
                        &crate::v2::deliverable_contract::ContractRoots::project_only(
                            site.project.to_string_lossy(),
                        ),
                        &prerequisites,
                    )
                })?;
            match crate::evaluate_floor_predicates(&prerequisites, &facts) {
                crate::FloorVerdict::Passed => {}
                crate::FloorVerdict::Failed { findings } => {
                    return Ok(CheckResult {
                        verdict_streams: Default::default(),
                        fault: None,
                        classification: None,
                        acceptance_id: reference.acceptance_id.clone(),
                        exit_code: Some(1),
                        quota_walk_count: 0,
                        stdout: vec![],
                        stderr: findings.join("; ").into_bytes(),
                        operational_error: None,
                        environment_note: None,
                    });
                }
            }
        }
    }
    super::super::process::run_at(site, &reference.acceptance_id, command, cancel).await
}
