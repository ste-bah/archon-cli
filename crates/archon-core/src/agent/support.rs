// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum AgentLoopError {
    #[error("API error: {0}")]
    ApiError(String),

    #[error("tool dispatch error: {0}")]
    ToolError(String),
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
