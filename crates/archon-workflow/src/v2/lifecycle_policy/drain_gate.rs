// The drain gate: a run may not report success while it still owns board work.
//
// "Leave no gaps" is an instruction today, which means it is a hope. This is
// the definition: for the run's own `run_id`, every board ISSUE must have ended
// as `resolved`, `promoted`, or `declined` with a recorded reason. Anything
// else — still open, still claimed, in review, gaps remaining, escalated and
// waiting on a human — is undrained work, and the run reports `needs_review`
// naming it rather than acceptance.
//
// WHY `escalated` IS NOT A DRAIN OUTCOME
//
// It is tempting to treat it as one: an escalated item has been dealt with by
// the agent that raised it. But escalation is a request for a decision that
// nobody has made yet, so a run that "succeeds" with escalated items has
// silently converted an open question into a shipped answer. The three
// permitted endings all record a decision — verified done, moved somewhere
// durable, or refused with a reason on the record.
//
// WHY NOTES ARE EXEMPT
//
// An issue is work that must happen and outlives the run. A note is context for
// whoever next touches the area and dies with the run. Requiring notes to be
// drained makes the gate fire on "looked at X, seemed fine" and the only way
// to keep a run green is to stop writing notes — which removes the handoff the
// board exists to carry.
//
// WHY IT IS A BARRIER AND NOT A FAN-OUT CHECK
//
// Board WRITES from a stage are replay-safe: they do not alter the stage's
// return value, and a resumed stage that does not re-run does not re-raise. A
// board READ is not. Concurrent branches inside a `Fanout` observe whatever
// their siblings have written by the time they look, which is scheduling
// order, so a cached prefix stops reproducing on resume and the failure is
// silent. The drain reads once, from sequential driver code after every wave,
// fan-out and review round has completed — a `Reduce`-equivalent point where
// the set of items is total and no sibling is still writing.

use crate::board_port::{DrainItem, DrainItemKind, DrainStatus};

/// One item the gate refused, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UndrainedItem {
    pub id: String,
    pub title: String,
    pub status: DrainStatus,
    pub reason: &'static str,
}

/// What the gate decided over a run's board.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DrainOutcome {
    pub run_id: String,
    pub inspected: usize,
    pub issues: usize,
    pub undrained: Vec<UndrainedItem>,
}

impl DrainOutcome {
    pub fn passed(&self) -> bool {
        self.undrained.is_empty()
    }

    /// The failure text, naming every item that blocked the run.
    ///
    /// Names them rather than counting them: a run that stops with "3 board
    /// items are not drained" costs whoever reads it another query to find out
    /// which, and the whole point of the board is that the handoff survives
    /// without one.
    pub fn failure_message(&self) -> String {
        if self.passed() {
            return format!(
                "board drain gate: all {} issue(s) for run {} are resolved, promoted, or declined",
                self.issues, self.run_id
            );
        }
        let named = self
            .undrained
            .iter()
            .map(|item| {
                format!(
                    "{} \"{}\" ({}: {})",
                    item.id,
                    item.title,
                    item.status.as_str(),
                    item.reason
                )
            })
            .collect::<Vec<_>>()
            .join("; ");
        format!(
            "board drain gate: run {} has {} unresolved board item(s): {named}",
            self.run_id,
            self.undrained.len()
        )
    }
}

/// Judge a run's board. Pure over the items — the read that produced them is
/// the driver's, and happens once, at the barrier.
pub fn evaluate(run_id: &str, items: &[DrainItem]) -> DrainOutcome {
    let mut undrained = Vec::new();
    let mut issues = 0usize;
    for item in items {
        if item.kind != DrainItemKind::Issue {
            continue;
        }
        issues += 1;
        if let Some(reason) = undrained_reason(item) {
            undrained.push(UndrainedItem {
                id: item.id.clone(),
                title: item.title.clone(),
                status: item.status,
                reason,
            });
        }
    }
    DrainOutcome {
        run_id: run_id.to_string(),
        inspected: items.len(),
        issues,
        undrained,
    }
}

fn undrained_reason(item: &DrainItem) -> Option<&'static str> {
    match item.status {
        DrainStatus::Resolved | DrainStatus::Promoted => None,
        DrainStatus::Declined => item
            .decline_reason
            .as_ref()
            .map(|reason| reason.trim())
            .filter(|reason| !reason.is_empty())
            .is_none()
            .then_some("declined without a recorded reason"),
        DrainStatus::Open => Some("still open"),
        DrainStatus::Claimed => Some("still claimed by an agent"),
        DrainStatus::InReview => Some("still in review"),
        DrainStatus::GapsRemain => Some("review left gaps remaining"),
        DrainStatus::Escalated => Some("escalated and awaiting a decision"),
    }
}

#[cfg(test)]
#[path = "drain_gate_tests.rs"]
mod tests;
