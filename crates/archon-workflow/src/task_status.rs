//! What a task file's `status:` declaration causes.
//!
//! `status:` was parsed into [`WorkflowV2TaskUniverseTask`] and carried through
//! every plan, and then nothing read it. A task the author had marked finished
//! was scheduled again; a task the author had marked blocked ran silently. This
//! module is the whole rule, in one place, so that "what does `status:` do" has
//! an answer that can be read rather than inferred from the scheduler.
//!
//! # The table
//!
//! | declared `status:` | effect on scheduling |
//! | --- | --- |
//! | `done`, `complete`, `completed`, `verified` | [`Complete`]. Never scheduled, on a fresh run or a resume, and counts as a satisfied dependency for everything downstream. The claim is the task file's, so it is surfaced in the run's evidence rather than applied silently. |
//! | `blocked` | [`Blocked`]. Scheduled only once every dependency it declares is complete — which is what the word means in these files, and the reason the corpus can be 15/17 `blocked` and still run. A `blocked` task that declares *no* dependency is refused: nothing in the task set can ever unblock it. |
//! | `ready`, `pending`, `todo`, `open`, `new`, `in_progress`, `in_review`, `needs_review`, `review`, `remediation` | [`Runnable`]. Ordinary work, scheduled when its dependencies are satisfied. `in_review` is deliberately here: review is not completion, so a run that has not itself completed the task must still do it. |
//! | absent, or an empty string | [`Runnable`]. A file that declares nothing has made no claim to honour. |
//! | anything else | **Refused.** The task set does not load, and the error names the value, the task and its file. |
//!
//! # Why an unrecognised value is refused rather than defaulted
//!
//! The two plausible defaults are both wrong in a way that hides the mistake.
//! Defaulting to runnable runs work the author may have marked `cancelled` or
//! `superseded`; defaulting to complete skips work nobody proved was done. A
//! typo (`don`, `blocekd`) reaches whichever default silently, and the author
//! sees a run that looks correct. Refusing costs one edit and is the only
//! reading that cannot be wrong about what the author meant.
//!
//! Absence is not the same as an unrecognised value, and is not refused: most
//! task files in the tree omit the field entirely, and "said nothing" is a
//! coherent state where "said something nobody understands" is not.

/// The scheduling consequence of a task file's declared `status:`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkflowV2DeclaredStatus {
    /// Ordinary work: scheduled once its dependencies are satisfied.
    Runnable,
    /// Already finished: never scheduled, and satisfies its dependents.
    Complete,
    /// Declared blocked by its author: runnable only behind its declared
    /// dependencies, and refused outright when it declares none.
    Blocked,
}

/// Classify one declared `status:` value.
///
/// `Err` carries the fragment of a message describing what was wrong; callers
/// prefix it with the task id and its file, because a status value on its own
/// is not something a reader can go and fix.
pub fn declared_status(raw: Option<&str>) -> Result<WorkflowV2DeclaredStatus, String> {
    let normalized = normalized_status(raw);
    if normalized.is_empty() {
        return Ok(WorkflowV2DeclaredStatus::Runnable);
    }
    match normalized.as_str() {
        "ready" | "pending" | "todo" | "open" | "new" | "in_progress" | "in_review"
        | "needs_review" | "review" | "remediation" => Ok(WorkflowV2DeclaredStatus::Runnable),
        "done" | "complete" | "completed" | "verified" => Ok(WorkflowV2DeclaredStatus::Complete),
        "blocked" => Ok(WorkflowV2DeclaredStatus::Blocked),
        other => Err(format!(
            "an unrecognised status: '{other}'. Scheduling refuses a status it cannot classify \
             rather than guessing: declare one of done/complete/completed/verified (already \
             finished), blocked (waits on its declared dependencies), or \
             ready/pending/todo/open/new/in_progress/in_review/needs_review/review/remediation \
             (ordinary work), or omit the field"
        )),
    }
}

/// True when the declaration says the work is already finished.
///
/// An unclassifiable value is *not* complete. It never reaches here in a loaded
/// task set — [`declared_status`] refuses it while the universe is built — and
/// treating it as finished would be the one reading that silently drops work.
pub fn declared_status_is_complete(raw: Option<&str>) -> bool {
    matches!(declared_status(raw), Ok(WorkflowV2DeclaredStatus::Complete))
}

/// True when the declaration says the author considered the task blocked.
pub fn declared_status_is_blocked(raw: Option<&str>) -> bool {
    matches!(declared_status(raw), Ok(WorkflowV2DeclaredStatus::Blocked))
}

fn normalized_status(raw: Option<&str>) -> String {
    raw.unwrap_or_default()
        .trim()
        .to_ascii_lowercase()
        .chars()
        .map(|ch| if ch == '-' || ch == ' ' { '_' } else { ch })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absent_and_empty_declarations_are_runnable() {
        assert_eq!(
            declared_status(None),
            Ok(WorkflowV2DeclaredStatus::Runnable)
        );
        assert_eq!(
            declared_status(Some("   ")),
            Ok(WorkflowV2DeclaredStatus::Runnable)
        );
    }

    #[test]
    fn the_corpus_values_classify() {
        assert_eq!(
            declared_status(Some("ready")),
            Ok(WorkflowV2DeclaredStatus::Runnable)
        );
        assert_eq!(
            declared_status(Some("pending")),
            Ok(WorkflowV2DeclaredStatus::Runnable)
        );
        assert_eq!(
            declared_status(Some("in_review")),
            Ok(WorkflowV2DeclaredStatus::Runnable),
            "review is not completion"
        );
        assert_eq!(
            declared_status(Some("blocked")),
            Ok(WorkflowV2DeclaredStatus::Blocked)
        );
    }

    #[test]
    fn separators_and_case_do_not_change_the_meaning() {
        assert_eq!(
            declared_status(Some("In-Review")),
            Ok(WorkflowV2DeclaredStatus::Runnable)
        );
        assert_eq!(
            declared_status(Some("  DONE ")),
            Ok(WorkflowV2DeclaredStatus::Complete)
        );
    }

    #[test]
    fn an_unrecognised_status_fails_closed_naming_the_value() {
        let error = declared_status(Some("blocekd")).expect_err("a typo is refused");
        assert!(error.contains("blocekd"), "{error}");
        assert!(
            error.contains("refuses a status it cannot classify"),
            "{error}"
        );
        assert!(
            !declared_status_is_complete(Some("blocekd")),
            "an unclassifiable status must never read as finished"
        );
        assert!(
            !declared_status_is_blocked(Some("blocekd")),
            "nor as blocked"
        );
    }
}
