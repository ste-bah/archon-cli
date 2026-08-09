//! Correction detection, behavioural-rule matching, and memory extraction.
//!
//! Split from `memory_integration.rs` to keep both halves under the 500-line
//! file-size gate. That file now covers what a turn INJECTS; this one covers
//! what a turn RECORDS afterwards.

use archon_consciousness::correction_classifier::CorrectionClassification;
use archon_consciousness::corrections::CorrectionType;

use super::correction_intake::{ShadowCorrectionLabel, record_shadow_correction_label};
use super::*;

impl Agent {
    /// Detect correction patterns in user input and record via CorrectionTracker.
    pub(super) async fn detect_and_record_correction(
        &mut self,
        user_input: &str,
        graph: &Arc<dyn MemoryTrait>,
    ) {
        // R3 shadow pass. Runs BEFORE the early return and on every user turn,
        // not only on the turns the heuristic accepts: the promotion gate needs
        // >=100 adjudicated non-corrections as well as >=100 corrections, and a
        // sample drawn only from what the heuristic already fires on can supply
        // neither an unbiased negative population nor a single false positive.
        //
        // What it decides is recorded and nothing else. The heuristic below
        // still owns every rule mutation, which is what the roadmap requires
        // until the R3 gate passes.
        let classification = super::correction_intake::shadow_classify(user_input);
        let heuristic = super::correction_intake::classify_correction(user_input);

        let Some(correction_type) = heuristic else {
            self.record_correction_shadow_label(user_input, &classification, None, None);
            // No keyword match. NOT the end of it: the periodic extractor runs a
            // semantic pass over the same turns and routes anything this missed
            // back through `record_extracted_corrections`, so a correction
            // phrased outside these patterns is caught late rather than lost.
            return;
        };

        let tracker = CorrectionTracker::new(graph.as_ref());
        let context = format!("turn:{}", self.turn_number);
        let engine = RulesEngine::new(graph.as_ref());
        let rules = match engine.get_rules_sorted() {
            Ok(rules) => rules,
            Err(error) => {
                tracing::warn!("rule lookup failed during correction handling: {error}");
                // The heuristic still decided "correction" on this turn, and
                // that decision is the measurement. Dropping the label here
                // would quietly bias the corpus towards turns where storage
                // happened to be healthy.
                self.record_correction_shadow_label(user_input, &classification, heuristic, None);
                return;
            }
        };
        let linked_rule_id = select_relevant_rule(user_input, &rules).map(|rule| rule.id.clone());
        // Bounded before storage: the detector matches a phrase anywhere in the
        // turn, so a pasted document that happens to contain one is recorded in
        // full otherwise. See `stored_correction_content`.
        let stored_content = stored_correction_content(user_input);
        // A correction that fitted is already in the user's own words, which
        // beats any paraphrase and costs nothing. Only a truncated one is worth
        // spending a call to restate.
        let was_truncated = stored_content != user_input;
        let correction = match tracker.record_correction(
            correction_type,
            &stored_content,
            &context,
            linked_rule_id.as_deref(),
        ) {
            Ok(correction) => {
                // Reference: archon-pipeline/src/learning/gnn/auto_trainer.rs::record_correction.
                // Closure-injection avoids cycle (archon-core cannot import archon-pipeline).
                if let Some(ref cb) = self.record_correction_callback {
                    cb();
                }
                Some(correction)
            }
            Err(e) => {
                tracing::warn!("failed to record correction: {e}");
                None
            }
        };

        // Remember what the fast path caught, so the extractor's semantic pass
        // reports only what it missed instead of a rival copy of this.
        if correction.is_some() {
            self.corrections_since_extraction
                .push(stored_content.clone());
        }

        // Emitted here rather than at the top of the function so the label can
        // carry the id of the correction that was actually written, which is
        // what a later adjudication pass joins on.
        self.record_correction_shadow_label(
            user_input,
            &classification,
            heuristic,
            correction.as_ref().map(|record| record.id.clone()),
        );

        // Improve the record after the fact, never before it. The correction is
        // already stored above, so a failed or empty summary leaves the bounded
        // raw text in place rather than losing what the user said.
        if was_truncated && let Some(ref correction) = correction {
            self.spawn_correction_summary(
                correction.id.clone(),
                user_input.to_string(),
                Arc::clone(graph),
            );
        }

        // CRIT-15 (ITEM 5): Notify inner voice of user correction.
        if let Some(ref iv) = self.inner_voice
            && let Ok(mut iv) = iv.try_lock()
        {
            iv.on_user_correction();
            // TASK #245: keep panic-mirror in lock-step (inside the same
            // try_lock guard, so mirror cannot drift relative to actual).
            if let Some(ref cb) = self.inner_voice_change_callback {
                cb(&iv);
            }
        }

        if let Some(ref cb) = self.record_user_correction_event_callback {
            let payload = UserCorrectionEventPayload {
                correction_type: format!("{correction_type:?}"),
                top_rule_id: correction.and_then(|record| record.rule_id),
                user_input_excerpt: user_correction_excerpt(user_input),
                session_context: context,
            };
            self.fire_before_learning_event_hook("UserCorrected", &payload)
                .await;
            cb(payload.clone());
            self.fire_after_learning_event_hook("UserCorrected", &payload)
                .await;
        }
    }

