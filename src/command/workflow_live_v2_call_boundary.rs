//! Batch G: what bounds one live agent call, whatever its kind.
//!
//! Built at the entry every workflow agent call funnels through
//! (`run_single_v2_agent_call_in_repository`), beside the run-store scope:
//!
//! - a READ-ONLY call's shell gets the OS write boundary the Issue-124
//!   machinery draws for isolated write branches, sealed tighter
//!   ([`read_only_boundary`]); a write-capable guard ignores it;
//! - EVERY call runs under the project-input tripwire
//!   ([`with_input_tripwire`]): a call that changed the project's acceptance
//!   inputs has them put back by the host and fails as an environment
//!   violation, its verdict never trusted.

use std::path::{Path, PathBuf};

use archon_tools::workflow_read_guard::ReadOnlyBoundaryScope;
use archon_workflow::v2::script::sanitize_v2_gap_id;
use archon_workflow::write_coordinator::input_tripwire::{EnvironmentViolation, watch_exempting};
use archon_workflow::{
    WorkflowError, WorkflowResult, WorkflowV2Evidence, WorkflowV2EvidenceKind,
    WorkflowV2ResidualGap, WorkflowV2Result, WorkflowV2ResultStore, WorkflowV2Status,
};

/// The roots a read-only call may never write: its working root, the
/// project root, the canonical checkout, the acceptance policy's roots (the
/// scratch parent holds the host's observation evidence) and the host's
/// agent transcript store. The run store is sealed by the run-store scope.
/// Inside them only the working root's toolchain directories are writable,
/// besides the temp, cache and target directories the host selects per
/// command.
pub(crate) fn read_only_boundary(
    v2_store: Option<&WorkflowV2ResultStore>,
    working_root: Option<&str>,
    project_root: Option<&str>,
    canonical_root: Option<&str>,
) -> Option<ReadOnlyBoundaryScope> {
    let working_root = working_root
        .map(str::trim)
        .filter(|root| !root.is_empty())?;
    let mut sealed: Vec<String> = [Some(working_root), project_root, canonical_root]
        .into_iter()
        .flatten()
        .map(str::to_string)
        .collect();
    if let Some(store) = v2_store {
        sealed.extend(
            recorded_policy_roots(store.run_root())
                .into_iter()
                .map(|root| root.display().to_string()),
        );
    }
    if let Some(home) = dirs::home_dir() {
        for store in ["sessions", "config.toml"] {
            sealed.push(home.join(".archon").join(store).display().to_string());
        }
    }
    sealed.sort();
    sealed.dedup();
    // The working, project and checkout roots' toolchain directories stay
    // writable, as a write branch's shared ones do: a verifier's `cargo test`
    // or `pytest` builds and caches there (never tracked, never an input).
    // A link there is never followed into what it names.
    let roots: Vec<&str> = [Some(working_root), project_root, canonical_root]
        .into_iter()
        .flatten()
        .collect();
    let toolchain: Vec<String> = (roots.iter())
        .flat_map(|root| {
            (archon_workflow::v2::write::SHARED_TOOLCHAIN_DIRS.iter())
                .map(move |name| Path::new(root).join(name))
        })
        .filter(|dir| {
            !dir.symlink_metadata()
                .is_ok_and(|m| m.file_type().is_symlink())
        })
        .map(|dir| dir.display().to_string())
        .collect();
    ReadOnlyBoundaryScope::new(working_root, &sealed, &toolchain)
}

/// The repository, project and scratch-parent roots of the run's recorded
/// acceptance policy (`v2/generated-metadata.json`), when it recorded one.
fn recorded_policy_roots(run_root: &Path) -> Vec<PathBuf> {
    let Ok(bytes) = std::fs::read(run_root.join("v2/generated-metadata.json")) else {
        return Vec::new();
    };
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        return Vec::new();
    };
    let Some(policy) = value.pointer("/observer_snapshot/native_execution/policy") else {
        return Vec::new();
    };
    ["repository", "project", "scratch_parent"]
        .iter()
        .filter_map(|key| policy.get(*key).and_then(serde_json::Value::as_str))
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .collect()
}

/// A write-capable call's own work, which the tripwire leaves to it: its
/// working tree and every path the write layer stamped writable for it (its
/// declared project deliveries, judged where they are). Nothing for a
/// read-only call.
pub(crate) fn own_work(
    execution: &archon_workflow::WorkflowV2CallExecution,
    working_root: Option<&str>,
) -> Vec<PathBuf> {
    if execution.call.write_mode.is_none() {
        return Vec::new();
    }
    let stamped = archon_workflow::agent_dispatch_port::write_boundary(&execution.input)
        .map(|(_, writable)| writable)
        .unwrap_or_default();
    (working_root.map(str::to_string).into_iter())
        .chain(stamped)
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .collect()
}

/// Run `call` under the project-input tripwire. A call that changed the
/// inputs (restored by the host) returns a FAILED result naming it, never
/// its own verdict, whether or not it succeeded -- except that a pause or
/// cancel still unwinds as itself (the call re-runs on resume).
pub(crate) async fn with_input_tripwire(
    v2_store: Option<&WorkflowV2ResultStore>,
    call_id: &str,
    own_work: &[PathBuf],
    call: impl std::future::Future<Output = WorkflowResult<WorkflowV2Result>>,
) -> WorkflowResult<WorkflowV2Result> {
    let run_root = v2_store.map(WorkflowV2ResultStore::run_root);
    let (outcome, violation) = watch_exempting(run_root, call_id, own_work, call).await;
    match (outcome, violation) {
        (
            Err(error @ (WorkflowError::ControlPaused(_) | WorkflowError::ControlCancelled(_))),
            _,
        ) => Err(error),
        (_, Some(violation)) => Ok(violation_result(call_id, &violation)),
        (outcome, None) => outcome,
    }
}

/// Fail every branch a host-run verifier judged while it changed the
/// project's inputs: none of those verdicts is trusted.
pub(crate) fn fail_outcomes(
    outcomes: &mut [archon_workflow::WorkflowV2BranchOutcome],
    violation: &EnvironmentViolation,
) {
    for outcome in outcomes {
        outcome.status = WorkflowV2Status::Failed;
        outcome.result = Some(violation_result(&outcome.item_id, violation));
    }
}

fn violation_result(call_id: &str, violation: &EnvironmentViolation) -> WorkflowV2Result {
    let message = violation.message();
    WorkflowV2Result {
        status: WorkflowV2Status::Failed,
        summary: message.clone(),
        evidence: vec![WorkflowV2Evidence::new(
            WorkflowV2EvidenceKind::Blocker,
            message.clone(),
        )],
        residual_gaps: vec![WorkflowV2ResidualGap {
            id: format!("environment-violation-{}", sanitize_v2_gap_id(call_id)),
            description: message,
            severity: Some("high".to_string()),
        }],
        data: serde_json::json!({ "environment_violation": violation }),
        ..WorkflowV2Result::default()
    }
}

#[cfg(test)]
#[path = "workflow_live_v2_call_boundary_tests.rs"]
mod tests;
