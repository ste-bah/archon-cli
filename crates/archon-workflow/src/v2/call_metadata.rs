//! Execution semantics are declared independently of durable call identity.
//! Prefix interpretation is confined to records without declared semantics.
use super::host_api::WorkflowV2HostCall;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CallKind {
    Other,
    NoopProof,
    Implementation,
    Remediation,
    ReviewRemediation,
    FocusedVerification,
    ReviewVerification,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallMetadata {
    pub kind: CallKind,
    #[serde(default)]
    pub task_scoped: bool,
    /// Whether this call participates in the canonical source graph.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_graph: Option<bool>,
    /// Optional order in the workflow's declared sequence, independent of names.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sequence: Option<u32>,
}

pub trait CallSemantics {
    fn metadata(&self) -> CallMetadata;
    fn verification(&self) -> bool {
        matches!(
            self.metadata().kind,
            CallKind::FocusedVerification | CallKind::ReviewVerification
        )
    }
    fn completion(&self) -> bool {
        self.metadata().kind != CallKind::Other
    }
}

impl CallSemantics for WorkflowV2HostCall {
    fn metadata(&self) -> CallMetadata {
        if let Some(metadata) = &self.options.metadata {
            return metadata.clone();
        }
        let kind = match self.options.item_kind.as_deref() {
            Some("implementation") => Some(match self.id.as_str().metadata().kind {
                CallKind::Remediation => CallKind::Remediation,
                CallKind::ReviewRemediation => CallKind::ReviewRemediation,
                _ => CallKind::Implementation,
            }),
            Some("focused_verification") => Some(CallKind::FocusedVerification),
            Some("review_verification") => Some(CallKind::ReviewVerification),
            Some("noop_proof") => Some(CallKind::NoopProof),
            Some(_) => Some(CallKind::Other),
            None => None,
        };
        if let Some(kind) = kind {
            return CallMetadata {
                kind,
                task_scoped: self.id.as_str().metadata().task_scoped,
                sequence: None,
                source_graph: self.id.as_str().metadata().source_graph,
            };
        }
        self.id.as_str().metadata()
    }
}

// Compatibility for already-recorded calls and the legacy public string API.
impl CallSemantics for str {
    fn metadata(&self) -> CallMetadata {
        let kind = if self.starts_with("noop-proof-verification-")
            || self.starts_with("noop-proof-reverification-")
        {
            CallKind::NoopProof
        } else if self.starts_with("review-verification-wave-") {
            CallKind::ReviewVerification
        } else if self.starts_with("verification-wave-") {
            CallKind::FocusedVerification
        } else if self.starts_with("review-remediation-wave-") {
            CallKind::ReviewRemediation
        } else if self.starts_with("remediation-wave-") || self.starts_with("remediate-task-") {
            CallKind::Remediation
        } else if self.starts_with("implementation-wave-") || self.starts_with("implement-task-") {
            CallKind::Implementation
        } else {
            CallKind::Other
        };
        CallMetadata {
            kind,
            task_scoped: self.starts_with("implement-task-")
                || self.starts_with("remediate-task-")
                || self.starts_with("verification-wave-verify-task-"),
            sequence: None,
            source_graph: if self.starts_with("implement-task-")
                || self.starts_with("remediate-task-")
            {
                Some(false)
            } else {
                None
            },
        }
    }
}
impl CallSemantics for String {
    fn metadata(&self) -> CallMetadata {
        self.as_str().metadata()
    }
}
