//! Turning an over-long correction into a memory worth keeping.
//!
//! `detect_and_record_correction` matches a phrase anywhere in the user's turn,
//! so a message that merely contains "should have" is recorded -- including one
//! that pastes a whole document. Bounding that (see `stored_correction_content`)
//! stops the graph filling with transcripts, but the stored result is still the
//! first two thousand characters of a paste: bounded, and useless.
//!
//! So when the raw turn had to be truncated, a background call restates the
//! correction and replaces the stored content with the restatement.
//!
//! Two properties make an LLM acceptable in a write path here:
//!
//! 1. The correction is recorded BEFORE the call, with the bounded raw text.
//!    A failed, slow, or empty response leaves that in place. The call can only
//!    improve the record, never lose it.
//! 2. The prompt restates rather than infers. A summariser that invented a
//!    correction the user never made would be worse than storing a document,
//!    because the result is indistinguishable from something they said.

use super::*;

/// Cap on the restatement itself.
///
/// A correction that needed summarising is exactly the one most likely to come
/// back long, and an unbounded "summary" would reintroduce the problem it was
/// called to solve.
const MAX_SUMMARY_TOKENS: u32 = 256;

/// Ask for a restatement of the correction in `user_input`.
///
/// The constraints are load-bearing, not politeness. Each maps to a way this
/// can produce something worse than what it replaces.
pub(super) fn build_correction_summary_prompt(user_input: &str) -> String {
    format!(
        r#"The user has just corrected an AI assistant. Restate their correction as an instruction the assistant should follow in future.

Rules:
- Restate ONLY what the user actually said. Never infer, extend, or generalise.
- Address the assistant, not the user. Write "Run the tests before pushing",
  never "The user wants tests run" and never "Avoid running the tests".
- At most two sentences.
- If the message contains a pasted document, log, diff, or code block, describe
  what the user wants done differently. Never reproduce the pasted content.
- If the message contains no actual correction of the assistant's behaviour,
  reply with exactly: NONE

Message:
{user_input}
"#
    )
}

/// Whether `summary` is usable as a correction's stored content.
///
/// A model asked for `NONE` will sometimes wrap it in a sentence, so the check
/// is a prefix match on the trimmed text rather than equality.
fn usable_summary(summary: &str) -> Option<String> {
    let trimmed = summary.trim();
    if trimmed.is_empty() || trimmed.to_uppercase().starts_with("NONE") {
        return None;
    }
    Some(trimmed.to_string())
}

impl Agent {
    /// Replace a truncated correction's content with a restatement of it.
    ///
    /// Fires only when the raw turn was actually truncated. A correction that
    /// fitted is already the user's own words, which is a better record than any
    /// paraphrase and costs nothing to keep.
    pub(super) fn spawn_correction_summary(
        &self,
        correction_id: String,
        user_input: String,
        graph: Arc<dyn MemoryTrait>,
    ) {
        let client = Arc::clone(&self.client);
        let model = self.config.model.clone();
        let attribution = self.config.runtime_attribution_extra(
            "correction_summary",
            "correction_summary",
            Some(self.turn_number),
            None,
            None,
        );

        tokio::spawn(async move {
            let request = LlmRequest {
                model,
                max_tokens: MAX_SUMMARY_TOKENS,
                system: vec![serde_json::json!({
                    "type": "text",
                    "text": "You restate user corrections as instructions for an AI assistant. \
                             Reply with the instruction only, or exactly NONE."
                })],
                messages: vec![serde_json::json!({
                    "role": "user",
                    "content": build_correction_summary_prompt(&user_input),
                })],
                tools: Vec::new(),
                thinking: None,
                speed: None,
                effort: None,
                extra: attribution,
                request_origin: Some("correction_summary".into()),
                reasoning_encrypted: None,
            };

            let mut response_text = String::new();
            match client.stream(request).await {
                Ok(mut rx) => {
                    while let Some(event) = rx.recv().await {
                        if let StreamEvent::TextDelta { text, .. } = event {
                            response_text.push_str(&text);
                        }
                    }
                }
                Err(error) => {
                    // The bounded raw text stays. Warn rather than escalate:
                    // the record is intact, only less readable.
                    tracing::warn!(%error, "correction summary call failed; keeping truncated text");
                    return;
                }
            }

            let Some(summary) = usable_summary(&response_text) else {
                tracing::debug!(
                    correction_id,
                    "correction summary returned nothing usable; keeping truncated text"
                );
                return;
            };

            // Bound the restatement too. Asked for two sentences, a model can
            // still return an essay, and this is the same write path that let a
            // document in.
            let summary = crate::agent::support::stored_correction_content(&summary);
            match graph.update_memory(&correction_id, Some(&summary), None) {
                Ok(()) => {
                    tracing::info!(correction_id, "correction replaced with a restatement")
                }
                Err(error) => {
                    tracing::warn!(%error, correction_id, "correction summary update failed")
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_forbids_the_inversion_and_the_paste() {
        let prompt = build_correction_summary_prompt("no, you should have run the tests");

        assert!(
            prompt.contains("never \"Avoid running the tests\""),
            "the prompt must name the inversion it is guarding against"
        );
        assert!(
            prompt.contains("Never reproduce the pasted content"),
            "the prompt must forbid echoing the document back"
        );
        assert!(
            prompt.contains("Restate ONLY what the user actually said"),
            "the prompt must forbid inference; an invented correction is worse \
             than a truncated one"
        );
        assert!(prompt.contains("no, you should have run the tests"));
    }

    #[test]
    fn none_and_blank_responses_are_rejected() {
        assert!(usable_summary("NONE").is_none());
        assert!(usable_summary("  none  ").is_none());
        // Models wrap the sentinel rather than emitting it bare.
        assert!(usable_summary("NONE - there is no correction here").is_none());
        assert!(usable_summary("   ").is_none());
        assert!(usable_summary("").is_none());
    }

    #[test]
    fn a_real_restatement_is_accepted_and_trimmed() {
        assert_eq!(
            usable_summary("  Run the tests before pushing.\n").as_deref(),
            Some("Run the tests before pushing.")
        );
    }
}
