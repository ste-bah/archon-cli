//! The decomposed-PRD lifecycle driver — the faithful port of the generated
//! scaffold JS (body_a/body_b + verification/ownership splices).
//!
//! [`LifecycleDriver`] runs one decomposed task universe to completion:
//! discovery, canonical inventory, dependency waves, implementation fan-out,
//! focused verification, triage, remediation, review, and the terminal gate. It
//! is the half of the lifecycle that *acts*; the half that *decides* is
//! [`crate::v2::lifecycle_policy`], and the driver consults it throughout.
//!
//! Every host interaction goes through [`crate::lifecycle_host_port`]. The
//! driver never names the host's type, which is what let it move here — it used
//! to hold `Arc<WorkflowScriptHost>`, and that type reaches `archon-pipeline`
//! and `archon-tools` through the live agent client.
//!
//! What stayed in the binary: the composition root. It builds the concrete
//! host, hands it to [`LifecycleDriver::new`], and turns the driver's outcome
//! into the script summary. It is spelled as an inherent
//! `impl WorkflowV2ScriptRunner`, so coherence pins it to the crate owning that
//! type — and it is also the only code that touches the concrete host at all,
//! so there was nothing to gain from moving it.

use std::sync::Arc;

use serde_json::Value;

pub(crate) use crate::error::WorkflowError;
pub(crate) use crate::generated_lifecycle_remediation as remediation;
pub(crate) use crate::generated_lifecycle_support as support;
pub(crate) use crate::generated_lifecycle_support::LifecycleContract;
pub(crate) use crate::lifecycle_host_port::{LifecycleHost, TERMINAL_HOST_CALL_MARKER};
pub(crate) use crate::task_universe::WorkflowV2TaskUniverse;
pub(crate) use crate::v2::lifecycle_policy;
pub(crate) use crate::v2::lifecycle_prompts as prompts;
pub(crate) use crate::v2::result::WorkflowV2Status;
pub(crate) use crate::v2::semantic_preservation;

/// Prefix on the internal error the terminal gate raises when a blocked report
/// must be rerouted instead of accepted. `LifecycleDriver::run` catches it and
/// restarts the lifecycle loop; it never reaches a user.
pub(crate) const TERMINAL_GATE_REROUTE_MARKER: &str = "workflow terminal gate reroute:";

pub struct LifecycleDriver {
    pub(crate) host: Arc<dyn LifecycleHost>,
    /// Read by the host to seed an orchestration ledger for the same run.
    /// Readable, not constructible: every other field is crate-private, so no
    /// struct literal is possible outside this crate.
    pub universe: WorkflowV2TaskUniverse,
    pub(crate) task_universe: Value,
    pub(crate) target_repository_root: Option<String>,
    pub(crate) project_artifact_root: Option<String>,
    pub(crate) governed_learning_context: Value,
    /// The run that owns this lifecycle's board partition (`wf-{uuid}`), and
    /// the board to drain at the end. Both or neither — a run wired to a board
    /// it cannot name has no partition to drain, and `with_board_drain` is the
    /// only way to set either. Absent means no board is configured for this
    /// run and the drain gate is a no-op; a board that IS configured and comes
    /// back with open items fails the run.
    pub(crate) board_drain: Option<(String, Arc<dyn crate::board_port::WorkflowBoardPort>)>,
    pub(crate) max_repair_iterations: usize,
    pub(crate) max_investigation_iterations: usize,
    pub(crate) max_dependency_waves: usize,
    /// How many ready tasks the write fan-out may dispatch at once, or `None`
    /// for the configured subagent concurrency. See
    /// `archon_core::config::decide_fanout_width` — this value can only ever be
    /// narrower than the cap, and the runtime clamps it again on the way out.
    pub(crate) write_wave_width: Option<usize>,
    pub(crate) runtime_state: std::sync::Mutex<lifecycle_policy::terminal_gate::TerminalGateState>,
}

/// The generated-workflow knobs the lifecycle reads.
///
/// A projection of `archon_core::config::GeneratedWorkflowConfig`, not that
/// type. The driver reads three of its five fields, and naming it here would
/// put `archon-core` in this crate's dependency graph to get them. The host
/// projects; the clamping stays here, next to the invariants it enforces.
#[derive(Clone, Copy, Debug)]
pub struct LifecycleLimits {
    pub max_repair_iterations: u8,
    pub max_investigation_iterations: u8,
    /// `None` means "the configured subagent concurrency". A value can only
    /// ever narrow a wave — the runtime clamps it again on the way out.
    pub implementation_wave_max_parallelism: Option<u8>,
}

