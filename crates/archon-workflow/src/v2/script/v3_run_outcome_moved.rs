//! REM-14 (review): a blocked task review remediation finished joined the
//! accepted set after the mandatory reviews ran, so nothing reviewed the
//! work that finished it. It stands only when the host's records show a
//! LATER review map of each mandatory kind -- adversarial and coverage, the
//! late `*_moved` maps the prelude runs over exactly the tasks that moved
//! included -- that reviewed it, positioned after the verifier that closed
//! its last finding. Read from the maps' own call records, never from what
//! the script reports or which options it passed.

use super::keys::TaskKeys;
use super::*;

/// The mandatory review kinds a finished task must be reviewed under again.
const REVIEW_KINDS: [&str; 2] = ["adversarial_findings", "uncovered_requirements"];

fn is_kind(kind: &str, base: &str) -> bool {
    kind == base || kind.strip_prefix(base) == Some("_moved")
}

pub(super) fn check_moved_reviews(
    accounting: &serde_json::Value,
    calls: &[AuthoredCallFact],
    keys: &TaskKeys<'_>,
    v: &mut Verdict,
) {
    for (task, closed_at) in super::closure::blocked_tasks_finished(accounting, calls, keys) {
        for base in REVIEW_KINDS {
            let reviewed = calls.iter().skip(closed_at + 1).any(|call| {
                matches!(&call.role, AuthoredCallRole::Review { kind, stage }
                    if stage == "map" && is_kind(kind, base))
                    && call
                        .task(&task)
                        .is_some_and(|outcome| !outcome.not_reviewed)
            });
            if !reviewed {
                v.block(
                    format!(
                        "task {task} was finished by review remediation, but no later {base} review map reviewed it"
                    ),
                    false,
                );
            }
        }
    }
}
