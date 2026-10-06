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
#[serde(deny_unknown_fields)]
pub struct DecompositionAttemptStateV1 {
    pub logical_attempt: u32,
    pub interrupted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
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

/// A binary revision change, returned for durable recording by the resume host.
/// Script, catalog and template drift are upgrades too; call-level input keys
/// decide reuse, while this identity check protects the run's roots.
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
                "fixed decomposition resume identity mismatch for {field}: persisted {expected:?}, current {actual:?}; resume paused; restore the launch {field} and invoke resume from the original project root, or select the matching run"
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
