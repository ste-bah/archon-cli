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
use archon_workflow::write_coordinator::input_tripwire::{EnvironmentViolation, watch_exempting};
use archon_workflow::write_coordinator::sealed_roots::sealed_host_roots;
use archon_workflow::{
    WorkflowError, WorkflowResult, WorkflowV2Evidence, WorkflowV2EvidenceKind, WorkflowV2Result,
    WorkflowV2ResultStore,
};

/// The roots a read-only call may never write: its working root and every
/// host root ([`sealed_host_roots`], the ONE list a write branch's boundary
/// seals too: project root, canonical checkout, run store, the acceptance
/// policy's roots whose scratch parent holds the host's observation
/// evidence, and the host's transcript store and configuration). Inside them
/// only the working root's toolchain directories are writable, besides the
/// temp, cache and target directories the host selects per command.
///
/// Batch G2: always drawn. A call with no repository root works in this
/// process's directory, which is then its working root; before, it got no
/// boundary at all and its shell ran unbounded.
pub(crate) fn read_only_boundary(
    v2_store: Option<&WorkflowV2ResultStore>,
    working_root: Option<&str>,
    project_root: Option<&str>,
    canonical_root: Option<&str>,
) -> Option<ReadOnlyBoundaryScope> {
    let working_root = match working_root.map(str::trim).filter(|root| !root.is_empty()) {
        Some(root) => root.to_string(),
        // Where the shell will run; failing that, a host root it is sealed
        // in anyway: never no boundary.
        None => std::env::current_dir()
            .ok()
            .map(|dir| dir.display().to_string())
            .or_else(|| project_root.or(canonical_root).map(str::to_string))
            .unwrap_or_else(|| "/".to_string()),
    };
    let working_root = working_root.as_str();
    let mut sealed: Vec<String> = sealed_host_roots(
        v2_store.map(WorkflowV2ResultStore::run_root),
        project_root.map(Path::new),
        canonical_root.map(Path::new),
    )
    .into_iter()
    .map(|root| root.display().to_string())
    .collect();
    sealed.push(working_root.to_string());
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

/// A write-capable call's own work, which the tripwire leaves to it: its
/// working tree and the paths the write layer stamped writable for it (its
/// seeded project data and declared artifact copies in its worktree or its
/// staging directory). Batch G2: never a path of the live project root
/// outside that working tree -- the host lands those, audited. Nothing for a
/// read-only call.
pub(crate) fn own_work(
    execution: &archon_workflow::WorkflowV2CallExecution,
    working_root: Option<&str>,
    v2_store: Option<&WorkflowV2ResultStore>,
) -> Vec<PathBuf> {
    if execution.call.write_mode.is_none() {
        return Vec::new();
    }
    let working_root = working_root
        .map(PathBuf::from)
        .filter(|root| root.is_absolute());
    let project_root = v2_store
        .and_then(|store| {
            archon_workflow::project_artifact_context_from_v2_root(store.root()).project_root
        })
        .map(PathBuf::from)
        .filter(|root| root.is_absolute());
    let stamped = archon_workflow::agent_dispatch_port::write_boundary(&execution.input)
        .map(|(_, writable)| writable)
        .unwrap_or_default();
    let live = |path: &Path| {
        project_root.as_deref().is_some_and(|project| {
            spelled_under(path, project)
                && !working_root
                    .as_deref()
                    .is_some_and(|root| spelled_under(path, root))
        })
    };
    (working_root.clone().into_iter())
        .chain(stamped.into_iter().map(PathBuf::from))
        .filter(|path| path.is_absolute())
        .filter(|path| working_root.as_deref() == Some(path.as_path()) || !live(path))
        .collect()
}

/// Whether `path` lies under `root`, as given or with each resolved.
fn spelled_under(path: &Path, root: &Path) -> bool {
    let real = |p: &Path| {
        p.canonicalize()
            .map(archon_shell::paths::plain)
            .ok()
            .or_else(|| {
                Some(
                    p.parent()?
                        .canonicalize()
                        .map(archon_shell::paths::plain)
                        .ok()?
                        .join(p.file_name()?),
                )
            })
    };
    path.starts_with(root)
        || match (real(path), real(root)) {
            (Some(path), Some(root)) => path.starts_with(root),
            _ => false,
        }
}

/// Run the agent call `call` makes under the project-input tripwire (Batch
/// G; Batch G2 for how a violation is resolved). A pause or cancel always
/// unwinds as itself (the call re-runs on resume). When the inputs changed
/// during the call the host has already put them back, and:
///
/// - a WRITE call not attributed the change (another call or command
///   overlapped its window) keeps its result: its work is in its own
///   worktree, and every gate after it judges that, not the project;
/// - a write call that IS attributed it is never re-run in its worktree
///   (the untrusted attempt's edits would land with the re-run's): it is
///   the host's operational error ([`operational_error`]);
/// - any other call is re-run once, as the host re-asks a dropped
///   transport: its verdict was not trusted, and neither the task nor its
///   branch is charged for it; a re-run that trips again is the host's
///   operational error, never a verdict on the work.
pub(crate) async fn with_input_tripwire<F, Fut>(
    v2_store: Option<&WorkflowV2ResultStore>,
    call_id: &str,
    own_work: &[PathBuf],
    mut call: F,
) -> WorkflowResult<WorkflowV2Result>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = WorkflowResult<WorkflowV2Result>>,
{
    let run_root = v2_store.map(WorkflowV2ResultStore::run_root);
    let write_call = !own_work.is_empty();
    let mut first: Option<EnvironmentViolation> = None;
    loop {
        let (outcome, violation) = watch_exempting(run_root, call_id, own_work, call()).await;
        let violation = match (outcome, violation) {
            (
                Err(error @ (WorkflowError::ControlPaused(_) | WorkflowError::ControlCancelled(_))),
                _,
            ) => return Err(error),
            (outcome, None) => return outcome.map(|result| rerun_noted(result, first.as_ref())),
            (outcome, Some(violation)) if write_call && !violation.attributed => {
                return outcome.map(|result| overlap_noted(result, &violation));
            }
            (_, Some(violation)) => violation,
        };
        // A write call is never re-run in its worktree: what the untrusted
        // attempt left there would land with the re-run's work. It fails as
        // the host's operational error, retried like a dropped transport.
        if first.is_some() || write_call {
            return Err(operational_error(call_id, &violation));
        }
        eprintln!(
            "{call_id}: re-running once after the host restored the project's inputs: {}",
            violation.message()
        );
        first = Some(violation);
    }
}

/// The host's own marker for a failure that is not a verdict on the work:
/// classified with transport and timeout failures (`BranchFailureKind::
/// Execution`), so it is retried, never routed to a task as a finding.
pub(crate) fn operational_error(call_id: &str, violation: &EnvironmentViolation) -> WorkflowError {
    WorkflowError::HostOperational(format!(
        "{call_id}: the project's inputs changed during this call and the host put them back; its result is not trusted and it is not re-run in place. {}",
        violation.message()
    ))
}

fn rerun_noted(
    mut result: WorkflowV2Result,
    first: Option<&EnvironmentViolation>,
) -> WorkflowV2Result {
    if let Some(first) = first {
        result.evidence.push(WorkflowV2Evidence::new(
            WorkflowV2EvidenceKind::Inspection,
            format!(
                "the host re-ran this call once: its first attempt was not trusted because the project's inputs changed during it (restored by the host; {} attributed to it): {}",
                if first.attributed { "" } else { "not" },
                first.message()
            ),
        ));
    }
    result
}

fn overlap_noted(
    mut result: WorkflowV2Result,
    violation: &EnvironmentViolation,
) -> WorkflowV2Result {
    result.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Inspection,
        format!(
            "the project's inputs changed during this write call while other calls ran; the host put them back, and the change is not attributed to this call: {}",
            violation.message()
        ),
    ));
    if !result.data.is_object() {
        result.data = serde_json::json!({});
    }
    if let Some(data) = result.data.as_object_mut() {
        data.insert(
            "environment_violation_unattributed".into(),
            serde_json::json!(violation),
        );
    }
    result
}

#[cfg(test)]
#[path = "workflow_live_v2_call_boundary_tests.rs"]
mod tests;
