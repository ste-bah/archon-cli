//! A site rebuilt for the checks still to run after one check's fault, and
//! the results of those checks when it cannot be rebuilt.
//!
//! A rebuild that fails is a host fault for the checks still to run, never
//! for the checks already judged (a keep-going build keeps the results of
//! the targets it built). A check whose own result was known before any site
//! existed (unauthorized, or refused for the path it names) keeps it. A
//! stall or a cancellation during the rebuild stays the batch's, so the
//! caller pauses or stops instead of recording host-unproven checks.

use super::*;

pub(super) enum Rebuild {
    Verified(identity::BuildIdentity),
    HostUnproven(WorkflowError),
}

/// `rebuilt`, as the checks still to run see it.
pub(super) fn classify(
    rebuilt: WorkflowResult<identity::BuildIdentity>,
    cancel: &AtomicBool,
) -> WorkflowResult<Rebuild> {
    match rebuilt {
        Ok(identity) => Ok(Rebuild::Verified(identity)),
        Err(error) => host_unproven(error, cancel).map(Rebuild::HostUnproven),
    }
}

/// Why a rebuilt site whose project reset or identity capture gave
/// `verified` (not its baseline) cannot be used by the checks still to run.
pub(super) fn unverified(
    verified: Option<WorkflowResult<identity::BuildIdentity>>,
    cancel: &AtomicBool,
) -> WorkflowResult<WorkflowError> {
    match verified {
        Some(Err(error)) => host_unproven(error, cancel),
        Some(Ok(_)) | None => Ok(invalid("native build identity changed before cache reuse")),
    }
}

/// `error`, unless it is a stall or a cancellation: those stay the batch's.
fn host_unproven(error: WorkflowError, cancel: &AtomicBool) -> WorkflowResult<WorkflowError> {
    let control = matches!(
        error,
        WorkflowError::ControlPaused(_) | WorkflowError::ControlCancelled(_)
    );
    if control || cancel.load(std::sync::atomic::Ordering::SeqCst) {
        return Err(error);
    }
    Ok(error)
}

/// Each check's own result before any site exists: its authorization
/// error, or the refusal of a path it names.
pub(super) fn pre_site<T>(
    commands: &[WorkflowResult<T>],
    refusals: &[Option<String>],
) -> Vec<Option<String>> {
    (commands.iter().zip(refusals))
        .map(|(command, refused)| {
            (command.as_ref().err().map(|e| e.to_string())).or_else(|| refused.clone())
        })
        .collect()
}

/// Each check's index among the site's checks: those with no `pre_site`
/// result, in order.
pub(super) fn positions(pre_site: &[Option<String>]) -> Vec<Option<usize>> {
    let mut next = 0;
    (pre_site.iter())
        .map(|own| {
            own.is_none().then(|| {
                next += 1;
                next - 1
            })
        })
        .collect()
}

/// The results of the checks `refs` still to run after a failed rebuild.
pub(super) fn unproven(
    refs: &[crate::acceptance_world::FrozenCommandRef],
    pre_site: &[Option<String>],
    error: &WorkflowError,
) -> Vec<CheckResult> {
    (refs.iter().zip(pre_site))
        .map(|(reference, own)| {
            let result = own
                .clone()
                .unwrap_or_else(|| format!("site rebuild failed; check is host-unproven: {error}"));
            super::operational(&reference.acceptance_id, result)
        })
        .collect()
}
