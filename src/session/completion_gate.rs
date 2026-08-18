//! Turn-completion gate (#187).
//!
//! A skill can describe a verification step, but it cannot stop a model from
//! declaring victory — prose is advice, and a model under time pressure is free
//! to decide it has done enough. That is the weak point of every
//! methodology-as-markdown system: the most important rule is the one most
//! likely to be rationalised away, precisely because it stands between the
//! model and finishing.
//!
//! This is the part that is not advice. When a review has left gaps on the
//! board, the turn does not end: the findings go back to the model as a repair
//! prompt and it works them off. `GapsRemain` is the signal because it means
//! exactly one thing — something reviewed this work and found it wanting. No
//! other flow sets it, so the gate cannot fire on ordinary task tracking.
//!
//! **Unavailable is not failing.** A session with no memory service has no
//! board, and blocking every turn in that case would make the feature a
//! liability. The gate allows the turn and says why, once.

use archon_core::agent::TurnFinalizationVerdict;
use archon_core::config::CompletionGate;
use archon_memory::board::{BoardItem, BoardStatus};

/// Verdict for a turn that is trying to finish.
///
/// `session_id` is mapped to its run through the same helper the board tools
/// use, so a subagent and its parent share one partition and a workflow stage
/// resolves to the run that owns it.
pub(super) fn verdict(session_id: &str, mode: CompletionGate) -> TurnFinalizationVerdict {
    if mode == CompletionGate::Off {
        return TurnFinalizationVerdict::Allowed;
    }

    let run_id = archon_tools::board::run_id_for_session(session_id);
    let access = match archon_tools::board::BoardHandle::Global.resolve() {
        Ok(access) => access,
        Err(reason) => {
            // Debug, not warn: a print session or a session with memory
            // disabled hits this on every turn, and a per-turn warning for a
            // supported configuration is noise.
            tracing::debug!(%reason, "completion gate inactive: no board in this process");
            return TurnFinalizationVerdict::Allowed;
        }
    };

    let open_gaps = match access.list_board_items_by_run(run_id, &[BoardStatus::GapsRemain]) {
        Ok(items) => items,
        Err(error) => {
            tracing::warn!(%error, run_id, "completion gate could not read the board");
            return TurnFinalizationVerdict::Allowed;
        }
    };

    if open_gaps.is_empty() {
        return TurnFinalizationVerdict::Allowed;
    }

    match mode {
        CompletionGate::Off => TurnFinalizationVerdict::Allowed,
        CompletionGate::Warn => {
            tracing::warn!(
                run_id,
                count = open_gaps.len(),
                items = %summarise_titles(&open_gaps),
                "turn finished with unresolved review gaps"
            );
            TurnFinalizationVerdict::Allowed
        }
        CompletionGate::Block => TurnFinalizationVerdict::Blocked {
            repair_prompt: repair_prompt(&open_gaps),
        },
    }
}

/// Comma-separated titles, for the warn-mode log line.
fn summarise_titles(items: &[BoardItem]) -> String {
    items
        .iter()
        .map(|item| item.title.as_str())
        .collect::<Vec<_>>()
        .join("; ")
}

/// The text handed back to the model when the gate blocks.
///
/// Written as work to do rather than as a scolding, and it carries the item ids
/// because the model needs them to close the loop — a block with no way to
/// clear it is a deadlock, not a gate.
fn repair_prompt(items: &[BoardItem]) -> String {
    let mut out =
        String::from("This turn cannot finish yet: a review left gaps that are still open.\n\n");
    for item in items {
        out.push_str(&format!("- `{}` — {}\n", item.id, item.title));
        if !item.evidence.trim().is_empty() {
            out.push_str(&format!("  evidence: {}\n", item.evidence.trim()));
        }
        if !item.acceptance.trim().is_empty() {
            out.push_str(&format!("  done when: {}\n", item.acceptance.trim()));
        }
    }
    out.push_str(
        "\nFix each one, then close it with `BoardResolve`. If a finding is \
         wrong or not worth acting on, decline it with a reason rather than \
         leaving it open — an unexplained gap keeps the turn from ending.",
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn gap(id: &str, title: &str) -> BoardItem {
        BoardItem {
            id: id.into(),
            run_id: "run".into(),
            kind: archon_memory::board::BoardItemKind::Issue,
            status: BoardStatus::GapsRemain,
            title: title.into(),
            evidence: "cargo test failed in module x".into(),
            acceptance: "the suite passes".into(),
            raised_by: "reviewer".into(),
            claimed_by: None,
            round: 0,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            decline_reason: None,
        }
    }

    /// `Off` must not even reach the board — a disabled gate should cost
    /// nothing on a session that has no memory open.
    #[test]
    fn off_allows_without_consulting_the_board() {
        assert_eq!(
            verdict("any-session", CompletionGate::Off),
            TurnFinalizationVerdict::Allowed
        );
    }

    /// The failure mode that would make this feature a liability: a session
    /// with no memory service must not have every turn blocked.
    #[test]
    fn an_unavailable_board_allows_the_turn() {
        assert_eq!(
            verdict("session-with-no-board", CompletionGate::Block),
            TurnFinalizationVerdict::Allowed,
            "no board must mean no gate, not a wedged session"
        );
    }

    #[test]
    fn the_repair_prompt_names_every_open_item() {
        let prompt = repair_prompt(&[gap("itm-1", "tests fail"), gap("itm-2", "no coverage")]);

        assert!(prompt.contains("itm-1"), "{prompt}");
        assert!(prompt.contains("itm-2"), "{prompt}");
        assert!(prompt.contains("tests fail"), "{prompt}");
    }

    /// A block the model cannot clear is a deadlock. The prompt has to say how
    /// to close the item, and that declining is a legitimate way out.
    #[test]
    fn the_repair_prompt_says_how_to_clear_the_block() {
        let prompt = repair_prompt(&[gap("itm-1", "tests fail")]);

        assert!(prompt.contains("BoardResolve"), "{prompt}");
        assert!(prompt.contains("decline"), "{prompt}");
    }

    #[test]
    fn the_repair_prompt_carries_evidence_and_acceptance() {
        let prompt = repair_prompt(&[gap("itm-1", "tests fail")]);

        assert!(prompt.contains("cargo test failed"), "{prompt}");
        assert!(prompt.contains("the suite passes"), "{prompt}");
    }

    /// Blank evidence must not leave a dangling label in the prompt.
    #[test]
    fn empty_fields_are_omitted_rather_than_labelled() {
        let mut item = gap("itm-1", "tests fail");
        item.evidence = "   ".into();
        item.acceptance = String::new();

        let prompt = repair_prompt(&[item]);

        assert!(!prompt.contains("evidence:"), "{prompt}");
        assert!(!prompt.contains("done when:"), "{prompt}");
    }

    #[test]
    fn warn_mode_summarises_every_title() {
        let summary = summarise_titles(&[gap("a", "first"), gap("b", "second")]);

        assert!(summary.contains("first"), "{summary}");
        assert!(summary.contains("second"), "{summary}");
    }
}
