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