    /// Persist one R3 shadow label: what the classifier decided, next to what
    /// the heuristic decided and whether the heuristic mutated anything.
    ///
    /// Fails open. Telemetry and shadow analysis may degrade without taking the
    /// foreground turn with them (roadmap global constraint), so every failure
    /// path here warns and returns; none of them can stop a correction being
    /// recorded, because the correction was already recorded before this ran.
    ///
    /// The write goes to the blocking pool: it is a SQLite insert plus a JSONL
    /// append, and a measurement that is not allowed to change behaviour is
    /// certainly not allowed to add latency to the turn that produced it.
    fn record_correction_shadow_label(
        &self,
        user_input: &str,
        classification: &CorrectionClassification,
        heuristic: Option<CorrectionType>,
        correction_id: Option<String>,
    ) {
        let Some(store) = self.cognitive_store.as_ref().map(Arc::clone) else {
            // No cognitive store means no metric substrate to write into. Worth
            // saying out loud: it is the difference between "the corpus is
            // empty because nothing happened" and "the corpus is empty because
            // nothing was listening".
            tracing::debug!("no cognitive store; R3 correction shadow label not recorded");
            return;
        };

        let label = ShadowCorrectionLabel {
            session_id: self.config.session_id.clone(),
            turn_number: self.turn_number,
            task_class: self
                .current_situation
                .as_ref()
                .map_or("unclassified", |situation| situation.kind.as_str())
                .to_string(),
            model_id: self.config.model.clone(),
            classification: classification.clone(),
            heuristic,
            correction_id,
            user_input_hash: super::correction_intake::user_input_hash(user_input),
            observed_at: chrono::Utc::now(),
        };

        archon_observability::spawn_blocking_named("record-correction-shadow-label", move || {
            let store = store
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            match record_shadow_correction_label(&store, &label) {
                Ok(outcome) => tracing::debug!(?outcome, "recorded R3 correction shadow label"),
                Err(error) => tracing::warn!(%error, "R3 correction shadow label write failed"),
            }
        });
    }

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
    pub(super) fn trigger_memory_extraction(&mut self) {
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
        // Reference: auto_trainer_runtime.rs — closure pointing at AutoTrainer.record_memories.
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
                                &format!("turn:{turn} (semantic pass)"),
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
                                // Reference: auto_trainer.rs::record_memories — bumps the
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
pub(super) fn extraction_window_from(messages: &[serde_json::Value], start: usize) -> Vec<String> {
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

const MATCH_THRESHOLD: f64 = 0.25;
const MIN_MEANINGFUL_OVERLAP: usize = 2;
const COMMON_CORRECTION_TOKENS: &[&str] = &[
    "a", "an", "and", "before", "do", "for", "i", "instead", "is", "it", "no", "not", "of", "or",
    "should", "that", "the", "this", "to", "use", "was", "with", "you",
];

fn select_relevant_rule<'a>(
    correction: &str,
    rules: &'a [archon_consciousness::rules::BehavioralRule],
) -> Option<&'a archon_consciousness::rules::BehavioralRule> {
    let correction_token_set = correction_tokens(correction);
    rules
        .iter()
        .filter_map(|rule| {
            let rule_tokens = correction_tokens(&rule.text);
            let overlap = correction_token_set.intersection(&rule_tokens).count();
            let union = correction_token_set.union(&rule_tokens).count();
            let jaccard = overlap as f64 / union.max(1) as f64;
            let dice = (2 * overlap) as f64
                / (correction_token_set.len() + rule_tokens.len()).max(1) as f64;
            let relevance = (jaccard + dice) / 2.0;
            (overlap >= MIN_MEANINGFUL_OVERLAP
                && jaccard >= MATCH_THRESHOLD
                && dice >= MATCH_THRESHOLD)
                .then_some((rule, relevance))
        })
        .max_by(
            |(left_rule, left_similarity), (right_rule, right_similarity)| {
                left_similarity
                    .total_cmp(right_similarity)
                    .then_with(|| left_rule.score.total_cmp(&right_rule.score))
                    .then_with(|| right_rule.id.cmp(&left_rule.id))
            },
        )
        .map(|(rule, _)| rule)
}

fn correction_tokens(text: &str) -> std::collections::BTreeSet<String> {
    text.split(|character: char| !character.is_alphanumeric())
        .map(str::to_lowercase)
        .filter(|token| token.len() > 1 && !COMMON_CORRECTION_TOKENS.contains(&token.as_str()))
        .collect()
}

#[cfg(test)]
#[path = "extraction_window_tests.rs"]
mod extraction_window_tests;
