//! Frozen verification keeps content defects separate from unavailable reads
//! and authenticated chain contradictions. The host owns transient backoff.
use super::FrozenStage;
use crate::command::workflow_gate::{GateEvaluation, GateFinding, GateId};
use anyhow::Result;
use archon_workflow::task_set_contract::{
    ACCEPTANCE_CONTRACT_FILE, ACCEPTANCE_LOCK_FILE, AcceptanceContract, TASK_SKELETON_FILE,
    TASK_SKELETON_LOCK_FILE,
};
use archon_workflow::{RemediationScope, defect::ValidationDefect};
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
#[error("reading frozen verification input {}: {source}", .path.display())]
struct VerificationRead {
    path: PathBuf,
    required: bool,
    #[source]
    source: std::io::Error,
}

pub(super) fn read(path: &Path, required: bool) -> Result<Vec<u8>> {
    std::fs::read(path).map_err(|source| {
        VerificationRead {
            path: path.into(),
            required,
            source,
        }
        .into()
    })
}

pub(super) fn is_directory(path: &Path) -> Result<bool> {
    match std::fs::metadata(path) {
        Ok(metadata) => Ok(metadata.is_dir()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(source) => Err(VerificationRead {
            path: path.into(),
            required: false,
            source,
        }
        .into()),
    }
}

#[derive(Debug, thiserror::Error)]
#[error("{message}")]
struct Authored {
    message: String,
    report: String,
    findings: Vec<GateFinding>,
}

pub(super) fn refuted(
    contract: &AcceptanceContract,
    root: &Path,
    message: String,
) -> anyhow::Error {
    let findings = crate::command::workflow_task_set::non_accepted_ids(contract)
        .into_iter()
        .map(|id| {
            let entry = contract.acceptance.iter().chain(&contract.supplementary)
                .find(|entry| entry.id == id).expect("non-accepted check exists");
            GateFinding::new(
                GateId::FreezeAcceptance,
                format!("check '{id}' was not accepted: {}; counterexample: {}; repair its check and deciding assertions", entry.judgment.reason, entry.judgment.counterexample),
                &id,
                Some(root.join(ACCEPTANCE_CONTRACT_FILE)),
                RemediationScope::CandidateArtifact,
            )
            .with_defect(archon_workflow::defect::DeterministicDefect::new(
                "check_not_accepted",
                id,
                "judgment",
            ))
        })
        .collect();
    Authored {
        message,
        report: serde_json::to_string(contract).expect("contract serializes"),
        findings,
    }
    .into()
}

fn defects(defects: &[ValidationDefect], root: &Path, acceptance: bool) -> Vec<GateFinding> {
    defects
        .iter()
        .map(|defect| {
            GateFinding::new(
                if acceptance {
                    GateId::FreezeAcceptance
                } else {
                    GateId::FreezeSkeleton
                },
                &defect.message,
                &defect.identity.subject,
                Some(root.join(if acceptance {
                    ACCEPTANCE_CONTRACT_FILE
                } else {
                    TASK_SKELETON_FILE
                })),
                if acceptance {
                    RemediationScope::CandidateArtifact
                } else {
                    RemediationScope::Skeleton
                },
            )
            .with_defect(defect.identity.clone())
        })
        .collect()
}

pub(super) fn evaluation(
    error: &anyhow::Error,
    project: &Path,
    root: &Path,
    stage: FrozenStage,
) -> GateEvaluation {
    if let Some(authored) = error.downcast_ref::<Authored>() {
        return GateEvaluation::new(&authored.report, authored.findings.clone());
    }
    // Both validators authenticate bytes against lock and pin before producing
    // shape defects. Empty defect lists represent chain/provenance failures.
    if let Some(error) =
        error.downcast_ref::<archon_workflow::task_set_contract::TaskSetContractError>()
        && !error.defects.is_empty()
    {
        return GateEvaluation::new(
            std::fs::read_to_string(root.join(ACCEPTANCE_CONTRACT_FILE)).unwrap_or_default(),
            defects(&error.defects, root, true),
        );
    }
    if let Some(error) = error.downcast_ref::<archon_workflow::task_skeleton::TaskSkeletonError>()
        && !error.defects.is_empty()
    {
        return GateEvaluation::new(error.to_string(), defects(&error.defects, root, false));
    }
    let transient = if let Some(read) = error.downcast_ref::<VerificationRead>() {
        !read.required || read.source.kind() != std::io::ErrorKind::NotFound
    } else {
        error.downcast_ref::<std::io::Error>().is_some()
            || error
                .downcast_ref::<archon_workflow::task_set_contract::TaskSetContractError>()
                .is_some_and(|error| error.io_kind.is_some())
            || error
                .downcast_ref::<archon_workflow::task_skeleton::TaskSkeletonError>()
                .is_some_and(|error| error.io_kind.is_some())
            || error
                .downcast_ref::<archon_workflow::task_set_publish_lock::PublishLockError>()
                .is_some()
            || (crate::command::workflow_task_set::UnsettledPublish::is(error)
                && !crate::command::workflow_task_set::UnsettledPublish::permanent(error))
    };
    let detail = if transient
        || crate::command::workflow_task_set::UnsettledPublish::permanent(error)
    {
        format!("{error:#}")
    } else {
        let (artifact, lock) = match stage {
            FrozenStage::Acceptance => (ACCEPTANCE_CONTRACT_FILE, ACCEPTANCE_LOCK_FILE),
            FrozenStage::Skeleton => (TASK_SKELETON_FILE, TASK_SKELETON_LOCK_FILE),
        };
        format!(
            "{error:#}; restore {} and {} to the bytes authenticated by {}; restore that pin from the same accepted publication before retrying verification",
            root.join(artifact).display(),
            root.join(lock).display(),
            crate::command::workflow_task_set::acceptance_pin_path(project, root).display()
        )
    };
    GateEvaluation::new(format!("{} failed", stage.command_id()), Vec::new())
        .with_operational_error(detail)
        .with_operational_error_kind(if transient { "transient" } else { "permanent" })
}

pub(super) fn is_authored(error: &anyhow::Error) -> bool {
    error.downcast_ref::<Authored>().is_some()
        || error
            .downcast_ref::<archon_workflow::task_set_contract::TaskSetContractError>()
            .is_some_and(|error| !error.defects.is_empty())
        || error
            .downcast_ref::<archon_workflow::task_skeleton::TaskSkeletonError>()
            .is_some_and(|error| !error.defects.is_empty())
}

pub(super) fn missing_skeleton(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<VerificationRead>()
        .is_some_and(|read| {
            read.required
                && read
                    .path
                    .file_name()
                    .is_some_and(|name| name == TASK_SKELETON_FILE)
                && read.source.kind() == std::io::ErrorKind::NotFound
        })
}

/// Duplicate authored identities cannot become duplicate launcher subjects:
/// retain one comparison entry while the owning author repairs the artifact.
pub(super) fn admission_skeleton(
    bytes: &[u8],
) -> Result<archon_workflow::task_skeleton::TaskSkeleton> {
    let mut skeleton: archon_workflow::task_skeleton::TaskSkeleton = serde_json::from_slice(bytes)?;
    let mut seen = std::collections::BTreeSet::new();
    skeleton
        .tasks
        .retain(|task| seen.insert(task.task_id.clone()));
    Ok(skeleton)
}
