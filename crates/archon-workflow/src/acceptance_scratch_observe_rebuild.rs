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
    CheckTeardown(WorkflowError),
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

/// A check's own result known before any site exists, with its typed
/// cause when it is the check's fault.
pub(super) type Own = (String, Option<faults::FaultCause>);

/// Each check's own result before any site exists: its authorization
/// error, or the refusal of a path it names (that check's own fault).
pub(super) fn pre_site<T>(
    commands: &[WorkflowResult<T>],
    refusals: &[Option<String>],
) -> Vec<Option<Own>> {
    let refused = |text: &String| (text.clone(), Some(faults::FaultCause::SiblingRootRefused));
    (commands.iter().zip(refusals))
        .map(|(command, refusal)| {
            (command.as_ref().err().map(|e| (e.to_string(), None)))
                .or_else(|| refusal.as_ref().map(refused))
        })
        .collect()
}

/// Each check's index among the site's checks: those with no `pre_site`
/// result, in order.
pub(super) fn positions(pre_site: &[Option<Own>]) -> Vec<Option<usize>> {
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
    pre_site: &[Option<Own>],
    error: &WorkflowError,
) -> Vec<CheckResult> {
    (refs.iter().zip(pre_site))
        .map(|(reference, own)| {
            let (result, cause) = own.clone().unwrap_or_else(|| {
                (
                    format!("site rebuild failed; check is host-unproven: {error}"),
                    match error {
                        WorkflowError::Io { source, .. } => source
                            .raw_os_error()
                            .map(|raw_os_error| faults::FaultCause::SiteSetupIo { raw_os_error }),
                        _ => None,
                    },
                )
            });
            super::faulted(&reference.acceptance_id, result, cause)
        })
        .collect()
}

/// Record a teardown failure against the check that alone used the verified
/// site, preserving every other result already recorded for the batch.
pub(super) fn attribute_teardown_failure(
    checks: &mut [super::process::CheckResult],
    owner: &str,
    error: &WorkflowError,
) {
    let check = checks
        .iter_mut()
        .rev()
        .find(|check| check.acceptance_id == owner)
        .expect("the check owning teardown was already recorded");
    let note = format!("the scratch site could not be removed after this check: {error}");
    check.operational_error = Some(match check.operational_error.take() {
        Some(existing) => format!("{existing}; {note}"),
        None => note,
    });
    if check.fault.is_none() {
        check.fault = Some(faults::FaultCause::SiteTeardownFailed);
    }
}
