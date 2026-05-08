//! Audited bundles for built-in coding/research pipeline runs.

pub mod export;
pub mod runtime;
pub mod store;
pub mod types;
pub mod verify;

pub use export::{TraceExportRow, export_jsonl};
pub use runtime::PipelineAuditRun;
pub use store::PipelineBundleStore;
pub use types::{
    AgentAttemptRecord, AgentAuditRecord, BundleManifest, BundleState, BundleStatus, PipelineEvent,
    PipelineEventLine, PromptAuditRecord,
};
pub use verify::{BundleVerificationReport, VerificationFinding, verify_bundle};
