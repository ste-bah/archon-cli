//! Periodic semantic memory extraction.
//!
//! Split out of the correction path to keep both files under the size gate,
//! and because they are different jobs: the parent decides what a turn
//! CORRECTED, this decides what a stretch of conversation is worth
//! remembering. They share only the writer that records corrections the
//! keyword pass missed.

use super::*;

impl Agent {
    /// The conversation slice this extraction should examine.
    ///
    /// Everything since the last extraction, so a correction the keyword pass
    /// declined cannot roll out of view before the semantic pass sees it. The
    /// previous fixed "last 10 messages" only covered the 5-turn interval when
    /// every turn was a bare exchange; one tool-using turn produces more than
    /// ten messages on its own, and anything older was silently dropped. A
    /// correction lost that way is lost for good -- the keyword pass already
    /// declined it and nothing looks again.
    ///
    /// Bounded three ways, because "everything since last time" is unbounded and
    /// extraction must not cost more than the work it observes:
    /// message count, per-message length, and a total character budget. When the
    /// window overflows, the OLDEST messages are dropped: the newest are the
    /// ones the next turn will act on.
    fn extraction_window(&self) -> Vec<String> {
        extraction_window_from(&self.state.messages, self.messages_at_last_extraction)
    }

    /// GAP 5: Trigger memory extraction in the background.
    pub(in crate::agent) fn trigger_memory_extraction(&mut self) {
        let graph = match self.memory {
            Some(ref g) => Arc::clone(g),
            None => return,
        };

        let messages = self.extraction_window();
        if messages.is_empty() {
            return;
        }

        let session_id = self.config.session_id.clone();
        let turn = self.turn_number as usize;
        let attribution = self.config.runtime_attribution_extra(
            "memory_extraction",
            "memory_extraction",
            Some(self.turn_number),
            None,
            None,
        );
        let client = Arc::clone(&self.client);
        let model = self.config.model.clone();
        // Reference: auto_trainer_runtime.rs -- closure pointing at AutoTrainer.record_memories.
        let mem_cb = self.record_memory_callback.as_ref().map(Arc::clone);

        // Record extraction so we don't fire again immediately
        self.extraction_state.record_extraction(turn);
        // Advance the window. Everything up to here has now been examined, so
        // the next extraction starts where this one stopped and nothing between
        // the two is skipped.
        self.messages_at_last_extraction = self.state.messages.len();
        // Hand the window's already-captured corrections to the semantic pass and
        // reset: the next window covers different turns, so holding these longer
        // would suppress genuinely new corrections that merely resemble them.
        let already_recorded = std::mem::take(&mut self.corrections_since_extraction);

        // Run extraction in background via a real LLM call
        tokio::spawn(async move {
            let prompt = build_extraction_prompt(&messages, &already_recorded);

            let request = LlmRequest {
                model,
                max_tokens: 1024,
                system: vec![serde_json::json!({
                    "type": "text",
                    "text": "You extract structured memories from conversations. Return ONLY a JSON array."
                })],
                messages: vec![serde_json::json!({
                    "role": "user",
                    "content": prompt,
                })],
                tools: Vec::new(),
                thinking: None,
                speed: None,
                effort: None,
                extra: attribution,
                request_origin: Some("memory_extraction".into()),
                reasoning_encrypted: None,
            };

            match client.stream(request).await {
                Ok(mut rx) => {
                    let mut response_text = String::new();
                    while let Some(event) = rx.recv().await {
                        if let StreamEvent::TextDelta { text, .. } = event {
                            response_text.push_str(&text);
                        }
                    }

                    let extracted = parse_extraction_response(&response_text).unwrap_or_default();

                    // Corrections go to the CorrectionTracker, not to
                    // `store_extracted`. One writer owns correction content, so
                    // these inherit the same bounding and scoring as the fast
                    // path instead of becoming a parallel record of it.
                    let (corrections, other): (Vec<_>, Vec<_>) =
                        extracted.into_iter().partition(|m| {
                            m.memory_type == archon_memory::types::MemoryType::Correction
                        });
                    if !corrections.is_empty() {
                        let recorded =
                            crate::agent::correction_intake::record_extracted_corrections(
                                &graph,
                                &corrections,
                                &archon_consciousness::correction_provenance::semantic_pass_context(
                                    turn as u64,
                                ),
                            );
                        tracing::info!(
                            recorded,
                            "recorded corrections the keyword detector missed"
                        );
                    }

                    let extracted = other;
                    if !extracted.is_empty() {
                        match store_extracted(graph.as_ref(), &extracted, &session_id) {
                            Ok(count) => {
                                tracing::info!("auto-extracted {count} memories at turn {turn}");
                                // Reference: auto_trainer.rs::record_memories -- bumps the
                                // GNN auto-trainer's memory counter so triggers fire when
                                // the configured threshold is met.
                                if let Some(ref cb) = mem_cb {
                                    cb(count as u64);
                                }
                            }
                            Err(e) => tracing::warn!("memory extraction storage failed: {e}"),
                        }
                    } else {
                        tracing::debug!("no memories extracted at turn {turn}");
                    }
                }
                Err(e) => {
                    tracing::warn!("memory extraction API call failed: {e}");
                }
            }
        });
    }
}

/// Most messages one extraction will examine.
///
/// The window is "everything since the last extraction", which is unbounded --
/// a stretch of tool-heavy turns produces hundreds of messages. Forty covers a
/// normal five-turn interval several times over while keeping the call small.
const MAX_EXTRACTION_MESSAGES: usize = 40;

/// Per-message excerpt fed to extraction.
///
/// A pasted document previously entered this prompt whole, so a single message
/// could dominate the call. Extraction is looking for what was decided and
/// corrected, which survives an excerpt.
const MAX_EXTRACTION_MESSAGE_CHARS: usize = 1_000;

/// Total character budget across the window.
///
/// Roughly six thousand tokens: enough to see a real interval of conversation,
/// small enough that extraction never rivals the turn that triggered it.
const MAX_EXTRACTION_PROMPT_CHARS: usize = 24_000;

/// Build the extraction window from `messages`, starting at `start`.
///
/// Free-standing so the bounding and ordering can be tested without an `Agent`.
pub(in crate::agent) fn extraction_window_from(
    messages: &[serde_json::Value],
    start: usize,
) -> Vec<String> {
    let start = start.min(messages.len());
    let window = &messages[start..];
    let window = if window.len() > MAX_EXTRACTION_MESSAGES {
        &window[window.len() - MAX_EXTRACTION_MESSAGES..]
    } else {
        window
    };

    let mut budget = MAX_EXTRACTION_PROMPT_CHARS;
    let mut collected: Vec<String> = Vec::new();
    // Walked newest-first so the budget is spent on the most recent messages,
    // then reversed: the model must read the conversation in the order it
    // happened. The previous implementation collected reversed and never
    // restored the order, so every extraction saw the conversation backwards.
    for message in window.iter().rev() {
        let role = message["role"].as_str().unwrap_or("unknown");
        let content = message["content"].as_str().unwrap_or("");
        if content.is_empty() {
            continue;
        }
        // Excerpt per message: extraction wants the shape of the conversation,
        // not its attachments, and one pasted document used to enter the prompt
        // whole.
        let excerpt: String = content.chars().take(MAX_EXTRACTION_MESSAGE_CHARS).collect();
        let line = format!("{role}: {excerpt}");
        let cost = line.chars().count();
        if cost > budget {
            break;
        }
        budget -= cost;
        collected.push(line);
    }
    collected.reverse();
    collected
}

#[cfg(test)]
#[path = "../extraction_window_tests.rs"]
mod extraction_window_tests;
