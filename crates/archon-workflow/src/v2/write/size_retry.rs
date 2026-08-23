//! Re-ask a write branch that lost its patch to the source-file line cap.
//!
//! The cap is checked when the patch manifest is validated, so a branch that
//! grows one file past it has the ENTIRE patch rejected — every other file in
//! it included — and the branch fails as a contract error with nothing landed.
//!
//! Observed live: 21 edits across five files lost because one
//! test file would have gone 495 -> 512 against a cap of 500, then lost again
//! at 504 on the next run. Both times the rejection named the remedy — relocate
//! into the module directory the branch already owns — and both times the
//! branch died before anyone could act on it.
//!
//! The rejection is a correctable instruction, not a verdict on the work, so it
//! is fed back and the branch re-dispatched. It keeps going while it is getting
//! closer; the moment an attempt repeats the previous overshoot it stops, so a
//! branch that cannot solve it cannot spin forever either.
//!
//! Reads only the rejection text and a line count, so it holds for any task,
//! language or PRD.

/// Ceiling on re-asks, far above what convergence needs. It exists so a
/// pathological branch cannot loop indefinitely; the progress test below is
/// what normally ends the loop.
pub(super) const MAX_SIZE_RETRIES: usize = 12;

/// The most provider dispatches one write branch can make.
///
/// DERIVED, not chosen. A branch re-asks for exactly two reasons and each has
/// its own budget: the patch was rejected by the size policy
/// ([`MAX_SIZE_RETRIES`]), or the transport dropped the call
/// ([`crate::v2::transport_retry::MAX_TRANSPORT_RETRIES`]). The total is the
/// first attempt plus both, so the ceiling cannot drift away from the budgets
/// it is made of — raising one raises this, visibly, in the same diff.
///
/// Writing it down matters because the ceiling was previously a product of
/// separate counters that no one had multiplied out. The loop bound also has to
/// be the SUM rather than either budget alone: bounding it by
/// `MAX_SIZE_RETRIES` made a transport blip consume an attempt reserved for
/// correcting a rejection, which is the opposite of what the loop's own comment
/// promised. Every attempt is still counted; none of them counts time, which is
/// what `call_time_budget_exhausted` is for.
pub(super) const MAX_BRANCH_DISPATCHES: usize =
    1 + MAX_SIZE_RETRIES + crate::v2::transport_retry::MAX_TRANSPORT_RETRIES;

/// Has this call spent the wall clock it was given, across every re-dispatch?
///
/// Needed because every budget in the branch loop counts ATTEMPTS and none of
/// them counts time. A branch could therefore re-dispatch far past any timeout
/// set for it: observed as one task still running at 6h20m under a two-hour
/// timeout, because the timeout bounded one attempt and nothing bounded the
/// call.
///
/// Separated from the loop so it can be exercised without a clock: a test that
/// sleeps to prove a timeout is the kind this repository has been deleting for
/// timing the machine rather than the behaviour.
///
/// `None` is unbounded, which is the honest reading for a dispatcher with no
/// configured timeout to derive a budget from.
pub(super) fn call_time_budget_exhausted(
    started: std::time::Instant,
    budget: Option<std::time::Duration>,
) -> bool {
    budget.is_some_and(|budget| started.elapsed() >= budget)
}

/// Why a call stopped when its total budget ran out.
///
/// Built here rather than at the loop so the wording sits beside the predicate
/// that decides it, and so the shared marker naming a recoverable outcome
/// cannot drift away from the check that produces it.
pub(super) fn call_time_budget_error(
    branch_id: &str,
    started: std::time::Instant,
    budget: Option<std::time::Duration>,
) -> crate::WorkflowError {
    crate::WorkflowError::port(format!(
        "write branch '{branch_id}' {} of {}s after {}s across re-dispatches; \
         the per-attempt timeout bounds one attempt, this bounds the call",
        super::errors::CALL_TIME_BUDGET_EXHAUSTED,
        budget.unwrap_or_default().as_secs(),
        started.elapsed().as_secs(),
    ))
}

/// Is this the wholesale size-policy rejection?
pub(super) fn is_line_cap_rejection(error: &str) -> bool {
    let lower = error.to_ascii_lowercase();
    lower.contains("would make source file") && lower.contains("entire patch is rejected")
}

/// The post-patch line count the rejection reports, used to tell whether a
/// re-ask got closer to the cap than the attempt before it.
pub(super) fn rejected_line_count(error: &str) -> Option<u32> {
    let start = error.find("would make source file")?;
    let tail = &error[start..];
    let lines_at = tail.find(" lines (currently")?;
    tail[..lines_at]
        .rsplit(|c: char| !c.is_ascii_digit())
        .find(|piece| !piece.is_empty())
        .and_then(|digits| digits.parse().ok())
}

/// Should the branch try again, given what the last two attempts overshot by?
///
/// Strictly decreasing only. Equal or worse means the branch is not converging
/// and another identical answer helps nobody.
pub(super) fn should_retry(previous: Option<u32>, current: Option<u32>) -> bool {
    match (previous, current) {
        (None, Some(_)) => true,
        (Some(before), Some(now)) => now < before,
        _ => false,
    }
}

/// What the branch is told on the re-ask, ahead of the rejection itself.
pub(super) fn retry_notice(error: &str) -> String {
    format!(
        "Your previous patch was REJECTED IN FULL and nothing was written. Every file you edited \
         was discarded, not just the one named below. Do not resubmit the same shape: move the new \
         code into the module directory named in the rejection, which you already own, and \
         re-export it from the file that is at the cap. Then redo the rest of the work.\n\n{error}"
    )
}

#[cfg(test)]
#[path = "size_retry_tests.rs"]
mod tests;
