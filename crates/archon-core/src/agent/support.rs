// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum AgentLoopError {
    #[error("API error: {0}")]
    ApiError(String),

    #[error("tool dispatch error: {0}")]
    ToolError(String),

    #[error("auto-compaction failed: {0}")]
    Compaction(#[from] super::autocompact::CompactionError),

    #[error("turn finalization blocked: {0}")]
    FinalizationBlocked(String),
}

// ---------------------------------------------------------------------------
// Plan text parser
// ---------------------------------------------------------------------------

/// Parse a plan from the assistant's text output.
/// Simple line-by-line state machine: extracts title, steps, risks, questions.
pub(super) fn parse_plan_from_text(text: &str) -> archon_session::plan::PlanDocument {
    use archon_session::plan::{PlanDocument, PlanStep, PlanStepStatus};

    enum Section {
        None,
        Steps,
        Risks,
        Questions,
    }

    let mut title = String::from("Untitled Plan");
    let mut steps = Vec::new();
    let mut risks = Vec::new();
    let mut questions = Vec::new();
    let mut section = Section::None;
    let mut step_num: u32 = 0;

    for line in text.lines() {
        let trimmed = line.trim();

        // Detect title from headings
        if let Some(t) = trimmed
            .strip_prefix("## Plan:")
            .or_else(|| trimmed.strip_prefix("# Plan:"))
        {
            let t = t.trim();
            if !t.is_empty() {
                title = t.to_string();
            }
            continue;
        }

        // Detect section headings
        if trimmed.starts_with("### Steps") || trimmed.starts_with("## Steps") {
            section = Section::Steps;
            continue;
        }
        if trimmed.starts_with("### Risks") || trimmed.starts_with("## Risks") {
            section = Section::Risks;
            continue;
        }
        if trimmed.starts_with("### Questions")
            || trimmed.starts_with("## Questions")
            || trimmed.starts_with("### Open Questions")
            || trimmed.starts_with("## Open Questions")
        {
            section = Section::Questions;
            continue;
        }
        // Any other heading resets section
        if trimmed.starts_with("### ") || trimmed.starts_with("## ") {
            section = Section::None;
            continue;
        }

        if trimmed.is_empty() {
            continue;
        }

        match section {
            Section::Steps => {
                // Match numbered items like "1. Do something" or "- Do something"
                let desc = if let Some(rest) = trimmed.strip_prefix(|c: char| c.is_ascii_digit()) {
                    // Strip remaining digits and the dot
                    let rest = rest.trim_start_matches(|c: char| c.is_ascii_digit());
                    rest.strip_prefix('.').or(Some(rest)).map(|s| s.trim())
                } else {
                    trimmed.strip_prefix("- ").map(|s| s.trim())
                };
                if let Some(desc) = desc
                    && !desc.is_empty()
                {
                    step_num += 1;
                    steps.push(PlanStep {
                        number: step_num,
                        description: desc.to_string(),
                        affected_files: Vec::new(),
                        status: PlanStepStatus::Pending,
                    });
                }
            }
            Section::Risks => {
                if let Some(r) = trimmed.strip_prefix("- ") {
                    risks.push(r.trim().to_string());
                } else {
                    risks.push(trimmed.to_string());
                }
            }
            Section::Questions => {
                if let Some(q) = trimmed.strip_prefix("- ") {
                    questions.push(q.trim().to_string());
                } else {
                    questions.push(trimmed.to_string());
                }
            }
            Section::None => {}
        }
    }

    let id = format!("plan-{}", chrono::Utc::now().timestamp_millis());
    let mut doc = PlanDocument::new(&id, &title);
    doc.steps = steps;
    doc.risks = risks;
    doc.questions = questions;
    doc.status = "active".to_string();
    doc
}

pub(super) fn user_correction_excerpt(user_input: &str) -> String {
    // TODO(v0.1.52): use the shared secret-redaction regex once it is exposed
    // as a public helper outside archon-observability's tracing internals.
    user_input.chars().take(200).collect()
}

