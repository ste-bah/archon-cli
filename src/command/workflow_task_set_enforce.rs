//! Enforce-only convenience wrappers over authoritative freeze preparation and publication.

use super::*;

pub(crate) async fn freeze_acceptance(
    project_root: &Path,
    tasks_root: &Path,
    prd_path: &Path,
    client: Arc<dyn WorkflowLlmClient>,
) -> Result<FreezeAcceptanceResult> {
    let prepared = prepare_acceptance_freeze(
        project_root,
        tasks_root,
        prd_path,
        GateMode::Enforce,
        client,
    )
    .await?;
    let findings = prepared.findings.clone();
    let publication_identity = prepared.publication_identity();
    let mut disposition = crate::command::workflow_gate::run_sync_gate(
        project_root,
        GateMode::Enforce,
        GateId::FreezeAcceptance,
        || {
            Ok(
                crate::command::workflow_gate::GateEvaluation::new("", findings)
                    .with_publication_identity(publication_identity),
            )
        },
    )?;
    disposition.require_allowed()?;
    let permit = disposition
        .take_publication_permit()
        .ok_or_else(|| anyhow!("clean acceptance freeze received no publication permit"))?;
    publish_acceptance_freeze(prepared, permit)
}

pub(crate) fn freeze_skeleton(
    project_root: &Path,
    tasks_root: &Path,
    prd_path: &Path,
) -> Result<FreezeSkeletonResult> {
    let prepared = prepare_skeleton_freeze(project_root, tasks_root, prd_path, GateMode::Enforce)?;
    let findings = prepared.findings.clone();
    let publication_identity = prepared.publication_identity();
    let mut disposition = crate::command::workflow_gate::run_sync_gate(
        project_root,
        GateMode::Enforce,
        GateId::FreezeSkeleton,
        || {
            Ok(
                crate::command::workflow_gate::GateEvaluation::new("", findings)
                    .with_publication_identity(publication_identity),
            )
        },
    )?;
    disposition.require_allowed()?;
    let permit = disposition
        .take_publication_permit()
        .ok_or_else(|| anyhow!("clean skeleton freeze received no publication permit"))?;
    publish_skeleton_freeze(prepared, permit)
}