/// Mutable evidence bundles — the JS lifecycle's top-level arrays.
#[derive(Default)]
pub struct LifecycleEvidence {
    pub(crate) implementation: Vec<Value>,
    pub(crate) verification: Vec<Value>,
    pub(crate) review: Vec<Value>,
    pub(crate) artifact: Vec<Value>,
    /// Read by the host's end-to-end coverage, which asserts on what a repair
    /// boundary rejected.
    pub repair_attempts: Vec<Value>,
    pub(crate) final_evidence_repair_attempts: Vec<Value>,
}

mod board_drain;
mod boundary_repair;
mod driver_a;
mod driver_b;
mod driver_c;
mod final_gates;
mod implementation;
mod orchestrated;
mod review;
mod review_assignment_invalid;
mod review_remediation;
mod review_verification;
mod verify;
mod verify_outcome_repair;
mod verify_remediation;
mod verify_triage;
mod waves;

/// Only the remediation tests reach this scoping helper from outside `verify`.
#[cfg(test)]
pub(crate) use verify::scope_repair_inventory_to_failed_outcomes;
// Everything except `is_transport_failure_text`, which is re-exported `pub`
// below. A glob at pub(crate) alongside a named pub re-export of one of its
// members leaves that name with two visibilities, and `-D warnings` rejects
// the ambiguity.
pub(crate) use verify_remediation::*;

pub use orchestrated::OrchestrationLedger;

#[cfg(test)]
mod board_drain_tests;
#[cfg(test)]
mod completion_gap_remediation_tests;
#[cfg(test)]
mod preservation_retry_tests;
#[cfg(test)]
mod review_assignment_invalid_tests;
#[cfg(test)]
mod review_remediation_tests;
#[cfg(test)]
mod review_round_bound_tests;
#[cfg(test)]
mod review_test_host;
#[cfg(test)]
mod review_verification_tests;
#[cfg(test)]
mod verify_actionable_tests;
#[cfg(test)]
mod verify_escalation_tests;
#[cfg(test)]
mod verify_remediation_tests;

pub(crate) fn normalize_null_report_collections(value: &mut Value) {
    const COLLECTION_FIELDS: &[&str] = &[
        "accepted_tasks",
        "actionable",
        "artifact_requirements",
        "artifacts",
        "blocked_tasks",
        "canonical_task_ids",
        "commands_run",
        "completed_ids",
        "dependency_ids",
        "evidence",
        "failed_tasks",
        "files_changed",
        "files_read",
        "focused_verification",
        "items",
        "missing_tasks",
        "noop_tasks",
        "outcomes",
        "remediation_actions",
        "repair_attempts",
        "residual_gaps",
        "retry_items",
        "review_blockers",
        "review_findings",
        "target_files",
        "task_coverage",
        "tests_run",
        "unresolved_issues",
    ];
    match value {
        Value::Object(object) => {
            for (key, child) in object {
                if COLLECTION_FIELDS.contains(&key.as_str()) {
                    // A null collection becomes empty; a null ELEMENT inside a
                    // present collection is dropped. The element carries no
                    // evidence, so removing it loses nothing and cannot
                    // fabricate a pass — but leaving it made a typed deserialize
                    // (Vec<WorkflowV2FileRecord>, Vec<WorkflowV2Evidence>) fail
                    // with "expected string or map" and crash terminal
                    // reporting. Nested collections still recurse via the
                    // retained elements below.
                    if child.is_null() {
                        *child = Value::Array(Vec::new());
                    } else if let Some(elements) = child.as_array_mut() {
                        elements.retain(|element| !element.is_null());
                        for element in elements {
                            normalize_null_report_collections(element);
                        }
                    } else {
                        normalize_null_report_collections(child);
                    }
                } else {
                    normalize_null_report_collections(child);
                }
            }
        }
        Value::Array(values) => {
            for child in values {
                normalize_null_report_collections(child);
            }
        }
        _ => {}
    }
}

pub(crate) fn terminal_marker_requires_report_fallback(status: Option<WorkflowV2Status>) -> bool {
    status == Some(WorkflowV2Status::Failed)
}

pub(crate) fn is_terminal_gate_reroute(error: &WorkflowError) -> bool {
    error.to_string().contains(TERMINAL_GATE_REROUTE_MARKER)
}