/// The correction text as it will be STORED, as opposed to
/// [`user_correction_excerpt`], which bounds what is reported.
///
/// `detect_and_record_correction` fires on a substring match anywhere in the
/// turn, so a message that merely contains "should have" counts -- including
/// one that pastes an entire document. Storing the raw turn put a
/// 25,793-character document into the graph as a `Correction`, which recall
/// then scanned and injection carried. The telemetry path was already bounded;
/// only the stored copy was not.
///
/// Truncated rather than dropped, unlike over-long extracted memories. A
/// correction is the user's stated intent, and losing it outright is worse than
/// keeping its opening -- which is where the correction itself nearly always
/// is, since the detector matched on a phrase near the start.
pub(super) fn stored_correction_content(user_input: &str) -> String {
    let limit =
        archon_memory::extraction::content_limit(archon_memory::types::MemoryType::Correction);
    if user_input.chars().count() <= limit {
        return user_input.to_string();
    }
    // The marker matters: a silently clipped correction reads as though the
    // user simply stopped mid-sentence.
    //
    // It is also counted against the limit rather than appended to it. Appending
    // produced a 2050-character "bounded" correction against a 2000 cap, so
    // `/memory prune` immediately reported every truncated correction as
    // oversized and offered to delete it -- a bound whose own output violated
    // the bound. Caught by running it; the unit tests only checked that
    // truncation happened.
    let marker = format!("… [truncated: correction exceeded {limit} characters]");
    let marker_len = marker.chars().count();
    let Some(room) = limit.checked_sub(marker_len) else {
        // Degenerate limit: keep it bounded and drop the explanation rather
        // than overflow.
        return user_input.chars().take(limit).collect();
    };
    let kept: String = user_input.chars().take(room).collect();
    format!("{kept}{marker}")
}

pub(super) fn message_text_content(message: &serde_json::Value) -> Option<String> {
    let content = message.get("content")?;
    if let Some(text) = content.as_str() {
        return Some(text.to_string());
    }
    let text = content
        .as_array()?
        .iter()
        .filter_map(|block| block.get("text").and_then(serde_json::Value::as_str))
        .collect::<Vec<_>>()
        .join(" ");
    (!text.is_empty()).then_some(text)
}

#[cfg(test)]
mod correction_content_tests {
    use super::*;

    /// An ordinary correction is stored exactly as the user wrote it.
    #[test]
    fn a_normal_correction_is_stored_verbatim() {
        let input = "no, you should have run the tests before pushing";
        assert_eq!(stored_correction_content(input), input);
    }

    /// A pasted document is bounded.
    ///
    /// The detector matches a phrase anywhere in the turn, so a message that
    /// merely contains "should have" is recorded -- and before this bound, a
    /// 25,793-character document went into the graph verbatim as a Correction.
    #[test]
    fn a_pasted_document_is_truncated_with_a_visible_marker() {
        let limit =
            archon_memory::extraction::content_limit(archon_memory::types::MemoryType::Correction);
        let pasted = format!("no, you should have read this:\n{}", "x".repeat(limit * 3));

        let stored = stored_correction_content(&pasted);

        assert!(
            stored.chars().count() < pasted.chars().count(),
            "an over-long correction must not be stored in full"
        );
        assert!(
            stored.starts_with("no, you should have read this:"),
            "the opening -- where the correction actually is -- must survive"
        );
        assert!(
            stored.contains("[truncated:"),
            "truncation must be visible, or the correction reads as though the \
             user stopped mid-sentence"
        );
        assert!(
            stored.chars().count() <= limit,
            "the bounded result must itself satisfy the bound, got {} against a \
             limit of {limit}; appending the marker on top of a full-length \
             excerpt made every truncated correction read as oversized to \
             `/memory prune`, which then offered to delete it",
            stored.chars().count()
        );
    }

    /// Exactly at the limit is not truncated: an off-by-one here would clip
    /// every correction that happens to land on the boundary.
    #[test]
    fn content_exactly_at_the_limit_is_untouched() {
        let limit =
            archon_memory::extraction::content_limit(archon_memory::types::MemoryType::Correction);
        let exact = "y".repeat(limit);
        assert_eq!(stored_correction_content(&exact), exact);
    }
}
