//! Durable provider-neutral state for the fixed decomposition workflow.
//!
//! The binary owns process and publication authority. These records only name
//! the immutable run identity and the orchestration frontier needed by the
//! persisted v3 engine and its resume/status surfaces.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub const FIXED_DECOMPOSITION_STATE_SCHEMA_VERSION: u32 = 1;
pub const FIXED_DECOMPOSITION_TEMPLATE_VERSION: &str = "fixed-decomposition-v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowRunKind {
    AuthoredTaskWorkflow,
    LegacyDecomposed,
    FixedDecompositionV1,
    FixedOrSavedScript,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecompositionPhase {
    Identity,
    Acceptance,
    Skeleton,
    Bodies,
    SetGates,
    Reconciliation,
    Completed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubjectDisposition {
    Pending,
    Accepted,
    AcceptedWithShadowFindings,
    Failed,
    Blocked,
    Interrupted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecompositionAttemptStateV1 {
    pub logical_attempt: u32,
    pub interrupted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FixedRunIdentityV1 {
    pub template_version: String,
    pub starting_binary_revision: String,
    pub script_digest: String,
    pub catalog_digest: String,
    pub project_root_identity: String,
    pub prd_identity: String,
    pub task_root_identity: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FixedDecompositionStateV1 {
    pub schema_version: u32,
    pub run_kind: WorkflowRunKind,
    pub identity: FixedRunIdentityV1,
    pub phase: DecompositionPhase,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub attempts: BTreeMap<String, DecompositionAttemptStateV1>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub dispositions: BTreeMap<String, SubjectDisposition>,
    pub log_path: String,
}

/// The one identity component a resume tolerates changing (Issue-59).
///
/// `starting_binary_revision` is the launch record of which build started the
/// run, not a replay key: persisted per-call results are keyed by the template
/// version, the script digest and the host-command catalog digest, and those
/// are still compared exactly. A build that changes only guard, prompt or
/// config behaviour must be able to resume an in-flight decomposition,
/// otherwise no harness fix can ever be deployed mid-decomposition. The drift
/// is returned so the caller records it against the run; the persisted
/// identity stays the launch record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BinaryRevisionDrift {
    pub persisted: String,
    pub current: String,
}

pub fn verify_fixed_resume_identity(
    persisted: &FixedRunIdentityV1,
    current: &FixedRunIdentityV1,
) -> crate::WorkflowResult<Option<BinaryRevisionDrift>> {
    for (field, expected, actual) in [
        (
            "template_version",
            persisted.template_version.as_str(),
            current.template_version.as_str(),
        ),
        (
            "script_digest",
            persisted.script_digest.as_str(),
            current.script_digest.as_str(),
        ),
        (
            "catalog_digest",
            persisted.catalog_digest.as_str(),
            current.catalog_digest.as_str(),
        ),
        (
            "project_root_identity",
            persisted.project_root_identity.as_str(),
            current.project_root_identity.as_str(),
        ),
        (
            "prd_identity",
            persisted.prd_identity.as_str(),
            current.prd_identity.as_str(),
        ),
        (
            "task_root_identity",
            persisted.task_root_identity.as_str(),
            current.task_root_identity.as_str(),
        ),
    ] {
        if expected != actual {
            return Err(crate::WorkflowError::ArtifactInvalid(format!(
                "fixed decomposition resume identity mismatch for {field}: persisted {expected:?}, current {actual:?}; do not deploy or replace the Archon binary while a decomposition is active — restore the starting binary/source identity or start a new decomposition in a fresh task root"
            )));
        }
    }
    if persisted.starting_binary_revision != current.starting_binary_revision {
        return Ok(Some(BinaryRevisionDrift {
            persisted: persisted.starting_binary_revision.clone(),
            current: current.starting_binary_revision.clone(),
        }));
    }
    Ok(None)
}
